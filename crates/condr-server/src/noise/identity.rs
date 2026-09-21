//! Host identity, authorized devices and one-time invitation storage.

use super::*;

/// How long an invite stays redeemable.
pub const INVITE_TTL: Duration = Duration::from_secs(10 * 60);

const DEVICE_KEY_FILE: &str = "device-key";

const AUTHORIZED_FILE: &str = "authorized-clients";

const INVITE_FILE: &str = "pending-invite";

const LOCK_FILE: &str = "identity.lock";

/// The Server host's Device key plus the devices it accepts.
///
/// With a store directory the authorized list and the pending invite live in files that
/// `condr server invite|clients|revoke` edit on the same host; without one only
/// `always_authorized` peers can connect.
pub struct ServerIdentity {
    pub(super) key: DeviceKey,
    store: Option<PathBuf>,
    always_authorized: Vec<PublicKey>,
}

/// One line of `authorized-clients`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizedClient {
    pub key: PublicKey,
    pub paired_at: u64,
    /// When the device last completed a handshake, as Unix seconds.
    pub last_seen: u64,
    pub name: String,
}

/// The one redeemable invite of a Server; a new invite replaces it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Invite {
    pub secret: Secret,
    pub expires_at: u64,
}

impl ServerIdentity {
    /// The identity stored in `directory`: `device-key`, the one key this machine also
    /// connects out with, plus the paired devices and the pending invite kept beside it.
    /// Processes on the host itself use the local socket, so no device is authorized by
    /// default. A machine whose key is created now has no paired devices either: an
    /// `authorized-clients` left from before the key existed can never match, so it goes.
    pub fn load_or_create(directory: &Path) -> io::Result<Self> {
        let key = with_store_lock(directory, || {
            let path = directory.join(DEVICE_KEY_FILE);
            if !path.exists() {
                match fs::remove_file(directory.join(AUTHORIZED_FILE)) {
                    Ok(()) => {
                        tracing::warn!("device-key created; discarded the old authorized-clients")
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            DeviceKey::load_or_create(&path)
        })?;
        Ok(Self {
            key,
            store: Some(directory.to_path_buf()),
            always_authorized: Vec::new(),
        })
    }

    /// A fresh key with no store, for tests and short-lived Servers.
    pub fn ephemeral() -> io::Result<Self> {
        Ok(Self {
            key: DeviceKey::generate()?,
            store: None,
            always_authorized: Vec::new(),
        })
    }

    pub fn with_authorized(mut self, peer: PublicKey) -> Self {
        self.always_authorized.push(peer);
        self
    }

    pub fn public_key(&self) -> PublicKey {
        self.key.public()
    }

    /// Whether `remote` may connect without an invite, as of the store on disk right now.
    pub fn is_authorized(&self, remote: &PublicKey) -> io::Result<bool> {
        if self.always_authorized.contains(remote) {
            return Ok(true);
        }
        let Some(store) = &self.store else {
            return Ok(false);
        };
        Ok(read_authorized(store)?
            .iter()
            .any(|client| client.key == *remote))
    }

    /// Notes that `remote` connected now; a device the store does not list is ignored.
    pub fn record_seen(&self, remote: &PublicKey) -> io::Result<()> {
        let Some(store) = &self.store else {
            return Ok(());
        };
        with_store_lock(store, || {
            let mut clients = read_authorized(store)?;
            let Some(client) = clients.iter_mut().find(|client| client.key == *remote) else {
                return Ok(());
            };
            client.last_seen = now();
            write_authorized(store, &clients)
        })
    }

    /// The pre-shared key to answer `remote` with: the zero key for an authorized device,
    /// or the pending invite for an unknown one. `None` refuses the peer.
    pub(super) fn psk_for(
        &self,
        remote: &PublicKey,
    ) -> io::Result<Option<(Secret, Option<Secret>)>> {
        if self.is_authorized(remote)? {
            return Ok(Some((Secret([0; 32]), None)));
        }
        let Some(store) = &self.store else {
            return Ok(None);
        };
        Ok(read_invite(store)?.map(|invite| (invite.secret.clone(), Some(invite.secret))))
    }

    /// Records `remote` as paired, provided `invite` is still the pending one: the first
    /// device to finish wins, and a stale invite cannot consume a newer one.
    pub(super) fn complete_pairing(
        &self,
        remote: PublicKey,
        name: &str,
        invite: &Secret,
    ) -> io::Result<()> {
        let store = self.store.as_deref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "this Server cannot pair devices",
            )
        })?;
        with_store_lock(store, || {
            if read_invite(store)?.is_none_or(|current| current.secret != *invite) {
                return Err(rejected("invite already used or expired"));
            }
            let mut clients = read_authorized(store)?;
            if !clients.iter().any(|client| client.key == remote) {
                let paired_at = now();
                clients.push(AuthorizedClient {
                    key: remote,
                    paired_at,
                    last_seen: paired_at,
                    name: name.split_whitespace().collect::<Vec<_>>().join(" "),
                });
                write_authorized(store, &clients)?;
            }
            match fs::remove_file(store.join(INVITE_FILE)) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            }
        })
    }
}

impl fmt::Debug for ServerIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerIdentity")
            .field("public_key", &self.key.public)
            .field("store", &self.store)
            .field("always_authorized", &self.always_authorized)
            .finish()
    }
}

/// Where this host keeps `device-key`, the authorized list and the invite.
pub fn identity_directory() -> io::Result<PathBuf> {
    condr_core::config_directory().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no configuration directory is available; set CONDR_CONFIG_DIR",
        )
    })
}

