//! `[[client.servers]]`: the Devices a client has paired with, shared by the GUI and
//! the `condr` CLI so both name the same Device the same way.

use std::io;
use std::path::Path;

use serde::Deserialize;

use super::Endpoint;
use crate::noise::DeviceKey;

const CLIENT_TABLE: [&str; 1] = ["client"];
const SERVERS_KEY: &str = "servers";

/// One `[[client.servers]]` entry. The Server's public key is what makes a saved TCP
/// Server trustworthy; invites are one-time and never written here.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct SavedServer {
    pub name: String,
    /// `tcp://<server key>@host:port` or `ssh://[user@]host[:port]`; never an invite.
    pub address: String,
}

impl SavedServer {
    /// `None` for a local socket: only remote Devices are worth remembering.
    pub fn from_endpoint(name: impl Into<String>, endpoint: &Endpoint) -> Option<Self> {
        let address = match endpoint {
            Endpoint::Local(_) => return None,
            Endpoint::Tcp(tcp) => format!("tcp://{}@{}", tcp.server_key, tcp.authority()),
            Endpoint::Ssh(ssh) => ssh.to_string(),
        };
        Some(Self {
            name: name.into(),
            address,
        })
    }

    pub fn endpoint(&self, device_key: Option<&DeviceKey>) -> io::Result<Endpoint> {
        Endpoint::parse(&self.address, device_key)
    }

    pub fn is_tcp(&self) -> bool {
        self.address.trim().starts_with("tcp://")
    }
}

/// Every `[[client.servers]]` entry; malformed TOML is `InvalidData`.
pub fn load_saved_servers(path: &Path) -> io::Result<Vec<SavedServer>> {
    condr_core::read_config_value(path, &CLIENT_TABLE, SERVERS_KEY)?.map_or_else(
        || Ok(Vec::new()),
        |value| {
            value
                .try_into()
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
        },
    )
}

/// Rewrites `[[client.servers]]` in place, keeping every other key and its formatting.
pub fn save_saved_servers(
    path: &Path,
    servers: impl IntoIterator<Item = SavedServer>,
) -> io::Result<()> {
    let mut saved = toml_edit::ArrayOfTables::new();
    for server in servers {
        let mut table = toml_edit::Table::new();
        table["name"] = toml_edit::value(server.name);
        table["address"] = toml_edit::value(server.address);
        saved.push(table);
    }
    condr_core::update_config_values(
        path,
        &CLIENT_TABLE,
        [(SERVERS_KEY, Some(toml_edit::Item::ArrayOfTables(saved)))],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_config(tag: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "condr-saved-servers-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&directory).unwrap();
        directory.join("config.toml")
    }

    #[test]
    fn ssh_entry_keeps_the_binary_path_and_needs_no_device_key() {
        let path = temp_config("ssh");
        let ssh = crate::SshEndpoint::parse("ssh://alice@build:2222?bin=/opt/a%20b/condr").unwrap();
        let server = SavedServer::from_endpoint("Build", &Endpoint::Ssh(ssh.clone())).unwrap();
        assert_eq!(server.address, ssh.to_string());
        save_saved_servers(&path, [server]).unwrap();
        let loaded = load_saved_servers(&path).unwrap();
        assert_eq!(loaded[0].name, "Build");
        let Endpoint::Ssh(loaded) = loaded[0].endpoint(None).unwrap() else {
            panic!("expected SSH");
        };
        assert_eq!(loaded.binary(), "/opt/a b/condr");
        assert_eq!(loaded, ssh);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn saving_preserves_the_server_table_and_local_endpoints_are_not_saved() {
        let path = temp_config("preserve");
        fs::write(&path, "[server]\nlisten = '127.0.0.1:4242'\n").unwrap();
        assert!(SavedServer::from_endpoint("Here", &Endpoint::Local("/tmp/x".into())).is_none());
        let address = format!("tcp://{}@127.0.0.1:4242", "0".repeat(64));
        save_saved_servers(
            &path,
            [SavedServer {
                name: "Linux".into(),
                address: address.clone(),
            }],
        )
        .unwrap();

        assert_eq!(
            condr_core::read_config_value(&path, &["server"], "listen")
                .unwrap()
                .and_then(|value| value.as_str().map(str::to_owned)),
            Some("127.0.0.1:4242".to_owned())
        );
        let servers = load_saved_servers(&path).unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "Linux");
        assert_eq!(servers[0].address, address);
        assert!(servers[0].is_tcp());
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
