use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;

use super::*;
#[cfg(target_os = "macos")]
use gpui_kit::component::input;
#[cfg(target_os = "macos")]
use rust_i18n::t;

/// Cmd on macOS and Ctrl elsewhere, as `secondary-` was before `KeyBinding::new` stopped
/// resolving it without an `App`.
macro_rules! secondary {
    ($keys:literal) => {
        if cfg!(target_os = "macos") {
            concat!("cmd-", $keys)
        } else {
            concat!("ctrl-", $keys)
        }
    };
}

pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", TerminalTab, Some("CondrTerminal")),
        KeyBinding::new("shift-tab", TerminalBackTab, Some("CondrTerminal")),
        KeyBinding::new(secondary!("s"), SaveFile, Some("CondrPreview")),
    ]);
    cx.bind_keys(super::directory_browser::key_bindings());
    cx.bind_keys(super::new_workspace::key_bindings());
    cx.bind_keys(super::command_palette::key_bindings());
    cx.bind_keys(super::git_graph::key_bindings());
    super::keymap::install(&super::keymap::Keymap::default(), cx);
}

/// The macOS menu bar. GPUI only answers Cmd+Q, Cmd+H and Cmd+M through menu items,
/// so without these the system shortcuts do nothing. Quit runs the `on_app_quit` hooks,
/// so pending config saves finish.
#[cfg(target_os = "macos")]
pub(super) fn install_app_menus(cx: &mut App) {
    cx.on_action(|_: &QuitApp, cx| cx.quit());
    cx.on_action(|_: &HideApp, cx| cx.hide());
    cx.on_action(|_: &HideOtherApps, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAllApps, cx| cx.unhide_other_apps());
    set_app_menus(cx);
}

