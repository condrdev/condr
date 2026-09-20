use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;

use super::*;

thread_local! {
    static FIXED_BINDINGS: Vec<KeyBinding> = fixed_bindings();
}

pub(super) fn fixed_shortcut(stroke: &Keystroke) -> Option<Box<dyn Action>> {
    if stroke.modifiers.function {
        return None;
    }
    let mut normalized = stroke.clone();
    let modifiers = stroke.modifiers;
    if !cfg!(target_os = "macos") && !modifiers.control && !modifiers.platform && modifiers.alt {
        // Windows GPUI turns Shift+= / Shift+- into + / _ and clears Shift.
        let key_char = stroke.key_char.as_deref();
        if stroke.key == "+" || key_char == Some("+") {
            normalized.key = "=".into();
            normalized.modifiers.shift = true;
        } else if stroke.key == "_" || key_char == Some("_") {
            normalized.key = "-".into();
            normalized.modifiers.shift = true;
        }
    }
    if cfg!(target_os = "macos") && modifiers.platform && !modifiers.control && !modifiers.alt {
        // GPUI may fold Cmd+Shift+] / [ into Cmd+} / {.
        match stroke.key.as_str() {
            "]" if modifiers.shift => normalized.key = "}".into(),
            "[" if modifiers.shift => normalized.key = "{".into(),
            _ => {}
        }
        if matches!(normalized.key.as_str(), "{" | "}") {
            normalized.modifiers.shift = false;
        }
    }
    // IME text is handled before this fallback; only the normalized command chord
    // participates here, so produced text cannot strip Ctrl/Alt into another shortcut.
    normalized.key_char = None;
    FIXED_BINDINGS.with(|bindings| {
        bindings
            .iter()
            .find(|binding| {
                binding.match_keystrokes(std::slice::from_ref(&normalized)) == Some(false)
            })
            .map(|binding| binding.action().boxed_clone())
    })
}

fn fixed_bindings() -> Vec<KeyBinding> {
    let mut bindings = vec![
        // `secondary` is Cmd on macOS and Ctrl elsewhere.
        KeyBinding::new("secondary-,", OpenSettings, Some("Condr")),
        KeyBinding::new("ctrl-tab", NextTab, Some("Condr")),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some("Condr")),
        // Workspaces stack vertically in the sidebar, so Up/Down walk them.
        KeyBinding::new("secondary-shift-up", PreviousWorkspace, Some("Condr")),
        KeyBinding::new("secondary-shift-down", NextWorkspace, Some("Condr")),
    ];
    let tab_modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "alt"
    };
    bindings.extend((0..9).map(|index| {
        KeyBinding::new(
            &format!("{tab_modifier}-{}", index + 1),
            ActivateTab { index },
            Some("Condr"),
        )
    }));
    if cfg!(target_os = "macos") {
        bindings.extend([
            KeyBinding::new("cmd-t", NewTab, Some("Condr")),
            KeyBinding::new("cmd-w", ClosePane, Some("Condr")),
            KeyBinding::new("cmd-}", NextTab, Some("Condr")),
            KeyBinding::new("cmd-{", PreviousTab, Some("Condr")),
            KeyBinding::new("cmd-d", SplitRight, Some("Condr")),
            KeyBinding::new("cmd-shift-d", SplitDown, Some("Condr")),
            KeyBinding::new("cmd-alt-left", FocusLeft, Some("Condr")),
            KeyBinding::new("cmd-alt-right", FocusRight, Some("Condr")),
            KeyBinding::new("cmd-alt-up", FocusUp, Some("Condr")),
            KeyBinding::new("cmd-alt-down", FocusDown, Some("Condr")),
            KeyBinding::new("cmd-ctrl-left", ResizeLeft, Some("Condr")),
            KeyBinding::new("cmd-ctrl-right", ResizeRight, Some("Condr")),
            KeyBinding::new("cmd-ctrl-up", ResizeUp, Some("Condr")),
            KeyBinding::new("cmd-ctrl-down", ResizeDown, Some("Condr")),
            KeyBinding::new("cmd-shift-enter", ToggleZoom, Some("Condr")),
            // VS Code's primary / secondary sidebar chords.
            KeyBinding::new("cmd-b", ToggleSidebar, Some("Condr")),
            KeyBinding::new("cmd-alt-b", ToggleChanges, Some("Condr")),
            // The system chords every macOS app answers; the app menu shows them too.
            KeyBinding::new("cmd-q", QuitApp, Some("Condr")),
            KeyBinding::new("cmd-h", HideApp, Some("Condr")),
            KeyBinding::new("cmd-alt-h", HideOtherApps, Some("Condr")),
            KeyBinding::new("cmd-m", MinimizeWindow, Some("Condr")),
        ]);
    } else {
        bindings.extend([
            KeyBinding::new("ctrl-shift-t", NewTab, Some("Condr")),
            KeyBinding::new("ctrl-shift-w", ClosePane, Some("Condr")),
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
            KeyBinding::new("alt-shift-enter", ToggleZoom, Some("Condr")),
            // Ctrl+B is the tmux prefix and readline's backward-char, so Shift joins it.
            KeyBinding::new("ctrl-shift-b", ToggleSidebar, Some("Condr")),
            KeyBinding::new("ctrl-alt-b", ToggleChanges, Some("Condr")),
        ]);
    }
    bindings
}

pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", TerminalTab, Some("CondrTerminal")),
        KeyBinding::new("shift-tab", TerminalBackTab, Some("CondrTerminal")),
    ]);
    FIXED_BINDINGS.with(|bindings| cx.bind_keys(bindings.clone()));
}

/// The macOS menu bar. GPUI only answers Cmd+Q, Cmd+H and Cmd+M through menu items,
/// so without these the system shortcuts do nothing; the key equivalents come from the
/// bindings above. Quit runs the `on_app_quit` hooks, so pending config saves finish.
#[cfg(target_os = "macos")]
pub(super) fn install_app_menus(cx: &mut App) {
    cx.on_action(|_: &QuitApp, cx| cx.quit());
    cx.on_action(|_: &HideApp, cx| cx.hide());
    cx.on_action(|_: &HideOtherApps, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAllApps, cx| cx.unhide_other_apps());
    cx.set_menus([
        Menu::new(condr_core::APP_NAME).items([
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action(format!("Hide {}", condr_core::APP_NAME), HideApp),
            MenuItem::action("Hide Others", HideOtherApps),
            MenuItem::action("Show All", ShowAllApps),
            MenuItem::separator(),
            MenuItem::action(format!("Quit {}", condr_core::APP_NAME), QuitApp),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", MinimizeWindow),
            MenuItem::action("Zoom", ZoomWindow),
        ]),
    ]);
}

#[cfg(not(target_os = "macos"))]
pub(super) fn install_app_menus(_cx: &mut App) {}

pub(super) fn connect_to_server_with<T>(
    endpoint: Endpoint,
    ensure_local_server: impl FnOnce() -> std::io::Result<Endpoint>,
    connect: impl FnOnce(&Endpoint) -> Result<T, String>,
) -> (Endpoint, Result<T, String>) {
    if endpoint != ServerConfig::default().local_endpoint() {
        let result = connect(&endpoint);
        return (endpoint, result);
    }

    match ensure_local_server() {
        Ok(connected_endpoint) => {
            let result = connect(&connected_endpoint);
            (connected_endpoint, result)
        }
        Err(error) => {
            let discovery_error = format!("Failed to start Condr on this device: {error}");
            let result = connect(&endpoint).map_err(|_| discovery_error);
            (endpoint, result)
        }
    }
}

