use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use atomicwrites::{AllowOverwrite, AtomicFile};
use gpui::SharedString;
use serde::Deserialize;

use condr_server::{PublicKey, StaticKey, TcpEndpoint};

use super::{Appearance, Condr, Endpoint, TerminalFont};

const APPEARANCE_KEY: &str = "appearance";
const SERVERS_KEY: &str = "servers";
/// `[client.terminal]` holds every Terminal preference.
const TERMINAL_TABLE: [&str; 2] = ["client", "terminal"];
const FONT_FAMILY_KEY: &str = "font_family";
const FONT_SIZE_KEY: &str = "font_size";
const COLOR_SCHEME_KEY: &str = "color_scheme";

/// One `[[client.servers]]` entry. The Server's public key is what makes a saved TCP
/// Server trustworthy; invites are one-time and never written here.
#[derive(Deserialize)]
pub(super) struct SavedServer {
    pub name: String,
    pub address: SocketAddr,
    pub server_key: String,
}

impl SavedServer {
    pub(super) fn endpoint(&self, device_key: &StaticKey) -> io::Result<Endpoint> {
        Ok(Endpoint::tcp(TcpEndpoint {
            address: self.address,
            server_key: PublicKey::parse(&self.server_key)?,
            client_key: device_key.clone(),
            invite: None,
        }))
    }
}

pub(super) fn default_path() -> Option<PathBuf> {
    condr_core::config_directory().map(|root| root.join("config.toml"))
}

pub(super) fn load_servers(path: &Path) -> io::Result<Vec<SavedServer>> {
    read_client_value(path, SERVERS_KEY)?.map_or_else(|| Ok(Vec::new()), decode_servers)
}

/// A missing or unreadable preference follows the system rather than failing the load.
pub(super) fn load_appearance(path: &Path) -> io::Result<Appearance> {
    Ok(read_client_value(path, APPEARANCE_KEY)?
        .as_ref()
        .and_then(toml::Value::as_str)
        .map(Appearance::from_str)
        .unwrap_or_default())
}

/// A missing or malformed key keeps its default so the terminal always has a font.
pub(super) fn load_terminal_font(path: &Path) -> io::Result<TerminalFont> {
    let mut font = TerminalFont::default();
    if let Some(family) = read_value(path, &TERMINAL_TABLE, FONT_FAMILY_KEY)?
        .as_ref()
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|family| !family.is_empty())
    {
        font.family = family.to_string().into();
    }
    if let Some(size) = read_value(path, &TERMINAL_TABLE, FONT_SIZE_KEY)?.and_then(|value| {
        value
            .as_float()
            .or_else(|| value.as_integer().map(|size| size as f64))
    }) {
        font.size = TerminalFont::clamp_size(size as f32);
    }
    Ok(font)
}

/// Empty when unset; the caller resolves unknown names to the default palette.
pub(super) fn load_terminal_color_scheme(path: &Path) -> io::Result<SharedString> {
    Ok(read_value(path, &TERMINAL_TABLE, COLOR_SCHEME_KEY)?
        .as_ref()
        .and_then(toml::Value::as_str)
        .map(|name| name.trim().to_string().into())
        .unwrap_or_default())
}

fn read_client_value(path: &Path, key: &str) -> io::Result<Option<toml::Value>> {
    read_value(path, &["client"], key)
}

fn read_value(path: &Path, tables: &[&str], key: &str) -> io::Result<Option<toml::Value>> {
    let root = read_root(path)?;
    let mut table = Some(&root);
    for name in tables {
        table = table
            .and_then(|table| table.get(*name))
            .and_then(toml::Value::as_table);
    }
    Ok(table.and_then(|table| table.get(key)).cloned())
}

fn decode_servers(value: toml::Value) -> io::Result<Vec<SavedServer>> {
    value.try_into().map_err(invalid_data)
}

