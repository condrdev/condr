//! The update check (ADR 0029): whether the channel the person follows has published a
//! build newer than this one. It only tells; installing is the person's.

use super::*;
use condr_core::BuildComparison;
use futures_lite::AsyncReadExt as _;
use gpui_kit::http_client::{AsyncBody, HttpClient};

/// The first check waits this long after start, so it never competes with startup.
pub(super) const FIRST_CHECK_DELAY: Duration = Duration::from_secs(5);
pub(super) const CHECK_INTERVAL: Duration = Duration::from_secs(5 * 60 * 60);
const REPOSITORY_API: &str = "https://api.github.com/repos/condrdev/condr";

/// Which published builds the update check looks for: `[client.updates] channel`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) enum UpdateChannel {
    Stable,
    Nightly,
}

impl UpdateChannel {
    pub(in crate::app) const ALL: [Self; 2] = [Self::Stable, Self::Nightly];

    pub(in crate::app) fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Nightly => "nightly",
        }
    }

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Stable => "Stable",
            Self::Nightly => "Nightly",
        }
    }

    /// Unset or unrecognized values follow this build.
    pub(in crate::app) fn from_str(value: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|channel| channel.as_str() == value)
            .unwrap_or_else(Self::of_this_build)
    }

    /// The channel this build was published on ([`condr_core::is_nightly`]).
    pub(in crate::app) fn of_this_build() -> Self {
        if condr_core::is_nightly() {
            Self::Nightly
        } else {
            Self::Stable
        }
    }

    /// Where the GitHub API names this channel's newest build, and the JSON field that
    /// holds it: the stable tag, or the commit the `nightly` tag points at.
    fn latest_source(self) -> (&'static str, &'static str) {
        match self {
            Self::Stable => ("releases/latest", "/tag_name"),
            Self::Nightly => ("git/ref/tags/nightly", "/object/sha"),
        }
    }
}

/// What the last check on the current channel found. Back to `Unknown` when the channel
/// changes or automatic checks are turned off.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::app) enum UpdateState {
    #[default]
    Unknown,
    UpToDate,
    Available(AvailableUpdate),
}

/// A build on the followed channel that is newer than this one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::app) struct AvailableUpdate {
    /// `Condr 0.2.0`, or `Nightly 1a2b3c4d5e6f`.
    pub(in crate::app) name: SharedString,
    /// Its release page, where the person picks a package.
    pub(in crate::app) url: SharedString,
}

/// Whether `latest`, the channel's newest build as GitHub names it, is newer than this
/// build: a higher version on stable, any other commit on nightly. A build without a
/// commit (a local `cargo build`) therefore always sees the nightly as newer.
fn available_update(
    channel: UpdateChannel,
    latest: &str,
    this_build: &str,
    this_commit: Option<&str>,
) -> Option<AvailableUpdate> {
    match channel {
        UpdateChannel::Stable => {
            let version = latest.strip_prefix('v').unwrap_or(latest);
            (condr_core::compare_builds(this_build, version) == BuildComparison::OtherNewer).then(
                || AvailableUpdate {
                    name: format!("Condr {version}").into(),
                    url: format!("{REPOSITORY_URL}/releases/tag/{latest}").into(),
                },
            )
        }
        UpdateChannel::Nightly => (this_commit != Some(latest)).then(|| AvailableUpdate {
            name: format!("Nightly {}", latest.get(..12).unwrap_or(latest)).into(),
            url: format!("{REPOSITORY_URL}/releases/tag/nightly").into(),
        }),
    }
}

/// Gives GPUI its HTTP client, built off the main thread: loading the platform's CA
/// certificates is not free, and nothing needs it before the first check. Only
/// `startup::run` calls this, so tests keep GPUI's test client and never reach GitHub.
pub(super) fn install_http_client(cx: &mut App) {
    let client = cx.background_executor().spawn(async {
        // Not `ReqwestClient::user_agent`: it leaves rustls to pick a crypto provider,
        // and with both ring (iroh) and aws-lc-rs in the build it cannot.
        reqwest_client::ReqwestClient::proxy_and_user_agent(
            None,
            &format!("Condr/{}", condr_core::build_identity()),
        )
    });
    cx.spawn(async move |cx| match client.await {
        Ok(client) => cx.update(|cx| cx.set_http_client(Arc::new(client))),
        Err(error) => tracing::warn!("No HTTP client, so no update check: {error}"),
    })
    .detach();
}

async fn fetch_latest(
    client: Arc<dyn HttpClient>,
    path: &str,
    field: &str,
) -> Result<String, String> {
    let mut response = client
        .get(
            &format!("{REPOSITORY_API}/{path}"),
            AsyncBody::empty(),
            true,
        )
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("GitHub answered {}", response.status()));
    }
    let mut body = Vec::new();
    response
        .body_mut()
        .read_to_end(&mut body)
        .await
        .map_err(|error| error.to_string())?;
    let json: serde_json::Value =
        serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    json.pointer(field)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("no {field} in GitHub's answer"))
}

