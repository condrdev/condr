#[cfg(test)]
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use gpui_kit::{AppContext as _, Context, SharedString};
use serde::Deserialize;

use condr_server::{DeviceKey, SavedServer, load_saved_servers, save_saved_servers};

use super::open_in::CustomEditor;
use super::syntax::CodeTheme;
use super::{Appearance, Condr, Endpoint, TerminalFont, UpdateChannel};
use std::collections::BTreeMap;

const APPEARANCE_KEY: &str = "appearance";
const FPS_MONITOR_KEY: &str = "fps_monitor";
const NOTIFICATIONS_KEY: &str = "notifications";
const KEEP_AWAKE_KEY: &str = "keep_awake";
/// `[client.updates]` holds the update check's preferences (ADR 0029).
const UPDATES_TABLE: [&str; 2] = ["client", "updates"];
const AUTO_CHECK_KEY: &str = "auto_check";
const CHANNEL_KEY: &str = "channel";
/// `[client] editor`: the "Open in" target used last, and so the default for a project
/// without its own choice.
const EDITOR_KEY: &str = "editor";
/// `[[client.editors]]`: the user's own "Open in" commands.
const EDITORS_KEY: &str = "editors";
/// `[[client.workspace_editors]]`: each project's "Open in" choice, keyed by its root.
const WORKSPACE_EDITORS_KEY: &str = "workspace_editors";
/// `[client.terminal]` holds every Terminal preference.
const TERMINAL_TABLE: [&str; 2] = ["client", "terminal"];
const FONT_FAMILY_KEY: &str = "font_family";
const FONT_SIZE_KEY: &str = "font_size";
const COLOR_SCHEME_KEY: &str = "color_scheme";
// The Preview and Diff Tabs' bat theme (ADR 0032).
const CODE_TABLE: [&str; 2] = ["client", "code"];
const CODE_THEME_KEY: &str = "theme";

/// One `[[client.workspace_editors]]` entry.
#[derive(Deserialize)]
struct SavedWorkspaceEditor {
    root: PathBuf,
    editor: String,
}

pub(super) struct LoadedConfig {
    pub path: Option<PathBuf>,
    pub device_key: Option<DeviceKey>,
    pub servers: Vec<(String, Endpoint)>,
    pub servers_error: Option<String>,
    pub error: Option<String>,
    pub appearance: Appearance,
    pub fps_monitor: bool,
    pub notifications: bool,
    pub keep_awake: bool,
    pub auto_check_updates: bool,
    pub update_channel: UpdateChannel,
    pub terminal_font: TerminalFont,
    pub terminal_color_scheme: SharedString,
    pub code_theme: SharedString,
    pub code_font_size: f32,
    pub default_editor: Option<String>,
    pub custom_editors: Vec<CustomEditor>,
    pub workspace_editors: BTreeMap<PathBuf, String>,
}

impl LoadedConfig {
    /// Read before starting the GUI event loop: the config and identity locks may wait.
    pub(super) fn read(path: Option<PathBuf>) -> Self {
        let device_key = condr_server::noise::identity_directory()
            .and_then(|directory| condr_server::noise::load_device_key(&directory));
        let client = path.as_deref().map_or(Ok(None), |path| {
            condr_core::read_config_value(path, &[], "client")
        });
        Self::from_snapshot(path, device_key, client)
    }

