use super::*;

/// One Server per host: it always answers on a private local socket, and on a TCP
/// address as well when `[server] listen` is configured. Both reach the same Session.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub socket_path: PathBuf,
    pub listen: Option<std::net::SocketAddr>,
    pub(super) snapshot_path: Option<PathBuf>,
    /// The Server's own `config.toml`; `None` keeps settings in memory only.
    pub(super) config_path: Option<PathBuf>,
    /// The key the TCP listener answers with; `None` loads the host identity at bind time.
    pub(super) identity: Option<Arc<ServerIdentity>>,
    /// The device key `ephemeral_tcp` authorized, so tests can connect over TCP.
    pub(super) test_device: Option<StaticKey>,
}

impl Default for ServerConfig {
    /// The host's Server: default socket, and the TCP address `config.toml` names, if any.
    fn default() -> Self {
        let config_path = condr_core::config_directory().map(|root| root.join("config.toml"));
        let socket_path = default_socket_path();
        Self {
            listen: load_listen(config_path.as_deref()),
            snapshot_path: default_snapshot_path(&socket_path),
            socket_path,
            config_path,
            identity: None,
            test_device: None,
        }
    }
}

/// `[server] listen` from `config.toml`: the TCP address the Server also answers on.
/// Absent, blank or malformed means no TCP listener.
pub fn load_listen(path: Option<&std::path::Path>) -> Option<std::net::SocketAddr> {
    load_server_setting(path?, "listen")?.trim().parse().ok()
}

/// Persists `[server] listen`, or removes it for `None`.
pub fn save_listen(path: &std::path::Path, listen: Option<std::net::SocketAddr>) -> io::Result<()> {
    save_server_setting(
        path,
        "listen",
        listen.map(|address| toml_edit::value(address.to_string())),
    )
}

fn load_server_setting(path: &std::path::Path, key: &str) -> Option<String> {
    crate::persistence::read_config_text(path)
        .ok()??
        .parse::<toml::Table>()
        .ok()?
        .get("server")?
        .get(key)?
        .as_str()
        .map(str::to_owned)
}

/// `[server.terminal] shell` from the Server's `config.toml`, blank when unset or the
/// file is missing or malformed. Read once at startup; hand edits need a restart.
pub(super) fn load_shell(path: Option<&std::path::Path>) -> String {
    let Some(path) = path else {
        return String::new();
    };
    crate::persistence::read_config_text(path)
        .ok()
        .flatten()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .and_then(|root| {
            root.get("server")?
                .get("terminal")?
                .get("shell")?
                .as_str()
                .map(|shell| shell.trim().to_owned())
        })
        .unwrap_or_default()
}

/// Writes `[server.terminal] shell` back, keeping the rest of the hand-editable file
/// (other keys, comments, formatting) as it was.
pub(super) fn save_shell(path: &std::path::Path, shell: &str) -> io::Result<()> {
    save_setting(
        path,
        &["server", "terminal"],
        "shell",
        Some(toml_edit::value(shell)),
    )
}

fn save_server_setting(
    path: &std::path::Path,
    key: &str,
    value: Option<toml_edit::Item>,
) -> io::Result<()> {
    save_setting(path, &["server"], key, value)
}

/// Sets or, for `None`, removes one key under `tables` in `config.toml`, leaving every
/// other key, comment and line as written.
fn save_setting(
    path: &std::path::Path,
    tables: &[&str],
    key: &str,
    value: Option<toml_edit::Item>,
) -> io::Result<()> {
    crate::persistence::update_config_values(path, tables, [(key, value)])
}

impl ServerConfig {
    /// The host's Server on a specific socket, with the default snapshot for that socket.
    pub fn at_socket(socket_path: impl Into<PathBuf>) -> Self {
        let socket_path = socket_path.into();
        Self {
            listen: None,
            snapshot_path: default_snapshot_path(&socket_path),
            socket_path,
            config_path: condr_core::config_directory().map(|root| root.join("config.toml")),
            identity: None,
            test_device: None,
        }
    }

    /// A Server on `socket_path` with no persistence and no TCP listener.
    pub fn ephemeral(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
            listen: None,
            snapshot_path: None,
            config_path: None,
            identity: None,
            test_device: None,
        }
    }

    /// An ephemeral Server that also listens on `address` with a fresh identity accepting
    /// exactly one fresh device key. [`BoundServer::endpoint`] then connects as that device.
    pub fn ephemeral_tcp(address: std::net::SocketAddr) -> io::Result<Self> {
        let client_key = StaticKey::generate()?;
        let identity = ServerIdentity::ephemeral()?.with_authorized(client_key.public());
        let socket_path = std::env::temp_dir().join(format!(
            "condr-tcp-{}-{}.sock",
            std::process::id(),
            runtime_epoch()
        ));
        Ok(Self {
            socket_path,
            listen: Some(address),
            snapshot_path: None,
            config_path: None,
            identity: Some(Arc::new(identity)),
            test_device: Some(client_key),
        })
    }

    pub fn with_listen(mut self, address: std::net::SocketAddr) -> Self {
        self.listen = Some(address);
        self
    }

    pub fn with_identity(mut self, identity: ServerIdentity) -> Self {
        self.identity = Some(Arc::new(identity));
        self
    }

    pub fn local_endpoint(&self) -> Endpoint {
        Endpoint::local(&self.socket_path)
    }

    pub fn with_snapshot_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.snapshot_path = Some(path.into());
        self
    }

    pub fn with_config_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config_path = Some(path.into());
        self
    }

    pub fn snapshot_path(&self) -> Option<&std::path::Path> {
        self.snapshot_path.as_deref()
    }
}
