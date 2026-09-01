use super::*;

pub(super) fn fixed_shortcut(stroke: &Keystroke) -> Option<Box<dyn Action>> {
    let modifiers = stroke.modifiers;
    if modifiers.platform || modifiers.function {
        return None;
    }
    if !modifiers.control && modifiers.alt {
        // Windows GPUI turns Shift+= / Shift+- into + / _ and clears Shift.
        let key_char = stroke.key_char.as_deref();
        if (modifiers.shift && stroke.key == "=") || stroke.key == "+" || key_char == Some("+") {
            return Some(Box::new(SplitRight));
        }
        if (modifiers.shift && stroke.key == "-") || stroke.key == "_" || key_char == Some("_") {
            return Some(Box::new(SplitDown));
        }
    }
    match (
        modifiers.control,
        modifiers.alt,
        modifiers.shift,
        stroke.key.as_str(),
    ) {
        (true, false, false, ",") => Some(Box::new(OpenSettings)),
        (true, false, true, "t") => Some(Box::new(NewTab)),
        (true, false, true, "w") => Some(Box::new(ClosePane)),
        (true, false, false, "tab") => Some(Box::new(NextTab)),
        (true, false, true, "tab") => Some(Box::new(PreviousTab)),
        (false, true, false, "left") => Some(Box::new(FocusLeft)),
        (false, true, false, "right") => Some(Box::new(FocusRight)),
        (false, true, false, "up") => Some(Box::new(FocusUp)),
        (false, true, false, "down") => Some(Box::new(FocusDown)),
        (false, true, true, "left") => Some(Box::new(ResizeLeft)),
        (false, true, true, "right") => Some(Box::new(ResizeRight)),
        (false, true, true, "up") => Some(Box::new(ResizeUp)),
        (false, true, true, "down") => Some(Box::new(ResizeDown)),
        _ => None,
    }
}

pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", TerminalTab, Some("CondrTerminal")),
        KeyBinding::new("shift-tab", TerminalBackTab, Some("CondrTerminal")),
        KeyBinding::new("ctrl-shift-t", NewTab, Some("Condr")),
        KeyBinding::new("ctrl-,", OpenSettings, Some("Condr")),
        KeyBinding::new("ctrl-shift-w", ClosePane, Some("Condr")),
        KeyBinding::new("ctrl-tab", NextTab, Some("Condr")),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some("Condr")),
        KeyBinding::new("alt-shift-=", SplitRight, Some("Condr")),
        KeyBinding::new("alt-shift--", SplitDown, Some("Condr")),
        KeyBinding::new("alt-left", FocusLeft, Some("Condr")),
        KeyBinding::new("alt-right", FocusRight, Some("Condr")),
        KeyBinding::new("alt-up", FocusUp, Some("Condr")),
        KeyBinding::new("alt-down", FocusDown, Some("Condr")),
        KeyBinding::new("alt-shift-left", ResizeLeft, Some("Condr")),
        KeyBinding::new("alt-shift-right", ResizeRight, Some("Condr")),
        KeyBinding::new("alt-shift-up", ResizeUp, Some("Condr")),
        KeyBinding::new("alt-shift-down", ResizeDown, Some("Condr")),
    ]);
}

pub(super) fn connect_to_server_with<T>(
    endpoint: Endpoint,
    ensure_local_server: impl FnOnce() -> std::io::Result<Endpoint>,
    connect: impl FnOnce(&Endpoint) -> Result<T, String>,
) -> (Endpoint, Result<T, String>) {
    if endpoint != ServerConfig::default().endpoint {
        let result = connect(&endpoint);
        return (endpoint, result);
    }

    match ensure_local_server() {
        Ok(connected_endpoint) => {
            let result = connect(&connected_endpoint);
            (connected_endpoint, result)
        }
        Err(error) => {
            let discovery_error =
                format!("Failed to start or discover local condr-server: {error}");
            let result = connect(&endpoint).map_err(|_| discovery_error);
            (endpoint, result)
        }
    }
}

pub(super) fn connect_to_server(
    endpoint: Endpoint,
    client_name: &'static str,
) -> (Endpoint, Result<ClientConnection, String>) {
    connect_to_server_with(endpoint, condr_server::ensure_local_server, |endpoint| {
        ClientConnection::connect(endpoint, client_name).map_err(|error| error.to_string())
    })
}

pub(crate) fn run() {
    let (endpoint, initial) = connect_to_server(ServerConfig::default().endpoint, "condr-gui");
    let config_path = config::default_path();
    let app = gpui_platform::application().with_assets(CondrAssets::new());

    app.run(move |cx| {
        gpui_component::init(cx);
        bind_keys(cx);
        let window_options = default_window_options(cx);
        cx.spawn(async move |cx| {
            cx.open_window(window_options, |window, cx| {
                let view = cx.new(|cx| Condr::new(endpoint, initial, config_path, window, cx));
                let root = cx.new(|cx| Root::new(view, window, cx));
                window.resize(DEFAULT_WINDOW_SIZE);
                root
            })
            .expect("failed to open Condr window");
        })
        .detach();
    });
}