    /// Decode one coherent `[client]` snapshot. A malformed optional preference falls
    /// back independently; a broken Device list still prevents destructive overwrites.
    fn from_snapshot(
        path: Option<PathBuf>,
        device_key: io::Result<DeviceKey>,
        client: io::Result<Option<toml::Value>>,
    ) -> Self {
        let (device_key, key_error) = match device_key {
            Ok(key) => (Some(key), None),
            Err(error) => (
                None,
                Some(format!(
                    "Failed to load the device key, so TCP devices are unavailable: {error}"
                )),
            ),
        };
        let file = path.as_deref().map_or_else(
            || "config.toml".to_owned(),
            |path| path.display().to_string(),
        );
        let (client, saved_servers) = match client {
            Ok(client) => {
                let servers =
                    decode_list::<SavedServer>(client.as_ref().and_then(|c| c.get("servers")));
                (client, servers)
            }
            Err(error) => (None, Err(error)),
        };
        let value = |key: &str| client.as_ref().and_then(|client| client.get(key));
        // Invalid addresses remain on disk; only an unreadable list disables Device edits.
        let mut skipped = Vec::new();
        let (servers, servers_error) = match saved_servers {
            Ok(saved) => (
                saved
                    .into_iter()
                    .filter(|server| device_key.is_some() || !server.is_tcp())
                    .filter_map(|server| match server.endpoint(device_key.as_ref()) {
                        Ok(endpoint) => Some((server.name, endpoint)),
                        Err(error) => {
                            skipped.push(format!("{} ({}): {error}", server.name, server.address));
                            None
                        }
                    })
                    .collect(),
                None,
            ),
            Err(error) => (
                Vec::new(),
                Some(format!(
                    "Failed to load {file}: {error}. Device list changes are disabled; fix the file and restart Condr."
                )),
            ),
        };
        let skipped_error = (!skipped.is_empty()).then(|| {
            format!(
                "Skipped saved devices with an address Condr cannot read; they stay in {file} until you re-add or remove them: {}",
                skipped.join("; ")
            )
        });
        // A malformed `[[client.editors]]` is reported, not silently dropped.
        let (custom_editors, editors_error) = match decode_list(value(EDITORS_KEY)) {
            Ok(editors) => (editors, None),
            Err(error) => (
                Vec::new(),
                Some(format!("Failed to load [[client.editors]]: {error}")),
            ),
        };
        let updates = value("updates");
        let terminal = value("terminal");
        let code = value("code");
        let mut terminal_font = TerminalFont::default();
        if let Some(family) = nonempty_string(terminal.and_then(|v| v.get(FONT_FAMILY_KEY))) {
            terminal_font.family = family.to_owned().into();
        }
        terminal_font.size = font_size(terminal.and_then(|v| v.get(FONT_SIZE_KEY)));
        let workspace_editors = decode_list::<SavedWorkspaceEditor>(value(WORKSPACE_EDITORS_KEY))
            .unwrap_or_default()
            .into_iter()
            .map(|entry| (entry.root, entry.editor))
            .collect();
        Self {
            appearance: value(APPEARANCE_KEY)
                .and_then(toml::Value::as_str)
                .map(Appearance::from_str)
                .unwrap_or_default(),
            fps_monitor: value(FPS_MONITOR_KEY)
                .and_then(toml::Value::as_bool)
                .unwrap_or(false),
            notifications: value(NOTIFICATIONS_KEY)
                .and_then(toml::Value::as_bool)
                .unwrap_or(true),
            keep_awake: value(KEEP_AWAKE_KEY)
                .and_then(toml::Value::as_bool)
                .unwrap_or(false),
            auto_check_updates: updates
                .and_then(|v| v.get(AUTO_CHECK_KEY))
                .and_then(toml::Value::as_bool)
                .unwrap_or(true),
            update_channel: updates
                .and_then(|v| v.get(CHANNEL_KEY))
                .and_then(toml::Value::as_str)
                .map_or_else(UpdateChannel::of_this_build, UpdateChannel::from_str),
            terminal_font,
            terminal_color_scheme: nonempty_string(terminal.and_then(|v| v.get(COLOR_SCHEME_KEY)))
                .unwrap_or_default()
                .to_owned()
                .into(),
            code_theme: nonempty_string(code.and_then(|v| v.get(CODE_THEME_KEY)))
                .unwrap_or_default()
                .to_owned()
                .into(),
            code_font_size: font_size(code.and_then(|v| v.get(FONT_SIZE_KEY))),
            default_editor: nonempty_string(value(EDITOR_KEY)).map(str::to_owned),
            custom_editors,
            workspace_editors,
            path,
            device_key,
            servers,
            error: servers_error
                .clone()
                .or(key_error)
                .or(skipped_error)
                .or(editors_error),
            servers_error,
        }
    }
}