/// The GUI introduces itself by host name, so a paired device is recognisable in
/// `condr server clients`.
pub(super) fn connect_to_server(
    endpoint: Endpoint,
    cancellation: ConnectionCancellation,
) -> (Endpoint, Result<ClientConnection, String>) {
    connect_to_server_with(endpoint, condr_server::ensure_local_server, |endpoint| {
        ClientConnection::connect_cancellable(
            endpoint,
            condr_server::noise::device_name(),
            cancellation,
        )
        .map_err(|error| endpoint.describe_connect_error(&error))
    })
}

pub(super) fn single_instance_lock_path() -> Option<PathBuf> {
    condr_core::runtime_directory().map(|root| root.join("condr.lock"))
}

/// Takes the GUI's single-instance lock, which the caller must hold for the whole run.
///
/// Keeps one GUI per user. Config transactions have their own cross-process lock;
/// this application lock is released when the process exits, including on a crash.
pub(super) fn acquire_single_instance_lock() -> io::Result<File> {
    let path = single_instance_lock_path().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no platform runtime directory for the single-instance lock",
        )
    })?;
    lock_exclusively(&path)
}

// `Path` alone would resolve to `gpui_kit::Path`.
pub(super) fn lock_exclusively(path: &std::path::Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    file.try_lock().map_err(|error| match error {
        TryLockError::WouldBlock => io::Error::new(
            io::ErrorKind::WouldBlock,
            "another Condr GUI is already running",
        ),
        TryLockError::Error(error) => error,
    })?;
    Ok(file)
}

pub(crate) fn run() {
    // Both held until `run` returns, which is when the GUI exits.
    let _log_guard = condr_server::logging::init("condr-gui");
    let _instance_lock = match acquire_single_instance_lock() {
        Ok(lock) => lock,
        Err(error) => {
            tracing::error!("Condr is already running, or its lock is unavailable: {error}");
            return;
        }
    };
    let endpoint = ServerConfig::default().local_endpoint();
    let config = config::LoadedConfig::read(condr_core::config_path());
    let app = gpui_kit::application().with_assets(CondrAssets::new());

    app.run(move |cx| {
        gpui_kit::init(cx);
        cx.set_app_identity(APP_IDENTITY, "Condr");
        register_notification_icon();
        bind_keys(cx);
        install_app_menus(cx);
        let window_options = default_window_options(cx);
        cx.spawn(async move |cx| {
            cx.open_window(window_options, |window, cx| {
                // The drawn title bar carries no OS title; the taskbar still needs one.
                window.set_window_title("Condr");
                let view = cx.new(|cx| Condr::new(endpoint, None, config, window, cx));
                let handle = window.window_handle();
                let target = view.downgrade();
                cx.on_system_notification_response(move |response, cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        target
                            .update(cx, |this, cx| {
                                this.handle_system_notification_response(response, window, cx)
                            })
                            .ok();
                    });
                });
                let root = cx.new(|cx| Root::new(view, window, cx));
                window.resize(DEFAULT_WINDOW_SIZE);
                root
            })
            .expect("failed to open Condr window");
        })
        .detach();
    });
}

const APP_IDENTITY: &str = "dev.condr.gui";

/// An unpackaged Windows app gets its toast icon from `IconUri` on its AUMID key. GPUI
/// registers only the display name, so point it at a copy of the packaged icon.
#[cfg(windows)]
fn register_notification_icon() {
    let Some(directory) = condr_core::data_directory() else {
        return;
    };
    let icon = directory.join("condr.png");
    if fs::create_dir_all(&directory)
        .and_then(|()| fs::write(&icon, include_bytes!("../../../../assets/brand/condr.png")))
        .is_err()
    {
        return;
    }
    let _ = windows_registry::CURRENT_USER
        .create(format!(r"Software\Classes\AppUserModelId\{APP_IDENTITY}"))
        .and_then(|key| key.set_string("IconUri", icon.display().to_string()));
}

#[cfg(not(windows))]
fn register_notification_icon() {}