/// How a GUI introduces itself in `Hello`, and so how a paired device is listed: the
/// host name, or `condr` when the system does not report one.
pub fn device_name() -> String {
    #[cfg(windows)]
    let host = std::env::var("COMPUTERNAME").ok();
    #[cfg(unix)]
    let host = nix::unistd::gethostname()
        .ok()
        .map(|name| name.to_string_lossy().into_owned());
    host.map(|host| host.trim().to_owned())
        .filter(|host| !host.is_empty())
        .unwrap_or_else(|| "condr".to_owned())
}

/// This machine's one key, which its GUI and CLI present to TCP Servers and its own
/// Server answers with; `ServerIdentity::load_or_create` reads the same file.
pub fn load_device_key(directory: &Path) -> io::Result<DeviceKey> {
    DeviceKey::load_or_create(&directory.join(DEVICE_KEY_FILE))
}

/// Replaces the pending invite with a fresh one valid for [`INVITE_TTL`].
pub fn create_invite(directory: &Path) -> io::Result<Invite> {
    let invite = Invite {
        secret: Secret::generate()?,
        expires_at: now() + INVITE_TTL.as_secs(),
    };
    with_store_lock(directory, || {
        let _ = fs::remove_file(directory.join(INVITE_FILE));
        write_secret_file(
            &directory.join(INVITE_FILE),
            &format!("{} {}", invite.secret.to_hex(), invite.expires_at),
        )
    })?;
    Ok(invite)
}

/// The pending invite, if one exists and has not expired.
fn read_invite(directory: &Path) -> io::Result<Option<Invite>> {
    let Some(text) = read_secret_file(&directory.join(INVITE_FILE))? else {
        return Ok(None);
    };
    let mut fields = text.split_whitespace();
    let (Some(secret), Some(expires_at)) = (fields.next(), fields.next()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "malformed pending-invite file",
        ));
    };
    let invite = Invite {
        secret: Secret::parse(secret)?,
        expires_at: expires_at
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "malformed invite expiry"))?,
    };
    Ok((invite.expires_at > now()).then_some(invite))
}

pub fn read_authorized(directory: &Path) -> io::Result<Vec<AuthorizedClient>> {
    let text = match fs::read_to_string(directory.join(AUTHORIZED_FILE)) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let malformed = || {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("malformed authorized-clients line: {line}"),
                )
            };
            let mut fields = line.splitn(4, ' ');
            let (Some(key), Some(paired_at), Some(last_seen), Some(name)) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                return Err(malformed());
            };
            Ok(AuthorizedClient {
                key: PublicKey::parse(key)?,
                paired_at: paired_at.parse().map_err(|_| malformed())?,
                last_seen: last_seen.parse().map_err(|_| malformed())?,
                name: name.to_owned(),
            })
        })
        .collect()
}

/// Removes the unique client matching `prefix` and returns its full key.
/// Ambiguity leaves the store untouched; an unmatched prefix returns `None`.
/// This refuses their next handshake; `ClientMessage::RevokeDevice` closes the
/// connections they hold now.
pub fn revoke(directory: &Path, prefix: &str) -> io::Result<Option<PublicKey>> {
    with_store_lock(directory, || {
        let mut clients = read_authorized(directory)?;
        let mut matches = clients
            .iter()
            .filter(|client| client.key.matches_prefix(prefix));
        let Some(key) = matches.next().map(|client| client.key) else {
            return Ok(None);
        };
        if matches.next().is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ambiguous device prefix; use a longer fingerprint",
            ));
        }
        clients.retain(|client| client.key != key);
        write_authorized(directory, &clients)?;
        Ok(Some(key))
    })
}

fn write_authorized(directory: &Path, clients: &[AuthorizedClient]) -> io::Result<()> {
    let text = clients
        .iter()
        .map(|client| {
            format!(
                "{} {} {} {}\n",
                client.key, client.paired_at, client.last_seen, client.name
            )
        })
        .collect::<String>();
    fs::create_dir_all(directory)?;
    let target = crate::persistence::resolve_write_target(&directory.join(AUTHORIZED_FILE));
    atomicwrites::AtomicFile::new(&target, atomicwrites::AllowOverwrite)
        .write(|file| file.write_all(text.as_bytes()))
        .map_err(|error| match error {
            atomicwrites::Error::Internal(error) | atomicwrites::Error::User(error) => error,
        })
}

/// Serializes read-modify-write cycles between the running Server and the CLI.
fn with_store_lock<T>(directory: &Path, f: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    fs::create_dir_all(directory)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join(LOCK_FILE))?;
    lock.lock()?;
    let result = f();
    let _ = lock.unlock();
    result
}

/// Reads a secret file, `None` when absent. On Unix a file readable by others is refused
/// rather than trusted, since a leaked key must be replaced, not reused.
pub(super) fn read_secret_file(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                let mode = fs::metadata(path)?.permissions().mode() & 0o077;
                if mode != 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!(
                            "{} is readable by other users (mode {:o}); make it 0600 or delete it",
                            path.display(),
                            fs::metadata(path)?.permissions().mode() & 0o777
                        ),
                    ));
                }
            }
            Ok(Some(text))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Creates `path` owner-only and refuses to overwrite an existing file.
pub(super) fn write_secret_file(path: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file: File = options.open(path)?;
    file.write_all(text.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()
}

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests;