fn decode_list<T: serde::de::DeserializeOwned>(value: Option<&toml::Value>) -> io::Result<Vec<T>> {
    value.map_or_else(
        || Ok(Vec::new()),
        |value| value.clone().try_into().map_err(invalid_data),
    )
}

fn nonempty_string(value: Option<&toml::Value>) -> Option<&str> {
    value
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn font_size(value: Option<&toml::Value>) -> f32 {
    value
        .and_then(|value| {
            value
                .as_float()
                .or_else(|| value.as_integer().map(|v| v as f64))
        })
        .map_or_else(
            || TerminalFont::default().size,
            |size| TerminalFont::clamp_size(size as f32),
        )
}

impl Condr {
    pub(super) fn save_servers(&mut self, cx: &mut Context<Self>) {
        if let Some(error) = self.servers_error.clone() {
            self.report_error(error, cx);
            return;
        }
        let device_key = self.device_key.clone();
        let mut servers = self
            .connections
            .iter()
            .filter_map(|connection| {
                SavedServer::from_endpoint(connection.label.clone(), &connection.endpoint)
            })
            .collect::<Vec<_>>();
        self.save_config(cx, move |path| {
            // Entries that did not load (a TCP Device without our key, an address that no
            // longer parses) are not in `connections`; keep them as they are on disk.
            servers.extend(
                load_saved_servers(path)?
                    .into_iter()
                    .filter(|server| server.endpoint(device_key.as_ref()).is_err()),
            );
            save_saved_servers(path, servers)
        });
    }

    pub(super) fn save_appearance(&mut self, cx: &mut Context<Self>) {
        let appearance = self.appearance;
        self.save_config(cx, move |path| {
            write_client_value(path, APPEARANCE_KEY, toml_edit::value(appearance.as_str()))
        });
    }

    pub(super) fn save_fps_monitor(&mut self, cx: &mut Context<Self>) {
        let enabled = self.fps_monitor;
        self.save_config(cx, move |path| {
            write_client_value(path, FPS_MONITOR_KEY, toml_edit::value(enabled))
        });
    }

    pub(super) fn save_notifications(&mut self, cx: &mut Context<Self>) {
        let enabled = self.notifications;
        self.save_config(cx, move |path| {
            write_client_value(path, NOTIFICATIONS_KEY, toml_edit::value(enabled))
        });
    }

    pub(super) fn save_keep_awake(&mut self, cx: &mut Context<Self>) {
        let enabled = self.keep_awake;
        self.save_config(cx, move |path| {
            write_client_value(path, KEEP_AWAKE_KEY, toml_edit::value(enabled))
        });
    }

    pub(super) fn save_auto_check_updates(&mut self, cx: &mut Context<Self>) {
        let enabled = self.updates.automatic();
        self.save_config(cx, move |path| {
            write_value(
                path,
                &UPDATES_TABLE,
                AUTO_CHECK_KEY,
                toml_edit::value(enabled),
            )
        });
    }

    pub(super) fn save_update_channel(&mut self, cx: &mut Context<Self>) {
        let channel = self.updates.channel();
        self.save_config(cx, move |path| {
            write_value(
                path,
                &UPDATES_TABLE,
                CHANNEL_KEY,
                toml_edit::value(channel.as_str()),
            )
        });
    }

    pub(super) fn save_terminal_font(&mut self, cx: &mut Context<Self>) {
        let font = self.terminal_font.normalized();
        self.save_config(cx, move |path| {
            write_values(
                path,
                &TERMINAL_TABLE,
                vec![
                    (FONT_FAMILY_KEY, toml_edit::value(font.family.as_ref())),
                    (FONT_SIZE_KEY, toml_edit::value(f64::from(font.size))),
                ],
            )
        });
    }

    /// The last-used editor and every project's choice, written together: a launch
    /// changes both.
    pub(super) fn save_open_in_choices(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.default_editor.clone() else {
            return;
        };
        let workspace_editors = self.workspace_editors.clone();
        self.save_config(cx, move |path| {
            let mut saved = toml_edit::ArrayOfTables::new();
            for (root, editor) in workspace_editors {
                let mut table = toml_edit::Table::new();
                table["root"] = toml_edit::value(root.to_string_lossy().into_owned());
                table["editor"] = toml_edit::value(editor);
                saved.push(table);
            }
            write_values(
                path,
                &["client"],
                vec![
                    (EDITOR_KEY, toml_edit::value(editor)),
                    (WORKSPACE_EDITORS_KEY, toml_edit::Item::ArrayOfTables(saved)),
                ],
            )
        });
    }

    pub(super) fn save_terminal_color_scheme(&mut self, cx: &mut Context<Self>) {
        let name = self.terminal_color_scheme.clone();
        self.save_config(cx, move |path| {
            write_value(
                path,
                &TERMINAL_TABLE,
                COLOR_SCHEME_KEY,
                toml_edit::value(name.as_ref()),
            )
        });
    }

    pub(super) fn save_code_font_size(&mut self, cx: &mut Context<Self>) {
        let size = f64::from(self.code_font_size);
        self.save_config(cx, move |path| {
            write_value(path, &CODE_TABLE, FONT_SIZE_KEY, toml_edit::value(size))
        });
    }

    pub(super) fn save_code_theme(&mut self, cx: &mut Context<Self>) {
        let name = cx.global::<CodeTheme>().name.clone();
        self.save_config(cx, move |path| {
            write_value(
                path,
                &CODE_TABLE,
                CODE_THEME_KEY,
                toml_edit::value(name.as_ref()),
            )
        });
    }

    fn save_config(
        &mut self,
        cx: &mut Context<Self>,
        write: impl FnOnce(&Path) -> io::Result<()> + Send + 'static,
    ) {
        let Some(path) = self.client_config_path.clone() else {
            return;
        };
        let previous = self.config_save.take();
        let (result_tx, result_rx) = async_channel::bounded(1);
        // GPUI shutdown polls background work while the foreground queue is stopped.
        self.config_save = Some(cx.background_spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            let result =
                write(&path).map_err(|error| format!("Failed to save {}: {error}", path.display()));
            let _ = result_tx.try_send(result);
        }));
        cx.spawn(async move |this, cx| {
            if let Ok(Err(error)) = result_rx.recv().await {
                let _ = this.update(cx, |this, cx| this.report_error(error, cx));
            }
        })
        .detach();
    }
}

