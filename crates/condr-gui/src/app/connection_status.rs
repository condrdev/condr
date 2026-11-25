use super::*;
use gpui_kit::component::clipboard::Clipboard;
use rust_i18n::t;
use std::borrow::Cow;

/// Wide enough for a connect error to wrap into two or three readable lines.
const DISCONNECTED_COLUMN_WIDTH: Rems = rems(30.);

/// One sentence on what to do about a connect failure: by the Server's typed refusal
/// when it sent one, otherwise keyed on the wording the connect path produces
/// (`Endpoint::describe_connect_error`, `disconnect_reason`, the SSH bridge, local
/// startup). The raw reason is shown under it either way.
fn connection_advice(
    reason: &str,
    endpoint: &Endpoint,
    refusal: Option<&condr_core::protocol::Refusal>,
) -> Cow<'static, str> {
    if let Some(condr_core::protocol::Refusal::IncompatibleProtocol) = refusal {
        return t!("connection.advice.incompatible_protocol");
    }
    // The reasons stay English (ADR 0066), so the matching works under every locale.
    if reason.contains("Failed to start Condr on this device") {
        return t!("connection.advice.local_start_failed");
    }
    if reason.contains("start the remote Server with") {
        return t!("connection.advice.ssh_not_running");
    }
    if reason.contains("nothing is listening") {
        return t!("connection.advice.nothing_listening");
    }
    if reason.contains("did not answer") || reason.contains("stopped answering") {
        return t!("connection.advice.no_answer");
    }
    if reason.contains("is unreachable") {
        return t!("connection.advice.unreachable");
    }
    if reason.contains("refused this connection") {
        return t!("connection.advice.refused");
    }
    if reason.contains("closed the connection") {
        return t!("connection.advice.closed");
    }
    if matches!(endpoint, Endpoint::Ssh(_)) {
        return t!("connection.advice.ssh_failed");
    }
    t!("connection.advice.retry")
}

impl Condr {
    /// The body when the active device has nothing to show because it is not connected:
    /// what is happening, what to do about it, and the raw reason for an issue report.
    /// The Welcome page is for a connected device without Workspaces; a failure is not
    /// a way to get started.
    pub(super) fn render_disconnected(
        &self,
        connection: &ServerConnection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = connection.key;
        let device = &connection.label;
        let reconnecting = connection.reconnect_deadline.is_some();
        let busy = connection.status == ConnectionStatus::Connecting || reconnecting;
        // No reason means nothing failed: the user disconnected, or never connected.
        let deliberate = connection.error.is_none();
        let title = if reconnecting {
            t!("connection.title.reconnecting", device = device)
        } else if busy {
            t!("connection.title.connecting", device = device)
        } else if deliberate {
            t!("connection.title.disconnected", device = device)
        } else {
            t!("connection.title.unreachable", device = device)
        };
        // A stale reason under a spinner reads as a fresh failure; show it when settled.
        let reason = (!busy).then(|| connection.error.clone()).flatten();
        let advice = reason.as_deref().map(|reason| {
            connection_advice(reason, &connection.endpoint, connection.refusal.as_ref())
        });
        let is_local = connection.endpoint.as_local_path().is_some();
        let connect_owner = cx.weak_entity();
        let edit_owner = cx.weak_entity();
        let muted = cx.theme().muted_foreground;
        v_flex()
            .debug_selector(move || format!("disconnected-page-{key}"))
            .size_full()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .w(DISCONNECTED_COLUMN_WIDTH)
                    .gap_3()
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(if busy {
                                Spinner::new().small().into_any_element()
                            } else if deliberate {
                                Icon::new(super::icons::CondrIconName::Circle)
                                    .size_5()
                                    .text_color(muted)
                                    .into_any_element()
                            } else {
                                Icon::new(IconName::TriangleAlert)
                                    .size_5()
                                    .text_color(cx.theme().warning)
                                    .into_any_element()
                            })
                            .child(div().text_lg().child(title)),
                    )
                    .when_some(advice, |column, advice| {
                        column.child(div().text_sm().child(advice))
                    })
                    .when_some(reason, |column, reason| {
                        column.child(
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .gap_2()
                                .items_start()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_xs()
                                        .text_color(muted)
                                        .child(reason.clone()),
                                )
                                .child(
                                    Clipboard::new(("connection-error-copy", key))
                                        .value(reason)
                                        .tooltip(t!("connection.copy_error")),
                                ),
                        )
                    })
                    .when(!busy, |column| {
                        column.child(
                            h_flex()
                                .gap_2()
                                .pt_1()
                                .child(
                                    Button::new(("connect-server", key))
                                        .debug_selector(move || format!("connect-server-{key}"))
                                        .small()
                                        .primary()
                                        .label(t!("connection.connect"))
                                        .on_click(move |_, window, cx| {
                                            let _ = connect_owner.update(cx, |this, cx| {
                                                this.connect_server(key, window, cx)
                                            });
                                        }),
                                )
                                .when(!is_local, |row| {
                                    row.child(
                                        Button::new(("edit-server", key))
                                            .small()
                                            .outline()
                                            .label(t!("connection.edit_device"))
                                            .on_click(move |_, window, cx| {
                                                let _ = edit_owner.update(cx, |this, cx| {
                                                    this.prompt_edit_server_on(key, window, cx)
                                                });
                                            }),
                                    )
                                }),
                        )
                    }),
            )
            .into_any_element()
    }

    /// The veil over a Workspace whose device is reconnecting on its own: the frozen
    /// Panes stay underneath, faded so their text is not read as live output, and the
    /// disconnected page's column says what is happening, in the place the failure takes
    /// if the retries give up. GPUI cannot blur what lies under an element, so the veil
    /// is the background colour, nearly opaque. Nothing for the first `RECONNECT_GRACE`,
    /// so a blip that heals on its own is never seen.
    pub(super) fn render_reconnect_veil(
        &self,
        connection: &ServerConnection,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let reconnecting = connection.status != ConnectionStatus::Connected
            && connection.reconnect_deadline.is_some();
        if !reconnecting
            || connection
                .disconnected_at
                .is_some_and(|at| at.elapsed() < RECONNECT_GRACE)
        {
            return None;
        }
        let key = connection.key;
        Some(
            div()
                .debug_selector(move || format!("reconnect-veil-{key}"))
                .absolute()
                .inset_0()
                .occlude()
                .bg(cx.theme().background.opacity(0.85))
                .child(self.render_disconnected(connection, cx))
                .into_any_element(),
        )
    }
}