impl Condr {
    pub(super) fn save_servers(&mut self) {
        let Some(path) = self.client_config_path.as_deref() else {
            return;
        };
        // Without a device key no saved TCP Server was loaded; rewriting the list now
        // would erase them.
        if self.device_key.is_none() {
            return;
        }
        let servers = self.connections.iter().filter_map(|connection| {
            let Endpoint::Tcp(tcp) = &connection.endpoint else {
                return None;
            };
            Some(SavedServer {
                name: connection.label.clone(),
                address: tcp.address,
                server_key: tcp.server_key.to_hex(),
            })
        });
        if let Err(error) = write_servers(path, servers) {
            self.app_error = Some(format!("Failed to save config: {error}"));
        }
    }

    pub(super) fn save_appearance(&mut self) {
        let Some(path) = self.client_config_path.as_deref() else {
            return;
        };
        let appearance = self.appearance;
        if let Err(error) =
            write_client_value(path, APPEARANCE_KEY, toml_edit::value(appearance.as_str()))
        {
            self.app_error = Some(format!("Failed to save config: {error}"));
        }
    }

    pub(super) fn save_terminal_font(&mut self) {
        let Some(path) = self.client_config_path.as_deref() else {
            return;
        };
        let font = self.terminal_font.normalized();
        if let Err(error) = write_values(
            path,
            &TERMINAL_TABLE,
            vec![
                (FONT_FAMILY_KEY, toml_edit::value(font.family.as_ref())),
                (FONT_SIZE_KEY, toml_edit::value(f64::from(font.size))),
            ],
        ) {
            self.app_error = Some(format!("Failed to save config: {error}"));
        }
    }

    pub(super) fn save_terminal_color_scheme(&mut self) {
        let Some(path) = self.client_config_path.as_deref() else {
            return;
        };
        let name = self.terminal_color_scheme.clone();
        if let Err(error) = write_value(
            path,
            &TERMINAL_TABLE,
            COLOR_SCHEME_KEY,
            toml_edit::value(name.as_ref()),
        ) {
            self.app_error = Some(format!("Failed to save config: {error}"));
        }
    }
}

fn write_servers(path: &Path, servers: impl IntoIterator<Item = SavedServer>) -> io::Result<()> {
    let mut saved = toml_edit::ArrayOfTables::new();
    for server in servers {
        let mut table = toml_edit::Table::new();
        table["name"] = toml_edit::value(server.name);
        table["address"] = toml_edit::value(server.address.to_string());
        table["server_key"] = toml_edit::value(server.server_key);
        saved.push(table);
    }
    write_client_value(path, SERVERS_KEY, toml_edit::Item::ArrayOfTables(saved))
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
    let mut document = read_document(path)?;
    let mut table: &mut dyn toml_edit::TableLike = document.as_table_mut();
    for name in tables {
        table = table
            .entry(name)
            .or_insert(toml_edit::table())
            .as_table_like_mut()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{name} must be a table"),
                )
            })?;
    }
    for (key, value) in entries {
        // `insert` replaces the key as well as the value, taking the comments around
        // both with it. Overwriting an existing scalar in place keeps them.
        match (
            table.get_mut(key).and_then(toml_edit::Item::as_value_mut),
            value.as_value(),
        ) {
            (Some(existing), Some(replacement)) => {
                let decor = existing.decor().clone();
                *existing = replacement.clone();
                *existing.decor_mut() = decor;
            }
            _ => {
                table.insert(key, value);
            }
        }
    }
    let text = document.to_string();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    // A symlinked config.toml (dotfiles) is updated through its target, not replaced.
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    AtomicFile::new(&target, AllowOverwrite)
        .write_with_options(|file| file.write_all(text.as_bytes()), options)
        .map_err(io::Error::from)
}

/// Reads for the typed load path. `toml` deserializes a hand-edited value into
/// `SavedServer` and reports a useful error; `toml_edit` is only for writing back.
fn read_root(path: &Path) -> io::Result<toml::Table> {
    match read_config_text(path)? {
        Some(text) => toml::from_str(&text).map_err(invalid_data),
        None => Ok(toml::Table::new()),
    }
}

