//! The update check (ADR 0029): whether the channel the person follows has published a
//! build newer than this one. It only tells; installing is the person's.

use super::*;
use condr_core::BuildComparison;
use futures_lite::AsyncReadExt as _;
use gpui_kit::http_client::{AsyncBody, HttpClient};

/// The first check waits this long after start, so it never competes with startup.
pub(super) const FIRST_CHECK_DELAY: Duration = Duration::from_secs(5);
pub(super) const CHECK_INTERVAL: Duration = Duration::from_secs(5 * 60 * 60);

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

    /// Where this channel's newest build is named, and the JSON field that holds it: the
    /// stable version condr.dev serves as plain text to the install scripts, or the
    /// commit the `nightly` tag points at, from the GitHub API.
    fn latest_source(self) -> (&'static str, Option<&'static str>) {
        match self {
            Self::Stable => ("https://condr.dev/version.txt", None),
            Self::Nightly => (
                "https://api.github.com/repos/condrdev/condr/git/ref/tags/nightly",
                Some("/object/sha"),
            ),
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
    /// Where the person picks a package: condr.dev's download page for stable, which
    /// always offers the latest release, or the nightly release page.
    pub(in crate::app) url: SharedString,
}

/// Whether `latest`, the channel's newest build (a version on stable, a commit on
/// nightly), is newer than this build: a higher version on stable, any other commit on
/// nightly. A build without a commit (a local `cargo build`) therefore always sees the
/// nightly as newer.
fn available_update(
    channel: UpdateChannel,
    latest: &str,
    this_build: &str,
    this_commit: Option<&str>,
) -> Option<AvailableUpdate> {
    match channel {
        UpdateChannel::Stable => (condr_core::compare_builds(this_build, latest)
            == BuildComparison::OtherNewer)
            .then(|| AvailableUpdate {
                name: format!("Condr {latest}").into(),
                url: "https://condr.dev/download/".into(),
            }),
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

/// The version in a plain-text answer. Only `1.2.3` counts: a captive portal or a proxy's
/// error page answers 200 too.
fn plain_version(body: &[u8]) -> Option<String> {
    let version = std::str::from_utf8(body).ok()?.trim();
    (version.split('.').count() == 3 && version.split('.').all(|part| part.parse::<u64>().is_ok()))
        .then(|| version.to_owned())
}

/// Reads `field` from the JSON at `url`, or without a field the version `url` answers
/// as plain text.
async fn fetch_latest(
    client: Arc<dyn HttpClient>,
    url: &str,
    field: Option<&str>,
) -> Result<String, String> {
    let mut response = client
        .get(url, AsyncBody::empty(), true)
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("{url} answered {}", response.status()));
    }
    let mut body = Vec::new();
    response
        .body_mut()
        .read_to_end(&mut body)
        .await
        .map_err(|error| error.to_string())?;
    let Some(field) = field else {
        return plain_version(&body).ok_or_else(|| format!("no version in {url}'s answer"));
    };
    let json: serde_json::Value =
        serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    json.pointer(field)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("no {field} in {url}'s answer"))
}

/// Owns the update preferences, result, acknowledgement and repeating check task.
/// The root keeps this value; Settings only reads it or invokes its commands.
pub(super) struct UpdateCheck {
    automatic: bool,
    channel: UpdateChannel,
    state: UpdateState,
    /// Opening Settings clears the dot until a check finds a newer build again.
    seen: bool,
    /// Only a manual check makes the Check button spin.
    checking: bool,
    _automatic_task: Task<()>,
}

impl UpdateCheck {
    pub(super) fn new(automatic: bool, channel: UpdateChannel) -> Self {
        Self {
            automatic,
            channel,
            state: UpdateState::Unknown,
            seen: false,
            checking: false,
            _automatic_task: Task::ready(()),
        }
    }

    pub(super) fn automatic(&self) -> bool {
        self.automatic
    }

    pub(super) fn channel(&self) -> UpdateChannel {
        self.channel
    }

    pub(super) fn state(&self) -> &UpdateState {
        &self.state
    }

    pub(super) fn checking(&self) -> bool {
        self.checking
    }

    /// A newer build is known and Settings has not been opened since the check found
    /// it: the sidebar's Settings button carries a dot and Settings opens on the update.
    pub(super) fn pending(&self) -> bool {
        matches!(self.state, UpdateState::Available(_)) && !self.seen
    }

    pub(super) fn mark_seen(&mut self) {
        self.seen = true;
    }

    fn set_automatic(&mut self, enabled: bool, cx: &mut Context<Condr>) -> bool {
        if self.automatic == enabled {
            return false;
        }
        self.automatic = enabled;
        if enabled {
            self.start_automatic(Duration::ZERO, cx);
        } else {
            self._automatic_task = Task::ready(());
            self.state = UpdateState::Unknown;
        }
        true
    }

