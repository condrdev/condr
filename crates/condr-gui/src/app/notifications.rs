use super::*;
use gpui_kit::{SystemNotification, SystemNotificationResponse};

const PANE_TAG_PREFIX: &str = "pane:";

/// One notification per Pane, an Agent's or a program's: a later one replaces the
/// earlier toast, and clicking it opens the Pane.
pub(super) fn pane_notification_tag(key: ConnectionKey, pane_id: PaneId) -> SharedString {
    format!("{PANE_TAG_PREFIX}{key}:{}", pane_id.as_u64()).into()
}

pub(super) fn parse_pane_notification_tag(tag: &str) -> Option<(ConnectionKey, PaneId)> {
    let (key, pane_id) = tag.strip_prefix(PANE_TAG_PREFIX)?.split_once(':')?;
    Some((key.parse().ok()?, PaneId::from_u64(pane_id.parse().ok()?)))
}

impl Condr {
    /// An agent in a Pane the user is not looking at finished or asked a question:
    /// tell the OS notification center. Clicking the toast selects that Pane.
    pub(super) fn notify_agent_change(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
        previous: Option<&AgentSnapshot>,
        agent: &AgentSnapshot,
        cx: &mut App,
    ) {
        if !self.notifications {
            return;
        }
        // The rule the Server pushes to the Companion by too (ADR 0047). A Blocked that
        // waits for something new replaces the toast without asking for attention again.
        let label = agent.kind.label();
        let (title, attention) = match AgentAlert::between(previous, agent) {
            Some(AgentAlert::Finished) => (format!("{label} finished"), true),
            Some(AgentAlert::Failed) => (format!("{label} failed"), true),
            Some(AgentAlert::NeedsInput { updated }) => {
                (format!("{label} needs your input"), !updated)
            }
            None => return,
        };
        let Some(connection) = self.connection(key) else {
            return;
        };
        let workspace = connection.session().and_then(|session| {
            session
                .workspace_for_pane(pane_id)
                .map(|workspace| workspace.name().to_owned())
        });
        // What it waits for, or why it failed, comes first (ADR 0024): that is what
        // decides whether to come.
        let body = [
            agent.detail.clone(),
            workspace,
            connection.model.terminal_titles.get(&pane_id).cloned(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        cx.show_system_notification(SystemNotification {
            tag: pane_notification_tag(key, pane_id),
            title: title.into(),
            body: body.into(),
            actions: Vec::new(),
        });
        // The taskbar button or Dock icon asks for attention too, the way a chat app
        // does; the platform skips it while the window is active. Deferred because the
        // Window is out of its table for the duration of the update this runs in.
        if !attention {
            return;
        }
        let window = self.main_window();
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, _| window.request_attention());
        });
    }

    /// A program in a Pane the user is not looking at asked for a desktop notification
    /// with OSC 9 or 777 (ADR 0054). Its title, or the Pane's, heads the toast.
    pub(super) fn notify_program(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
        title: Option<String>,
        body: String,
        cx: &mut App,
    ) {
        if !self.notifications || self.terminal_input.focused == Some((key, pane_id)) {
            return;
        }
        let Some(connection) = self.connection(key) else {
            return;
        };
        let title = title
            .or_else(|| connection.pane_title(pane_id))
            .unwrap_or_else(|| "Terminal".into());
        let workspace = connection.session().and_then(|session| {
            session
                .workspace_for_pane(pane_id)
                .map(|workspace| workspace.name().to_owned())
        });
        let body = [(!body.is_empty()).then_some(body), workspace]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
        cx.show_system_notification(SystemNotification {
            tag: pane_notification_tag(key, pane_id),
            title: title.into(),
            body: body.into(),
            actions: Vec::new(),
        });
    }

    pub(in crate::app) fn set_notifications(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.notifications == enabled {
            return;
        }
        self.notifications = enabled;
        self.save_notifications(cx);
        cx.notify();
    }

    /// A toast to check that the OS shows them at all; it ignores the switch, since the
    /// user is asking for it.
    pub(in crate::app) fn send_test_notification(&self, cx: &App) {
        cx.show_system_notification(SystemNotification {
            tag: "test".into(),
            title: "Condr notifications work".into(),
            body: "Agents that finish or need you while you look elsewhere show up like this."
                .into(),
            actions: Vec::new(),
        });
    }

    pub(super) fn handle_system_notification_response(
        &mut self,
        response: SystemNotificationResponse,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((key, pane_id)) = parse_pane_notification_tag(&response.tag) else {
            return;
        };
        window.activate_window();
        self.select_pane(key, pane_id, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{pane_notification_tag, parse_pane_notification_tag};
    use condr_core::PaneId;

    #[test]
    fn pane_notification_tag_round_trips_and_rejects_foreign_tags() {
        let tag = pane_notification_tag(3, PaneId::from_u64(42));
        assert_eq!(
            parse_pane_notification_tag(&tag),
            Some((3, PaneId::from_u64(42)))
        );
        assert_eq!(parse_pane_notification_tag("pane:3"), None);
        assert_eq!(parse_pane_notification_tag("update:3:42"), None);
    }
}
