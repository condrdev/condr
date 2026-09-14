use super::*;

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
        Some(
            h_flex()
                .debug_selector(move || format!("connection-status-{key}"))
                .w_full()
                .min_w_0()
                .gap_2()
                .items_center()
                .text_sm()
                .when(busy, |row| row.child(Spinner::new().xsmall()))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(cx.theme().muted_foreground)
                        .child(text),
                )
                .when_some(connection.error.clone(), |row, error| {
                    row.child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(cx.theme().danger)
                            .child(error),
                    )
                })
                .when(!busy, |row| {
                    row.child(
                        Button::new(("connect-server", key))
                            .debug_selector(move || format!("connect-server-{key}"))
                            .small()
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