/// Rewrites one `[client]` key. The file is shared with the Server and meant to be
/// hand-editable, so this edits the parsed document in place and keeps every other
/// key, its comments and its formatting.
fn write_client_value(path: &Path, key: &str, value: toml_edit::Item) -> io::Result<()> {
    write_value(path, &["client"], key, value)
}

/// Like [`write_client_value`], for a key nested under `tables`, creating them as needed.
fn write_value(path: &Path, tables: &[&str], key: &str, value: toml_edit::Item) -> io::Result<()> {
    write_values(path, tables, vec![(key, value)])
}

/// Writes several keys of one table in a single read-modify-write, so related
/// values never land half-updated.
fn write_values(
    path: &Path,
    tables: &[&str],
    entries: Vec<(&str, toml_edit::Item)>,
) -> io::Result<()> {
    condr_core::update_config_values(
        path,
        tables,
        entries.into_iter().map(|(key, value)| (key, Some(value))),
    )
}

fn invalid_data(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_config(path: &Path) -> LoadedConfig {
        LoadedConfig::from_snapshot(
            Some(path.to_path_buf()),
            Ok(DeviceKey::from_seed([1; 32])),
            condr_core::read_config_value(path, &[], "client"),
        )
    }

    #[test]
    fn decoding_a_snapshot_does_not_mix_in_later_config_edits() {
        let directory = std::env::temp_dir().join(format!(
            "condr-client-snapshot-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.toml");
        fs::write(
            &path,
            "[client]\nappearance = 'dark'\n[client.terminal]\nfont_family = 'Cascadia Mono'\nfont_size = 17\n",
        )
        .unwrap();
        let snapshot = condr_core::read_config_value(&path, &[], "client");
        fs::write(&path, "this later edit is not valid TOML").unwrap();

        let config =
            LoadedConfig::from_snapshot(Some(path), Ok(DeviceKey::from_seed([1; 32])), snapshot);
        assert_eq!(config.appearance, Appearance::Dark);
        assert_eq!(config.terminal_font.family, "Cascadia Mono");
        assert_eq!(config.terminal_font.size, 17.);
        assert!(config.error.is_none());
        assert!(config.servers_error.is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn malformed_preferences_fall_back_independently_and_editors_report_an_error() {
        let directory = std::env::temp_dir().join(format!(
            "condr-client-partial-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.toml");
        fs::write(
            &path,
            "[client]\nappearance = 7\nfps_monitor = true\nnotifications = 'broken'\nworkspace_editors = false\n\
             [client.terminal]\nfont_family = 'Cascadia Mono'\nfont_size = 'broken'\n\
             [client.updates]\nauto_check = 'broken'\n\
             [[client.servers]]\nname = 'Build'\naddress = 'ssh://alice@build'\n\
             [[client.editors]]\nname = 'Missing command'\n",
        )
        .unwrap();

        let config = read_config(&path);
        assert_eq!(config.appearance, Appearance::System);
        assert!(config.fps_monitor);
        assert!(config.notifications);
        assert!(config.auto_check_updates);
        assert_eq!(config.terminal_font.family, "Cascadia Mono");
        assert_eq!(config.terminal_font.size, TerminalFont::default().size);
        assert!(config.workspace_editors.is_empty());
        assert!(config.custom_editors.is_empty());
        assert_eq!(config.servers.len(), 1);
        assert!(config.servers_error.is_none());
        assert!(config.error.unwrap().contains("[[client.editors]]"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn an_unreadable_snapshot_or_device_list_keeps_the_write_protection() {
        let directory = std::env::temp_dir().join(format!(
            "condr-client-protected-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.toml");
        for text in [
            "not valid TOML",
            "[client]\nappearance = 'dark'\n[[client.servers]]\nname = 'Missing address'\n",
        ] {
            fs::write(&path, text).unwrap();
            let config = read_config(&path);
            assert!(config.servers.is_empty());
            assert!(config.servers_error.is_some());
            assert_eq!(config.error, config.servers_error);
        }
        assert_eq!(read_config(&path).appearance, Appearance::Dark);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn saving_the_appearance_preserves_the_other_config_keys() {
        let directory = std::env::temp_dir().join(format!(
            "condr-client-appearance-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = directory.join("config.toml");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&path, "[server]\nlisten = '127.0.0.1:4242'\n").unwrap();
        save_saved_servers(
            &path,
            [SavedServer {
                name: "Linux".into(),
                address: format!("tcp://{}@127.0.0.1:4242", "0".repeat(64)),
            }],
        )
        .unwrap();

        assert_eq!(read_config(&path).appearance, Appearance::System);
        write_client_value(&path, APPEARANCE_KEY, Appearance::Dark.as_str().into()).unwrap();

        assert_eq!(read_config(&path).appearance, Appearance::Dark);
        assert_eq!(load_saved_servers(&path).unwrap().len(), 1);
        assert_eq!(
            condr_core::read_config_value(&path, &["server"], "listen")
                .unwrap()
                .as_ref()
                .and_then(toml::Value::as_str),
            Some("127.0.0.1:4242")
        );

        write_client_value(&path, APPEARANCE_KEY, toml_edit::value("solarized")).unwrap();
        assert_eq!(read_config(&path).appearance, Appearance::System);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn fps_monitor_defaults_off_and_round_trips() {
        let directory = std::env::temp_dir().join(format!(
            "condr-client-fps-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = directory.join("config.toml");
        fs::create_dir_all(&directory).unwrap();

        assert!(!read_config(&path).fps_monitor);
        write_client_value(&path, FPS_MONITOR_KEY, toml_edit::value(true)).unwrap();
        assert!(read_config(&path).fps_monitor);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn notifications_default_on_and_round_trip() {
        let directory = std::env::temp_dir().join(format!(
            "condr-client-notifications-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = directory.join("config.toml");
        fs::create_dir_all(&directory).unwrap();

        assert!(read_config(&path).notifications);
        write_client_value(&path, NOTIFICATIONS_KEY, toml_edit::value(false)).unwrap();
        assert!(!read_config(&path).notifications);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn terminal_font_round_trips_and_tolerates_bad_values() {
        let directory = std::env::temp_dir().join(format!(
            "condr-client-terminal-font-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = directory.join("config.toml");
        fs::create_dir_all(&directory).unwrap();

        assert_eq!(read_config(&path).terminal_font, TerminalFont::default());

        write_value(
            &path,
            &TERMINAL_TABLE,
            FONT_FAMILY_KEY,
            "Cascadia Mono".into(),
        )
        .unwrap();
        write_value(&path, &TERMINAL_TABLE, FONT_SIZE_KEY, toml_edit::value(15)).unwrap();
        let font = read_config(&path).terminal_font;
        assert_eq!(font.family.as_ref(), "Cascadia Mono");
        assert_eq!(font.size, 15.);
        let saved = fs::read_to_string(&path).unwrap();
        assert!(
            saved.contains("[client.terminal]"),
            "terminal keys must live in their own table:\n{saved}"
        );

        write_value(&path, &TERMINAL_TABLE, FONT_FAMILY_KEY, "   ".into()).unwrap();
        write_value(
            &path,
            &TERMINAL_TABLE,
            FONT_SIZE_KEY,
            toml_edit::value(1000),
        )
        .unwrap();
        let font = read_config(&path).terminal_font;
        assert_eq!(font.family, TerminalFont::default().family);
        assert_eq!(font.size, TerminalFont::MAX_SIZE);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn terminal_color_scheme_round_trips_and_defaults_to_empty() {
        let directory = std::env::temp_dir().join(format!(
            "condr-client-color-scheme-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = directory.join("config.toml");
        fs::create_dir_all(&directory).unwrap();

        assert_eq!(read_config(&path).terminal_color_scheme, "");
        write_value(
            &path,
            &TERMINAL_TABLE,
            COLOR_SCHEME_KEY,
            "Gruvbox Dark".into(),
        )
        .unwrap();
        assert_eq!(read_config(&path).terminal_color_scheme, "Gruvbox Dark");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn code_theme_and_font_size_round_trip_and_default() {
        let directory = std::env::temp_dir().join(format!(
            "condr-client-code-theme-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = directory.join("config.toml");
        fs::create_dir_all(&directory).unwrap();

        assert_eq!(read_config(&path).code_theme, "");
        write_value(&path, &CODE_TABLE, CODE_THEME_KEY, "Dracula".into()).unwrap();
        assert_eq!(read_config(&path).code_theme, "Dracula");

        assert_eq!(
            read_config(&path).code_font_size,
            TerminalFont::default().size
        );
        write_value(&path, &CODE_TABLE, FONT_SIZE_KEY, 17.into()).unwrap();
        assert_eq!(read_config(&path).code_font_size, 17.);
        write_value(&path, &CODE_TABLE, FONT_SIZE_KEY, 500.0.into()).unwrap();
        assert_eq!(read_config(&path).code_font_size, TerminalFont::MAX_SIZE);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn open_in_choices_round_trip_and_tolerate_a_bad_entry() {
        let directory = std::env::temp_dir().join(format!(
            "condr-client-open-in-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = directory.join("config.toml");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            &path,
            "[client]\nappearance = 'dark'\n\n[[client.editors]]\nname = 'Helix'\ncommand = ['wezterm', 'start', 'hx']\n",
        )
        .unwrap();

        assert_eq!(read_config(&path).default_editor, None);
        assert!(read_config(&path).workspace_editors.is_empty());
        assert_eq!(
            read_config(&path).custom_editors,
            [CustomEditor {
                name: "Helix".into(),
                command: vec!["wezterm".into(), "start".into(), "hx".into()],
            }]
        );

        let mut saved = toml_edit::ArrayOfTables::new();
        let mut table = toml_edit::Table::new();
        table["root"] = toml_edit::value("/repo/a");
        table["editor"] = toml_edit::value("zed");
        saved.push(table);
        write_values(
            &path,
            &["client"],
            vec![
                (EDITOR_KEY, toml_edit::value("zed")),
                (WORKSPACE_EDITORS_KEY, toml_edit::Item::ArrayOfTables(saved)),
            ],
        )
        .unwrap();

        assert_eq!(read_config(&path).default_editor.as_deref(), Some("zed"));
        assert_eq!(
            read_config(&path).workspace_editors,
            BTreeMap::from([(PathBuf::from("/repo/a"), "zed".to_owned())])
        );
        assert_eq!(read_config(&path).appearance, Appearance::Dark);
        assert_eq!(read_config(&path).custom_editors.len(), 1);

        // A custom entry missing its command is a load error, not a silent drop, so the
        // user learns why their editor is not in the menu.
        fs::write(&path, "[[client.editors]]\nname = 'Broken'\n").unwrap();
        assert!(
            read_config(&path)
                .error
                .unwrap()
                .contains("[[client.editors]]")
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn saving_keeps_a_hand_edited_config_readable() {
        let directory = std::env::temp_dir().join(format!(
            "condr-client-handedit-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = directory.join("config.toml");
        fs::create_dir_all(&directory).unwrap();
        let original = "\
# Which endpoint this Server listens on.
[server]
listen = '127.0.0.1:4242'   # keep the port in sync with the client

[client]
# Pinned so screenshots stay readable.
appearance = 'light'   # was system

# The Linux box in the corner.
[[client.servers]]
name = 'Linux'
address = 'tcp://AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA@127.0.0.1:4242'
";
        fs::write(&path, original).unwrap();

        write_client_value(&path, APPEARANCE_KEY, toml_edit::value("dark")).unwrap();

        let saved = fs::read_to_string(&path).unwrap();
        assert!(
            saved.contains("# Which endpoint this Server listens on."),
            "a hand-written comment must survive a save:\n{saved}"
        );
        assert!(
            saved.contains("# keep the port in sync with the client"),
            "a trailing comment must survive a save:\n{saved}"
        );
        assert!(
            saved.contains("# The Linux box in the corner."),
            "a comment on a rewritten section must survive a save:\n{saved}"
        );
        assert!(
            saved.contains("listen = '127.0.0.1:4242'"),
            "an untouched value must keep its original quoting:\n{saved}"
        );
        assert!(
            saved.contains("# Pinned so screenshots stay readable."),
            "a comment above the rewritten key must survive:\n{saved}"
        );
        assert!(
            saved.contains("# was system"),
            "a trailing comment on the rewritten key must survive:\n{saved}"
        );
        assert_eq!(read_config(&path).appearance, Appearance::Dark);
        assert_eq!(load_saved_servers(&path).unwrap().len(), 1);

        // Rewriting the server list regenerates it from the live connections, so its own
        // entries are reformatted, but nothing around them may be disturbed.
        save_saved_servers(
            &path,
            [SavedServer {
                name: "Linux".into(),
                address: format!("tcp://{}@127.0.0.1:4242", "0".repeat(64)),
            }],
        )
        .unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(
            saved.contains("# Which endpoint this Server listens on."),
            "rewriting the server list must not disturb the rest of the file:\n{saved}"
        );
        assert!(
            saved.contains("# Pinned so screenshots stay readable."),
            "rewriting the server list must not disturb the other client keys:\n{saved}"
        );
        assert!(saved.contains("# was system"), "{saved}");
        assert_eq!(load_saved_servers(&path).unwrap().len(), 1);
        fs::remove_dir_all(directory).unwrap();
    }
}
