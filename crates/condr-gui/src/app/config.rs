use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use atomicwrites::{AllowOverwrite, AtomicFile};
use serde::{Deserialize, Serialize};

use super::{Condr, Endpoint};

#[derive(Deserialize, Serialize)]
pub(super) struct SavedServer {
    pub name: String,
    pub address: SocketAddr,
}

pub(super) fn default_path() -> Option<PathBuf> {
    condr_core::config_directory().map(|root| root.join("config.toml"))
}

pub(super) fn load_servers(path: &Path) -> io::Result<Vec<SavedServer>> {
    let root = read_root(path)?;
    root.get("client")
        .and_then(toml::Value::as_table)
        .and_then(|client| client.get("servers"))
        .cloned()
        .map_or_else(|| Ok(Vec::new()), decode_servers)
}

fn decode_servers(value: toml::Value) -> io::Result<Vec<SavedServer>> {
    value.try_into().map_err(invalid_data)
}

impl Condr {
    pub(super) fn save_servers(&mut self) {
        let Some(path) = self.client_config_path.as_deref() else {
            return;
        };
        let servers = self.connections.iter().filter_map(|connection| {
            let Endpoint::Tcp(address) = &connection.endpoint else {
                return None;
            };
            Some(SavedServer {
                name: connection.label.clone(),
                address: *address,
            })
        });
        if let Err(error) = write_servers(path, servers) {
            self.app_error = Some(format!("Failed to save config: {error}"));
        }
    }
}

fn write_servers(path: &Path, servers: impl IntoIterator<Item = SavedServer>) -> io::Result<()> {
    let mut root = read_root(path)?;
    let client = root
        .entry("client")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "client must be a table"))?;
    client.insert(
        "servers".into(),
        toml::Value::try_from(servers.into_iter().collect::<Vec<_>>()).map_err(invalid_data)?,
    );
    let text = toml::to_string_pretty(&root).map_err(invalid_data)?;

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
    AtomicFile::new(path, AllowOverwrite)
        .write_with_options(|file| file.write_all(text.as_bytes()), options)
        .map_err(io::Error::from)
}

fn read_root(path: &Path) -> io::Result<toml::Table> {
    match fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(invalid_data),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(toml::Table::new()),
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
}
