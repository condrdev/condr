//! Pushes to the Companion through Condr Cloud (ADR 0047). The terminal monitor finds an
//! Agent's change on its own thread, which holds the `RuntimeState` lock and has no tokio
//! context, so it hands an owned [`Push`], recipients and text resolved, to the one sender
//! task on the Server's runtime.

use super::*;
use crate::noise::{Authority, LazyIdentity, PublicKey};
use condr_core::{AgentAlert, ProxyMode, ProxySetting};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// How long one post to Condr Cloud may take.
pub(super) const PUSH_TIMEOUT: Duration = Duration::from_secs(5);
/// The longest body: what is left of APNs' and FCM's 4 KB is no limit on a Workspace name.
const MAX_BODY_BYTES: usize = 1024;
/// The most recipients Condr Cloud takes in one push.
const MAX_RECIPIENTS: usize = 16;

/// The body of Condr Cloud's `POST /api/push/send`.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Push {
    pub(super) to: Vec<String>,
    pub(super) title_loc_key: &'static str,
    pub(super) title_loc_args: [&'static str; 1],
    pub(super) body: String,
    pub(super) pane: String,
}

/// The monitor's end: whom to push to, and the sender task's queue.
pub(super) struct Pushes {
    identity: Arc<LazyIdentity>,
    queue: UnboundedSender<Push>,
}

impl Pushes {
    /// Starts the sender task on `runtime`, posting to Condr Cloud at `base_url` as this
    /// machine's Device.
    pub(super) fn start(
        runtime: &tokio::runtime::Handle,
        base_url: String,
        identity: Arc<LazyIdentity>,
        proxy: &ProxySetting,
    ) -> io::Result<Self> {
        let queue = start_sender(
            runtime,
            base_url,
            Arc::clone(&identity) as _,
            proxy,
            PUSH_TIMEOUT,
        )?;
        Ok(Self { identity, queue })
    }
}

impl RuntimeState {
    /// Queues a push for `agent`'s change in `pane_id` to each Device that takes pushes,
    /// skipping one watching that Pane now (ADR 0041). Nobody left means no push.
    pub(super) fn push_agent_alert(
        &self,
        pane_id: PaneId,
        alert: AgentAlert,
        agent: &AgentSnapshot,
    ) {
        let Some(pushes) = &self.pushes else {
            return;
        };
        let recipients = match pushes.identity.get().and_then(|id| id.push_recipients()) {
            Ok(recipients) => recipients,
            Err(error) => {
                tracing::warn!("cannot read the Devices that take pushes: {error}");
                return;
            }
        };
        let watching: Vec<PublicKey> = self
            .watched_terminals
            .iter()
            .filter(|(_, panes)| panes.contains(&pane_id))
            .filter_map(|(client_id, _)| self.tcp_peers.get(client_id).map(|(key, _)| *key))
            .collect();
        // ponytail: past 16 Devices the rest get nothing; send several pushes if anyone
        // pairs that many phones.
        let to: Vec<String> = recipients
            .into_iter()
            .filter(|key| !watching.contains(key))
            .take(MAX_RECIPIENTS)
            .map(|key| key.to_string())
            .collect();
        if to.is_empty() {
            return;
        }
        let workspace = self
            .session
            .workspace_for_pane(pane_id)
            .map(|workspace| workspace.name());
        let device_name = crate::noise::device_name();
        let _ = pushes.queue.send(Push {
            to,
            title_loc_key: match alert {
                AgentAlert::Finished => "push_finished",
                AgentAlert::Failed => "push_failed",
                AgentAlert::NeedsInput { .. } => "push_blocked",
            },
            title_loc_args: [agent.kind.label()],
            body: push_body([
                agent.detail.as_deref(),
                workspace,
                self.terminal_titles.get(&pane_id).map(String::as_str),
                Some(device_name.as_str()),
            ]),
            pane: pane_id.as_u64().to_string(),
        });
    }
}

/// `detail · Workspace · terminal title · Server`, empty parts left out, cut to 1 KB of
/// UTF-8 at a character boundary.
pub(super) fn push_body<'a>(parts: impl IntoIterator<Item = Option<&'a str>>) -> String {
    let mut body = parts
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    body.truncate(body.floor_char_boundary(MAX_BODY_BYTES));
    body
}

