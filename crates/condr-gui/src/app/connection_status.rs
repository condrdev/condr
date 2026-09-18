use super::*;
use gpui_kit::component::clipboard::Clipboard;

impl Condr {
    /// Where a device's connection stands, for the welcome page and the banner over a
    /// Workspace whose device went away. Nothing while connected: the Tab strip carries
    /// ordinary errors.
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
            ConnectionStatus::Disconnected => (format!("Not connected to {label}"), false),
        };
        let owner = cx.weak_entity();
        let row = h_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .items_center()
            .when(busy, |row| row.child(Spinner::new().xsmall()))
            .child(
                div()
                    .flex_1()
                    .text_color(cx.theme().muted_foreground)
                    .child(text),
            )
            .when(!busy, |row| {
                row.child(
                    Button::new(("connect-server", key))
                        .debug_selector(move || format!("connect-server-{key}"))
                        .small()
                        .outline()
                        .label("Connect")
                        .on_click(move |_, window, cx| {
                            let _ =
                                owner.update(cx, |this, cx| this.connect_server(key, window, cx));
                        }),
                )
            });
        // The reason on its own line, in full, with a copy button: a truncated
        // "could not connect to ssh…" is no help in an issue report.
        Some(
            v_flex()
                .debug_selector(move || format!("connection-status-{key}"))
                .w_full()
                .min_w_0()
                .gap_1()
                .text_sm()
                .child(row)
                .when_some(connection.error.clone(), |column, error| {
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
                                    .text_color(cx.theme().danger)
                                    .child(error.clone()),
                            )
                            .child(
                                Clipboard::new(("connection-error-copy", key))
                                    .value(error)
                                    .tooltip("Copy error"),
                            ),
                    )
                })
                .into_any_element(),
        )
    }
}