    /// A channel change forgets the old result and checks immediately, outside the
    /// five-hour schedule, when automatic checks are enabled.
    fn set_channel(&mut self, channel: UpdateChannel, cx: &mut Context<Condr>) -> bool {
        if self.channel == channel {
            return false;
        }
        self.channel = channel;
        self.state = UpdateState::Unknown;
        if self.automatic {
            self.check(false, cx).detach();
        }
        true
    }

    /// The first automatic check after `delay`, then one every five hours; replacing the
    /// task stops them. Each checks the channel current when it runs.
    pub(super) fn start_automatic(&mut self, delay: Duration, cx: &mut Context<Condr>) {
        if !self.automatic {
            return;
        }
        self._automatic_task = cx.spawn(async move |this, cx| {
            let mut delay = delay;
            loop {
                cx.background_executor().timer(delay).await;
                delay = CHECK_INTERVAL;
                let Ok(check) = this.update(cx, |this, cx| this.updates.check(false, cx)) else {
                    break;
                };
                check.await;
            }
        });
    }

    /// One check of the current channel. `manual` is the Check button: its spinner turns
    /// while the request runs and a failure is reported. An automatic check only logs
    /// its failure, and the next one retries.
    fn check(&mut self, manual: bool, cx: &mut Context<Condr>) -> Task<()> {
        let channel = self.channel;
        let (url, field) = channel.latest_source();
        if manual {
            self.checking = true;
        }
        cx.spawn(async move |this, cx| {
            let client = cx.update(|cx| cx.http_client());
            let latest = cx
                .background_executor()
                .spawn(fetch_latest(client, url, field))
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = this.updates.finish(channel, latest, manual) {
                    this.report_error(error, cx);
                }
                cx.notify();
            });
        })
    }

    fn finish(
        &mut self,
        channel: UpdateChannel,
        latest: Result<String, String>,
        manual: bool,
    ) -> Result<(), String> {
        if manual {
            self.checking = false;
        }
        // The channel changed while the request ran: its answer is about the old one.
        if channel != self.channel {
            return Ok(());
        }
        match latest {
            Ok(latest) => {
                self.seen = false;
                self.state = available_update(
                    channel,
                    &latest,
                    condr_core::build_identity(),
                    option_env!("CONDR_BUILD_COMMIT").filter(|commit| !commit.is_empty()),
                )
                .map_or(UpdateState::UpToDate, UpdateState::Available);
            }
            Err(error) if manual => {
                return Err(format!("Failed to check for updates: {error}"));
            }
            Err(error) => tracing::warn!(
                "Update check on the {} channel failed: {error}",
                channel.as_str()
            ),
        }
        Ok(())
    }
}

impl Condr {
    pub(in crate::app) fn set_auto_check_updates(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.updates.set_automatic(enabled, cx) {
            self.save_auto_check_updates(cx);
            cx.notify();
        }
    }

    pub(in crate::app) fn set_update_channel(
        &mut self,
        channel: UpdateChannel,
        cx: &mut Context<Self>,
    ) {
        if self.updates.set_channel(channel, cx) {
            self.save_update_channel(cx);
            cx.notify();
        }
    }

    /// The Check button works with automatic checks off, and leaves their schedule alone.
    pub(in crate::app) fn check_for_updates_now(&mut self, cx: &mut Context<Self>) {
        self.updates.check(true, cx).detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{AvailableUpdate, UpdateChannel, available_update, plain_version};

    #[test]
    fn stable_reads_only_a_bare_version() {
        assert_eq!(plain_version(b"0.2.0\n").as_deref(), Some("0.2.0"));
        assert_eq!(plain_version(b"<html>Access denied</html>"), None);
        assert_eq!(plain_version(b"0.2"), None);
        assert_eq!(plain_version(b"v0.2.0"), None);
    }

    #[test]
    fn stable_offers_only_a_higher_version() {
        let offer = |latest, this| available_update(UpdateChannel::Stable, latest, this, None);
        assert_eq!(
            offer("0.2.0", "0.1.0+1a2b3c4d5e6f"),
            Some(AvailableUpdate {
                name: "Condr 0.2.0".into(),
                url: "https://condr.dev/download/".into(),
            })
        );
        assert_eq!(offer("0.1.0", "0.1.0"), None);
        // A nightly of the released version is not older than the release.
        assert_eq!(offer("0.1.0", "0.1.0+1a2b3c4d5e6f"), None);
        assert_eq!(offer("0.1.0", "0.2.0"), None);
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