/// The menus themselves, made again whenever the keymap changes: an item's key
/// equivalent comes from the bindings when it is made, and its text from the locale.
#[cfg(target_os = "macos")]
pub(super) fn set_app_menus(cx: &mut App) {
    let app = condr_core::APP_NAME;
    cx.set_menus([
        Menu::new(app).items([
            MenuItem::action(t!("menu.app.settings"), OpenSettings),
            MenuItem::separator(),
            MenuItem::os_submenu(t!("menu.app.services"), SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action(t!("menu.app.hide", app = app), HideApp),
            MenuItem::action(t!("menu.app.hide_others"), HideOtherApps),
            MenuItem::action(t!("menu.app.show_all"), ShowAllApps),
            MenuItem::separator(),
            MenuItem::action(t!("menu.app.quit", app = app), QuitApp),
        ]),
        // A text field answers every item and a terminal Copy and Paste. The OS actions
        // let macOS recognise the menu and append Dictation and Emoji & Symbols.
        Menu::new(t!("menu.edit.title")).items([
            MenuItem::os_action(t!("menu.edit.undo"), input::Undo, OsAction::Undo),
            MenuItem::os_action(t!("menu.edit.redo"), input::Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action(t!("menu.edit.cut"), input::Cut, OsAction::Cut),
            MenuItem::os_action(t!("menu.edit.copy"), input::Copy, OsAction::Copy),
            MenuItem::os_action(t!("menu.edit.paste"), input::Paste, OsAction::Paste),
            MenuItem::os_action(
                t!("menu.edit.select_all"),
                input::SelectAll,
                OsAction::SelectAll,
            ),
        ]),
        // GPUI makes macOS's window list of the menu named exactly "Window", so that one
        // name stays English.
        Menu::new("Window").items([
            MenuItem::action(t!("menu.window.minimize"), MinimizeWindow),
            MenuItem::action(t!("menu.window.zoom"), ZoomWindow),
            MenuItem::separator(),
            MenuItem::action(t!("menu.window.full_screen"), ToggleFullScreen),
        ]),
    ]);
}

#[cfg(not(target_os = "macos"))]
pub(super) fn install_app_menus(_cx: &mut App) {}

#[cfg(not(target_os = "macos"))]
pub(super) fn set_app_menus(_cx: &mut App) {}

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

/// The GUI's `Hello`: its name, and the hint that it takes the Server's Tasks (ADR 0068).
pub(super) fn gui_hello(client_name: impl Into<String>) -> condr_core::protocol::Hello {
    condr_core::protocol::Hello {
        tasks: true,
        ..condr_core::protocol::Hello::new(client_name)
    }
}

/// The GUI introduces itself by host name, so a paired device is recognisable in
/// `condr server clients`.
pub(super) fn connect_to_server(
    endpoint: Endpoint,
    cancellation: ConnectionCancellation,
    refusal: &std::cell::Cell<Option<condr_core::protocol::Refusal>>,
) -> (Endpoint, Result<ClientConnection, String>) {
    connect_to_server_with(endpoint, condr_server::ensure_local_server, |endpoint| {
        ClientConnection::connect_cancellable(
            endpoint,
            gui_hello(condr_server::noise::device_name()),
            cancellation,
        )
        .map_err(|error| {
            refusal.set(
                condr_server::Refused::from_error(&error).map(|refused| refused.refusal.clone()),
            );
            endpoint.describe_connect_error(&error)
        })
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

/// A drag-and-drop DMG has no installer step, so the GUI inside `Condr.app` registers
/// the `condr` beside it on every start (`condr server install` is idempotent; ADR 0016)
/// and starts the Server from that installed copy, as the AppImage's `AppRun` does, so
/// the Server outlives a replaced bundle. The GUI is the bundle's main executable
/// rather than a script launcher because the notarization ticket lists only Mach-O
/// code, so Gatekeeper would block a script. Outside a bundle nothing happens.
fn install_bundled_cli() -> io::Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let executable = std::env::current_exe()?;
    let Some(directory) = executable
        .parent()
        .filter(|directory| directory.ends_with("Contents/MacOS"))
    else {
        return Ok(());
    };
    let output = std::process::Command::new(directory.join("condr"))
        .args(["server", "install", "--json"])
        .stdin(std::process::Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let path = report["install_path"]
        .as_str()
        .ok_or_else(|| io::Error::other("the install report names no install_path"))?;
    condr_server::set_server_executable(path.into());
    Ok(())
}

pub(crate) fn run() {
    // First, while this is the only thread: the Server inherits what this adopts.
    #[cfg(unix)]
    let login_environment = super::login_environment::adopt();
    // Both held until `run` returns, which is when the GUI exits.
    let _log_guard = condr_server::logging::init("condr-gui");
    #[cfg(unix)]
    if let Err(error) = login_environment {
        tracing::warn!("kept the launch environment, not the login shell's: {error}");
    }
    if let Err(error) = install_bundled_cli() {
        tracing::warn!(
            "the bundled condr was not installed, so the Server starts from the bundle: {error}"
        );
    }
    let _instance_lock = match acquire_single_instance_lock() {
        Ok(lock) => lock,
        Err(error) => {
            tracing::error!("Condr is already running, or its lock is unavailable: {error}");
            return;
        }
    };
    let endpoint = ServerConfig::default().local_endpoint();
    let config = config::LoadedConfig::read(condr_core::config_path());
    let state = gui_state::LoadedState::read(gui_state::GuiState::default_path());
    // macOS would otherwise keep a windowless process that a Dock click cannot reopen.
    let app = gpui_kit::application()
        .with_assets(CondrAssets::new())
        .with_quit_mode(gpui_kit::QuitMode::LastWindowClosed);

    app.run(move |cx| {
        gpui_kit::init(cx);
        // Before anything is worded, the menus first (ADR 0066).
        gpui_kit::component::set_locale(crate::i18n::resolve(
            config.language.as_deref(),
            sys_locale::get_locales(),
        ));
        cx.set_app_identity(APP_IDENTITY, "Condr");
        register_notification_icon();
        bind_keys(cx);
        install_app_menus(cx);
        super::updates::install_http_client(None, cx);
        let window_options = default_window_options(state.state.window, cx);
        let restored_window = state.state.window.is_some();
        cx.spawn(async move |cx| {
            cx.open_window(window_options, |window, cx| {
                // The drawn title bar carries no OS title; the taskbar still needs one.
                window.set_window_title("Condr");
                let view = cx.new(|cx| Condr::new(endpoint, None, config, state, window, cx));
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
                if !restored_window {
                    window.resize(DEFAULT_WINDOW_SIZE);
                }
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