/// Spawns the sender task and returns its queue.
pub(super) fn start_sender(
    runtime: &tokio::runtime::Handle,
    base_url: String,
    signer: Arc<dyn Authority>,
    proxy: &ProxySetting,
    timeout: Duration,
) -> io::Result<UnboundedSender<Push>> {
    // reqwest is built without a crypto provider, as iroh builds it; ring is the one there.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let builder = reqwest::Client::builder().timeout(timeout);
    let builder = match (proxy.mode, proxy.manual_url()) {
        (ProxyMode::System, _) => builder,
        (ProxyMode::Manual, Some(url)) => {
            builder.proxy(reqwest::Proxy::all(url.as_str()).map_err(io::Error::other)?)
        }
        (ProxyMode::None | ProxyMode::Manual, _) => builder.no_proxy(),
    };
    let client = builder.build().map_err(io::Error::other)?;
    let (queue, pushes) = unbounded_channel();
    runtime.spawn(send_pushes(pushes, client, base_url, signer));
    Ok(queue)
}

/// Posts one push at a time and keeps only the latest waiting push per Pane. A timeout
/// or a 5xx is retried once, unless a newer push for that Pane is waiting, so a late
/// Blocked never replaces the Done that followed it. Anything else is lost.
async fn send_pushes(
    mut queue: UnboundedReceiver<Push>,
    client: reqwest::Client,
    base_url: String,
    signer: Arc<dyn Authority>,
) {
    // At most one per Pane, in the order the Panes first waited.
    let mut waiting: Vec<Push> = Vec::new();
    loop {
        if waiting.is_empty() {
            let Some(push) = queue.recv().await else {
                return;
            };
            waiting.push(push);
        }
        take_queued(&mut queue, &mut waiting);
        let push = waiting.remove(0);
        let mut retried = false;
        let failure = loop {
            match post(&client, &base_url, &*signer, &push).await {
                Ok(()) => break None,
                Err((true, reason)) => {
                    take_queued(&mut queue, &mut waiting);
                    if retried || waiting.iter().any(|newer| newer.pane == push.pane) {
                        break Some(reason);
                    }
                    retried = true;
                }
                Err((false, reason)) => break Some(reason),
            }
        };
        if let Some(reason) = failure {
            tracing::warn!(pane = %push.pane, "push to Condr Cloud lost: {reason}");
        }
    }
}

fn take_queued(queue: &mut UnboundedReceiver<Push>, waiting: &mut Vec<Push>) {
    while let Ok(push) = queue.try_recv() {
        match waiting.iter_mut().find(|older| older.pane == push.pane) {
            Some(older) => *older = push,
            None => waiting.push(push),
        }
    }
}

/// Condr Cloud answers HTTP 200 with the outcome in `code`, like every API of its template.
#[derive(serde::Deserialize)]
struct Reply {
    code: String,
}

/// One `POST /api/push/send`; the error says whether it may be retried, and why it failed.
async fn post(
    client: &reqwest::Client,
    base_url: &str,
    signer: &dyn Authority,
    push: &Push,
) -> Result<(), (bool, String)> {
    let key = signer
        .device_key()
        .map_err(|error| (false, error.to_string()))?;
    let issued_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let body = serde_json::to_vec(push).map_err(|error| (false, error.to_string()))?;
    let response = client
        .post(format!("{base_url}/api/push/send"))
        .header("API-Version", "1")
        .header(
            reqwest::header::AUTHORIZATION,
            format!(
                "Condr {}",
                condr_client::cloud::token(&key, base_url, issued_at)
            ),
        )
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await;
    let response = response.map_err(|error| (error.is_timeout(), error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err((status.is_server_error(), status.to_string()));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| (error.is_timeout(), error.to_string()))?;
    let code = serde_json::from_slice::<Reply>(&bytes)
        .map_err(|error| (false, error.to_string()))?
        .code;
    match code.as_str() {
        "200" => Ok(()),
        _ => Err((code.starts_with('5'), format!("code {code}"))),
    }
}