impl Condr {
    pub(in crate::app) fn set_auto_check_updates(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.auto_check_updates == enabled {
            return;
        }
        self.auto_check_updates = enabled;
        self.save_auto_check_updates(cx);
        if enabled {
            self.start_automatic_update_checks(Duration::ZERO, cx);
        } else {
            self._automatic_update_checks = Task::ready(());
            self.update_state = UpdateState::Unknown;
        }
        cx.notify();
    }

    /// What the last channel's check found no longer applies; with automatic checks on,
    /// the new channel is checked right away, outside the five-hour schedule.
    pub(in crate::app) fn set_update_channel(
        &mut self,
        channel: UpdateChannel,
        cx: &mut Context<Self>,
    ) {
        if self.update_channel == channel {
            return;
        }
        self.update_channel = channel;
        self.save_update_channel(cx);
        self.update_state = UpdateState::Unknown;
        if self.auto_check_updates {
            self.check_for_updates(false, cx).detach();
        }
        cx.notify();
    }

    /// The Check button: works whether or not automatic checks are on, and leaves their
    /// schedule alone.
    pub(in crate::app) fn check_for_updates_now(&mut self, cx: &mut Context<Self>) {
        self.check_for_updates(true, cx).detach();
        cx.notify();
    }

    /// The first automatic check after `delay`, then one every five hours; replacing the
    /// task stops them. Each checks the channel current when it runs.
    pub(super) fn start_automatic_update_checks(
        &mut self,
        delay: Duration,
        cx: &mut Context<Self>,
    ) {
        self._automatic_update_checks = cx.spawn(async move |this, cx| {
            let mut delay = delay;
            loop {
                cx.background_executor().timer(delay).await;
                delay = CHECK_INTERVAL;
                let Ok(check) = this.update(cx, |this, cx| this.check_for_updates(false, cx))
                else {
                    break;
                };
                check.await;
            }
        });
    }

    /// One check of the current channel. `manual` is the Check button: its spinner turns
    /// while the request runs and a failure is reported. An automatic check nobody asked
    /// for only logs its failure, and the next one retries.
    fn check_for_updates(&mut self, manual: bool, cx: &mut Context<Self>) -> Task<()> {
        let channel = self.update_channel;
        let (path, field) = channel.latest_source();
        if manual {
            self.checking_updates = true;
        }
        cx.spawn(async move |this, cx| {
            let client = cx.update(|cx| cx.http_client());
            let latest = cx
                .background_executor()
                .spawn(fetch_latest(client, path, field))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.finish_update_check(channel, latest, manual, cx);
            });
        })
    }

    fn finish_update_check(
        &mut self,
        channel: UpdateChannel,
        latest: Result<String, String>,
        manual: bool,
        cx: &mut Context<Self>,
    ) {
        if manual {
            self.checking_updates = false;
        }
        cx.notify();
        // The channel changed while the request ran: its answer is about the old one.
        if channel != self.update_channel {
            return;
        }
        match latest {
            Ok(latest) => {
                self.update_state = available_update(
                    channel,
                    &latest,
                    condr_core::build_identity(),
                    option_env!("CONDR_BUILD_COMMIT").filter(|commit| !commit.is_empty()),
                )
                .map_or(UpdateState::UpToDate, UpdateState::Available);
            }
            Err(error) if manual => {
                self.report_error(format!("Failed to check for updates: {error}"), cx);
            }
            Err(error) => tracing::warn!(
                "Update check on the {} channel failed: {error}",
                channel.as_str()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AvailableUpdate, UpdateChannel, available_update};

    #[test]
    fn stable_offers_only_a_higher_version() {
        let offer = |latest, this| available_update(UpdateChannel::Stable, latest, this, None);
        assert_eq!(
            offer("v0.2.0", "0.1.0+1a2b3c4d5e6f"),
            Some(AvailableUpdate {
                name: "Condr 0.2.0".into(),
                url: "https://github.com/condrdev/condr/releases/tag/v0.2.0".into(),
            })
        );
        assert_eq!(offer("v0.1.0", "0.1.0"), None);
        // A nightly of the released version is not older than the release.
        assert_eq!(offer("v0.1.0", "0.1.0+1a2b3c4d5e6f"), None);
        assert_eq!(offer("v0.1.0", "0.2.0"), None);
    }

    #[test]
    fn nightly_offers_any_other_commit() {
        let sha = "1a2b3c4d5e6f7a8b9c0d1a2b3c4d5e6f7a8b9c0d";
        let offer = |this_commit| {
            available_update(
                UpdateChannel::Nightly,
                sha,
                "0.1.0+1a2b3c4d5e6f",
                this_commit,
            )
        };
        assert_eq!(offer(Some(sha)), None);
        let update = offer(Some("0000000000000000000000000000000000000000")).unwrap();
        assert_eq!(update.name.as_ref(), "Nightly 1a2b3c4d5e6f");
        assert_eq!(
            update.url.as_ref(),
            "https://github.com/condrdev/condr/releases/tag/nightly"
        );
        // A local build has no commit, so every nightly is news to it.
        assert!(offer(None).is_some());
    }

    #[test]
    fn unknown_channels_follow_the_build() {
        for channel in UpdateChannel::ALL {
            assert_eq!(UpdateChannel::from_str(channel.as_str()), channel);
        }
        assert_eq!(
            UpdateChannel::from_str("beta"),
            UpdateChannel::of_this_build()
        );
    }
}