fn read_document(path: &Path) -> io::Result<toml_edit::DocumentMut> {
    match read_config_text(path)? {
        Some(text) => text.parse().map_err(invalid_data),
        None => Ok(toml_edit::DocumentMut::new()),
    }
}

fn read_config_text(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn invalid_data(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_the_platform_config_directory() {
        assert_eq!(
            default_path(),
            condr_core::config_directory().map(|root| root.join("config.toml"))
        );
    }

    #[test]
    fn saving_client_servers_preserves_server_config() {
        let directory =
            std::env::temp_dir().join(format!("condr-client-config-{}", std::process::id()));
        let path = directory.join("config.toml");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&path, "[server]\nlisten = '127.0.0.1:4242'\n").unwrap();

        write_servers(
            &path,
            [SavedServer {
                name: "Linux".into(),
                address: "127.0.0.1:4242".parse().unwrap(),
                server_key: "0".repeat(64),
            }],
        )
        .unwrap();

        let root = read_root(&path).unwrap();
        assert_eq!(root["server"]["listen"].as_str(), Some("127.0.0.1:4242"));
        let servers = load_servers(&path).unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "Linux");
        assert_eq!(servers[0].address, "127.0.0.1:4242".parse().unwrap());
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
        write_servers(
            &path,
            [SavedServer {
                name: "Linux".into(),
                address: "127.0.0.1:4242".parse().unwrap(),
                server_key: "0".repeat(64),
            }],
        )
        .unwrap();

        assert_eq!(load_appearance(&path).unwrap(), Appearance::System);
        write_client_value(&path, APPEARANCE_KEY, Appearance::Dark.as_str().into()).unwrap();

        assert_eq!(load_appearance(&path).unwrap(), Appearance::Dark);
        assert_eq!(load_servers(&path).unwrap().len(), 1);
        let root = read_root(&path).unwrap();
        assert_eq!(root["server"]["listen"].as_str(), Some("127.0.0.1:4242"));

        write_client_value(&path, APPEARANCE_KEY, toml_edit::value("solarized")).unwrap();
        assert_eq!(load_appearance(&path).unwrap(), Appearance::System);
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

        assert_eq!(load_terminal_font(&path).unwrap(), TerminalFont::default());

        write_value(
            &path,
            &TERMINAL_TABLE,
            FONT_FAMILY_KEY,
            "Cascadia Mono".into(),
        )
        .unwrap();
        write_value(&path, &TERMINAL_TABLE, FONT_SIZE_KEY, toml_edit::value(15)).unwrap();
        let font = load_terminal_font(&path).unwrap();
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
        let font = load_terminal_font(&path).unwrap();
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

        assert_eq!(load_terminal_color_scheme(&path).unwrap(), "");
        write_value(
            &path,
            &TERMINAL_TABLE,
            COLOR_SCHEME_KEY,
            "Gruvbox Dark".into(),
        )
        .unwrap();
        assert_eq!(load_terminal_color_scheme(&path).unwrap(), "Gruvbox Dark");
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
address = '127.0.0.1:4242'
server_key = '0000000000000000000000000000000000000000000000000000000000000000'
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
        assert_eq!(load_appearance(&path).unwrap(), Appearance::Dark);
        assert_eq!(load_servers(&path).unwrap().len(), 1);

        // Rewriting the server list regenerates it from the live connections, so its own
        // entries are reformatted, but nothing around them may be disturbed.
        write_servers(
            &path,
            [SavedServer {
                name: "Linux".into(),
                address: "127.0.0.1:4242".parse().unwrap(),
                server_key: "0".repeat(64),
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
        assert_eq!(load_servers(&path).unwrap().len(), 1);
        fs::remove_dir_all(directory).unwrap();
    }
}
