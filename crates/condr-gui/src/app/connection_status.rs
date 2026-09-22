use super::*;
use gpui_kit::component::button::ButtonRounded;
use gpui_kit::component::clipboard::Clipboard;
use gpui_kit::component::popover::Popover;

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
) -> &'static str {
    if let Some(condr_core::protocol::Refusal::IncompatibleProtocol) = refusal {
        return "Condr there and here speak different protocol versions, so trying again \
                will not help. Update both to the same version.";
    }
    if reason.contains("Failed to start Condr on this device") {
        return "Condr's own server could not start on this device. Connect tries again.";
    }
    if reason.contains("start the remote Server with") {
        return "SSH reached the device, but Condr is not running there. Start it with \
                `condr server start` on the device.";
    }
    if reason.contains("nothing is listening") {
        return "Nothing answers at this address. Check that Condr is running there and \
                that the address is right.";
    }
    if reason.contains("did not answer") || reason.contains("stopped answering") {
        return "The device did not answer. Check that it is awake and on the network.";
    }
    if reason.contains("is unreachable") {
        return "This device cannot reach that address. Check the network and any VPN.";
    }
    if reason.contains("refused this connection") {
        return "The device refused this connection. Check that this device is still paired \
                there.";
    }
    if reason.contains("closed the connection") {
        return "The device closed the connection. Condr there may have stopped or restarted.";
    }
    if matches!(endpoint, Endpoint::Ssh(_)) {
        return "SSH could not reach the device. Try the same `ssh` destination from a \
                terminal on this device.";
    }
    "Connect tries again. The reason below says what went wrong."
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
        let label = &connection.label;
        let reconnecting = connection.reconnect_deadline.is_some();
        let busy = connection.status == ConnectionStatus::Connecting || reconnecting;
        let title = if reconnecting {
            format!("Reconnecting to {label}…")
        } else if busy {
            format!("Connecting to {label}…")
        } else {
            format!("Can't reach {label}")
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
                                        .tooltip("Copy error"),
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
                                        .label("Connect")
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
                                            .label("Edit device")
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

    /// Where a device's connection stands, as a pill over a Workspace whose device went
    /// away: persistent and non-modal, the reason only on request. A device with nothing
    /// open gets `render_disconnected` instead. Nothing while connected, and nothing for
    /// the first `RECONNECT_GRACE` of a drop that may heal on its own.
    pub(super) fn render_connection_status(
        &self,
        connection: &ServerConnection,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let key = connection.key;
        let label = &connection.label;
        let reconnecting = connection.reconnect_deadline.is_some();
        let (text, busy) = match connection.status {
            ConnectionStatus::Connected => return None,
            ConnectionStatus::Connecting | ConnectionStatus::Disconnected if reconnecting => {
                (format!("Reconnecting to {label}…"), true)
            }
            ConnectionStatus::Connecting => (format!("Connecting to {label}…"), true),
            ConnectionStatus::Disconnected => (format!("Disconnected from {label}"), false),
        };
        if reconnecting
            && connection
                .disconnected_at
                .is_some_and(|at| at.elapsed() < RECONNECT_GRACE)
        {
            return None;
        }
        let owner = cx.weak_entity();
        let reason = (!busy).then(|| connection.error.clone()).flatten();
        let endpoint = connection.endpoint.clone();
        let refusal = connection.refusal.clone();
        let muted = cx.theme().muted_foreground;
        Some(
            h_flex()
                .debug_selector(move || format!("connection-status-{key}"))
                .gap_2()
                .items_center()
                .pl_3()
                .pr_1()
                .py_1()
                .rounded_full()
                .bg(cx.theme().popover)
                .border_1()
                .border_color(cx.theme().border)
                .shadow_sm()
                .text_sm()
                .child(if busy {
                    Spinner::new().xsmall().into_any_element()
                } else {
                    div()
                        .size_2()
                        .rounded_full()
                        .bg(cx.theme().danger)
                        .into_any_element()
                })
                .child(div().text_color(muted).child(text))
                .when_some(reason, |pill, reason| {
                    let advice = connection_advice(&reason, &endpoint, refusal.as_ref());
                    pill.child(
                        Popover::new(("connection-details", key))
                            .trigger(
                                Button::new(("connection-details-trigger", key))
                                    .small()
                                    .rounded(ButtonRounded::Size(px(999.)))
                                    .ghost()
                                    .label("Details"),
                            )
                            .content(move |_, _, cx| {
                                v_flex()
                                    .gap_2()
                                    .max_w(DISCONNECTED_COLUMN_WIDTH)
                                    .text_sm()
                                    .child(advice)
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .items_start()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(reason.clone()),
                                            )
                                            .child(
                                                Clipboard::new(("connection-error-copy", key))
                                                    .value(reason.clone())
                                                    .tooltip("Copy error"),
                                            ),
                                    )
                            }),
                    )
                })
                .when(!busy, |pill| {
                    pill.child(
                        Button::new(("connect-server", key))
                            .debug_selector(move || format!("connect-server-{key}"))
                            .small()
                            .rounded(ButtonRounded::Size(px(999.)))
                            .outline()
                            .label("Connect")
                            .on_click(move |_, window, cx| {
                                let _ = owner
                                    .update(cx, |this, cx| this.connect_server(key, window, cx));
                            }),
                    )
                })
                .into_any_element(),
        )
    }
}
