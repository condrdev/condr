use super::*;
use gpui_kit::{SystemNotification, SystemNotificationResponse};

const AGENT_TAG_PREFIX: &str = "agent:";

/// One notification per Pane: a later change replaces the earlier toast.
pub(super) fn agent_notification_tag(key: ConnectionKey, pane_id: PaneId) -> SharedString {
    format!("{AGENT_TAG_PREFIX}{key}:{}", pane_id.as_u64()).into()
}

pub(super) fn parse_agent_notification_tag(tag: &str) -> Option<(ConnectionKey, PaneId)> {
    let (key, pane_id) = tag.strip_prefix(AGENT_TAG_PREFIX)?.split_once(':')?;
    Some((key.parse().ok()?, PaneId::from_u64(pane_id.parse().ok()?)))
}

impl Condr {
    /// An agent in a Pane the user is not looking at finished or asked a question:
    /// tell the OS notification center. Clicking the toast selects that Pane.
    pub(super) fn notify_agent_change(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
        previous: Option<AgentState>,
        agent: &AgentSnapshot,
        cx: &mut App,
    ) {
        if !self.notifications {
            return;
        }
        // Blocked to Blocked is a new question or request: the Server publishes only
        // changed snapshots. It replaces the toast without asking for attention again.
        let (title, attention) = match (previous, agent.state) {
            (Some(AgentState::Working | AgentState::Blocked), AgentState::Idle) => {
                (format!("{} finished", agent.kind.label()), true)
            }
            (Some(AgentState::Working), AgentState::Blocked) => {
                (format!("{} needs your input", agent.kind.label()), true)
            }
            (Some(AgentState::Blocked), AgentState::Blocked) if agent.blocked_on.is_some() => {
                (format!("{} needs your input", agent.kind.label()), false)
            }
            _ => return,
        };
        let Some(connection) = self.connection(key) else {
            return;
        };
        let workspace = Session::restore(connection.snapshot.clone())
            .ok()
            .and_then(|session| {
                session
                    .workspace_for_pane(pane_id)
                    .map(|workspace| workspace.name().to_owned())
            });
        // What it waits for comes first (ADR 0024): that is what decides whether to come.
        let body = [
            agent.blocked_on.clone(),
            workspace,
            connection.terminal_titles.get(&pane_id).cloned(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        cx.show_system_notification(SystemNotification {
            tag: agent_notification_tag(key, pane_id),
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
        let window = self.window_handle;
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, _| window.request_attention());
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
        let Some((key, pane_id)) = parse_agent_notification_tag(&response.tag) else {
            return;
        };
        window.activate_window();
        self.select_pane(key, pane_id, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{agent_notification_tag, parse_agent_notification_tag};
    use condr_core::PaneId;

    #[test]
    fn agent_notification_tag_round_trips_and_rejects_foreign_tags() {
        let tag = agent_notification_tag(3, PaneId::from_u64(42));
        assert_eq!(
            parse_agent_notification_tag(&tag),
            Some((3, PaneId::from_u64(42)))
        );
        assert_eq!(parse_agent_notification_tag("agent:3"), None);
        assert_eq!(parse_agent_notification_tag("update:3:42"), None);
    }
}
