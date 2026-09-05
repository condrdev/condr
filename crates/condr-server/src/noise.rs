//! Authentication and encryption for TCP endpoints, modelled on WireGuard.
//!
//! Both sides hold a persistent X25519 static key. A connection is one
//! `Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s` handshake: the Client already knows the Server's
//! public key, sends its own static key encrypted in the first message, and the Server only
//! answers peers it recognises. An authorized peer uses the all-zero pre-shared key, as an
//! unconfigured WireGuard peer does. Pairing a new device uses a one-time, short-lived invite
//! secret as that pre-shared key instead; the Server records the device's public key once the
//! first transport message proves the Client held the secret.
//!
//! After the handshake every byte of the ordinary framed protocol travels inside Noise
//! transport records of at most 64 KiB (`u16` big-endian length, then ciphertext).

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use snow::{HandshakeState, TransportState};

const NOISE_PATTERN: &str = "Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s";
/// `IKpsk2` mixes the pre-shared key into the second message.
const PSK_LOCATION: usize = 2;
const TAG_LEN: usize = 16;
const MAX_RECORD: usize = 65535;
const MAX_PLAINTEXT: usize = MAX_RECORD - TAG_LEN;
/// How long an invite stays redeemable.
pub const INVITE_TTL: Duration = Duration::from_secs(10 * 60);

const SERVER_KEY_FILE: &str = "server-key";
const CLIENT_KEY_FILE: &str = "client-key";
const AUTHORIZED_FILE: &str = "authorized-clients";
const INVITE_FILE: &str = "pending-invite";
const LOCK_FILE: &str = "identity.lock";

/// An X25519 public key; its hex form is the fingerprint shown to people.
#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub struct PublicKey([u8; 32]);

/// An X25519 static key pair. Debug output never shows the private half.
#[derive(Clone, Eq, PartialEq)]
pub struct StaticKey {
    private: [u8; 32],
    public: PublicKey,
}

/// A 32-byte pre-shared secret: an invite. Debug output never shows it.
#[derive(Clone, Eq, PartialEq)]
pub struct Secret([u8; 32]);

impl PublicKey {
    pub fn parse(hex: &str) -> io::Result<Self> {
        hex_decode(hex).map(Self)
    }

    pub fn to_hex(&self) -> String {
        hex_encode(&self.0)
    }

    /// True when `prefix` is a hex prefix of this key, so people can name a key by its
    /// first characters as Git does with commits.
    pub fn matches_prefix(&self, prefix: &str) -> bool {
        !prefix.is_empty() && self.to_hex().starts_with(&prefix.to_ascii_lowercase())
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({})", self.to_hex())
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl StaticKey {
    pub fn generate() -> io::Result<Self> {
        random().map(Self::from_private)
    }

    pub fn from_private(private: [u8; 32]) -> Self {
        let public = curve25519_dalek::MontgomeryPoint::mul_base_clamped(private).to_bytes();
        Self {
            private,
            public: PublicKey(public),
        }
    }

    pub fn public(&self) -> PublicKey {
        self.public
    }

    /// Reads the hex private key at `path`, or generates one and stores it owner-only.
    /// Two processes creating the same key at once both end up with the one that won.
    pub fn load_or_create(path: &Path) -> io::Result<Self> {
        if let Some(text) = read_secret_file(path)? {
            return hex_decode(text.trim()).map(Self::from_private);
        }
        let key = Self::generate()?;
        match write_secret_file(path, &hex_encode(&key.private)) {
            Ok(()) => Ok(key),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let text = read_secret_file(path)?.ok_or(error)?;
                hex_decode(text.trim()).map(Self::from_private)
            }
            Err(error) => Err(error),
        }
    }
}

impl fmt::Debug for StaticKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StaticKey({}, private redacted)", self.public)
    }
}

impl Secret {
    pub fn generate() -> io::Result<Self> {
        random().map(Self)
    }

    pub fn parse(hex: &str) -> io::Result<Self> {
        hex_decode(hex).map(Self)
    }

    pub fn to_hex(&self) -> String {
        hex_encode(&self.0)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(redacted)")
    }
}

/// The Server host's static key plus the devices it accepts.
///
/// With a store directory the authorized list and the pending invite live in files that
/// `condr server invite|clients|revoke` edit on the same host; without one only
/// `always_authorized` peers can connect.
pub struct ServerIdentity {
    key: StaticKey,
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
    /// The identity stored in `directory`: `server-key`, plus the paired devices and the
    /// pending invite kept beside it. Processes on the host itself use the local socket,
    /// so no device is authorized by default.
    pub fn load_or_create(directory: &Path) -> io::Result<Self> {
        Ok(Self {
            key: StaticKey::load_or_create(&directory.join(SERVER_KEY_FILE))?,
            store: Some(directory.to_path_buf()),
            always_authorized: Vec::new(),
        })
    }

    /// A fresh key with no store, for tests and short-lived Servers.
    pub fn ephemeral() -> io::Result<Self> {
        Ok(Self {
            key: StaticKey::generate()?,
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
    fn psk_for(&self, remote: &PublicKey) -> io::Result<Option<(Secret, Option<Secret>)>> {
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
    fn complete_pairing(&self, remote: PublicKey, name: &str, invite: &Secret) -> io::Result<()> {
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

/// Where this host keeps `server-key`, `client-key`, the authorized list and the invite.
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

/// The device key a GUI keeps beside its `config.toml` and presents to TCP Servers.
pub fn load_device_key(directory: &Path) -> io::Result<StaticKey> {
    StaticKey::load_or_create(&directory.join(CLIENT_KEY_FILE))
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
fn read_secret_file(path: &Path) -> io::Result<Option<String>> {
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
fn write_secret_file(path: &Path, text: &str) -> io::Result<()> {
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

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn random() -> io::Result<[u8; 32]> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(bytes)
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_decode(hex: &str) -> io::Result<[u8; 32]> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "expected 64 hex characters");
    if hex.len() != 64 || !hex.is_ascii() {
        return Err(invalid());
    }
    let mut bytes = [0; 32];
    for (byte, pair) in bytes.iter_mut().zip(hex.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).map_err(|_| invalid())?, 16)
            .map_err(|_| invalid())?;
    }
    Ok(bytes)
}

/// A TCP connection carrying Noise records. Clones share one transport state, so one
/// clone may read while another writes, as the framed protocol does.
pub struct NoiseStream {
    socket: TcpStream,
    noise: Arc<Mutex<Noise>>,
    /// Decrypted bytes not yet handed to the reader.
    plaintext: Vec<u8>,
    consumed: usize,
    /// Written bytes not yet encrypted; `flush` sends them as one record.
    pending: Vec<u8>,
    ciphertext: Vec<u8>,
}

enum Noise {
    Initiator {
        handshake: Box<HandshakeState>,
        server_key: PublicKey,
    },
    Responder {
        handshake: Box<HandshakeState>,
        identity: Arc<ServerIdentity>,
    },
    Transport {
        transport: Box<TransportState>,
        /// The peer's static key: the Server's on the Client side, the device's on the
        /// Server side.
        remote: PublicKey,
        /// Set on the Server side, so a connection knows whether its peer is the host.
        identity: Option<Arc<ServerIdentity>>,
        /// The invite the peer is redeeming, until the pairing is recorded.
        pairing: Option<Secret>,
        /// Whether a transport message from the peer has decrypted yet.
        verified: bool,
    },
    Failed,
}

impl NoiseStream {
    /// The Client side: `client_key` connects to the Server whose static key is
    /// `server_key`, presenting `invite` when this device is not paired yet.
    pub fn initiator(
        socket: TcpStream,
        server_key: &PublicKey,
        client_key: &StaticKey,
        invite: Option<&Secret>,
    ) -> io::Result<Self> {
        let psk = invite.map_or([0; 32], |invite| invite.0);
        let handshake = builder()?
            .local_private_key(&client_key.private)
            .and_then(|builder| builder.remote_public_key(&server_key.0))
            .and_then(|builder| builder.psk(PSK_LOCATION as u8, &psk))
            .and_then(|builder| builder.build_initiator())
            .map_err(noise_error)?;
        Ok(Self::new(
            socket,
            Noise::Initiator {
                handshake: Box::new(handshake),
                server_key: *server_key,
            },
        ))
    }

    /// The Server side; the handshake runs on first use, not on accept.
    pub fn responder(socket: TcpStream, identity: Arc<ServerIdentity>) -> io::Result<Self> {
        let handshake = builder()?
            .local_private_key(&identity.key.private)
            .and_then(|builder| builder.build_responder())
            .map_err(noise_error)?;
        Ok(Self::new(
            socket,
            Noise::Responder {
                handshake: Box::new(handshake),
                identity,
            },
        ))
    }

    fn new(socket: TcpStream, noise: Noise) -> Self {
        Self {
            socket,
            noise: Arc::new(Mutex::new(noise)),
            plaintext: Vec::new(),
            consumed: 0,
            pending: Vec::new(),
            ciphertext: Vec::new(),
        }
    }

    pub fn socket(&self) -> &TcpStream {
        &self.socket
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            socket: self.socket.try_clone()?,
            noise: Arc::clone(&self.noise),
            plaintext: Vec::new(),
            consumed: 0,
            pending: Vec::new(),
            ciphertext: Vec::new(),
        })
    }

    /// Records the peer as an authorized device if it connected with an invite. Call once
    /// the peer's first message has been read, which is what proves it held the invite.
    pub fn complete_pairing(&self, name: &str) -> io::Result<()> {
        let noise = Arc::clone(&self.noise);
        let mut noise = lock(&noise);
        let Noise::Transport {
            remote,
            identity,
            pairing,
            verified,
            ..
        } = &mut *noise
        else {
            return Err(rejected("handshake incomplete"));
        };
        match (pairing.as_ref(), *verified, identity.as_ref()) {
            (None, _, _) => Ok(()),
            (Some(_), false, _) => Err(rejected("invite not yet proven")),
            (Some(_), true, None) => Err(rejected("no Server identity to record the device")),
            (Some(invite), true, Some(identity)) => {
                identity.complete_pairing(*remote, name, invite)?;
                *pairing = None;
                Ok(())
            }
        }
    }

    /// Server side: whether the store still authorizes the peer. Read again after the
    /// handshake so a device revoked meanwhile is refused before it is served.
    pub fn peer_authorized(&self) -> io::Result<bool> {
        match &*lock(&self.noise) {
            Noise::Transport {
                remote,
                identity: Some(identity),
                ..
            } => identity.is_authorized(remote),
            Noise::Transport { identity: None, .. } => Ok(true),
            _ => Ok(false),
        }
    }

    /// Server side: records that the peer connected now.
    pub fn record_seen(&self) -> io::Result<()> {
        match &*lock(&self.noise) {
            Noise::Transport {
                remote,
                identity: Some(identity),
                ..
            } => identity.record_seen(remote),
            _ => Ok(()),
        }
    }

    /// Runs the handshake now instead of on first use; tests use it to hold a
    /// handshaken connection without sending anything.
    #[cfg(test)]
    pub(crate) fn handshake(&mut self) -> io::Result<()> {
        self.ensure_transport()
    }

    /// The peer's static key once the handshake has run.
    pub fn remote_public_key(&self) -> Option<PublicKey> {
        match &*lock(&self.noise) {
            Noise::Transport { remote, .. } => Some(*remote),
            _ => None,
        }
    }

    /// Closes the connection for every clone; the reader sees end of stream.
    pub fn shutdown(&self) -> io::Result<()> {
        self.socket.shutdown(std::net::Shutdown::Both)
    }

    /// Runs the handshake if it has not happened yet.
    fn ensure_transport(&mut self) -> io::Result<()> {
        let noise = Arc::clone(&self.noise);
        let mut noise = lock(&noise);
        match &*noise {
            Noise::Transport { .. } => return Ok(()),
            Noise::Failed => return Err(rejected("handshake failed earlier")),
            Noise::Initiator { .. } | Noise::Responder { .. } => {}
        }
        let mut buffer = vec![0; MAX_RECORD];
        let result = match std::mem::replace(&mut *noise, Noise::Failed) {
            Noise::Initiator {
                mut handshake,
                server_key,
            } => {
                write_handshake(&mut self.socket, &mut handshake, &mut buffer)?;
                if !read_handshake(&mut self.socket, &mut handshake, &mut buffer)? {
                    return Err(rejected(
                        "the Server closed the handshake: this device is not authorized \
                         (pair it with `condr server invite`) or the Server key differs",
                    ));
                }
                handshake
                    .into_transport_mode()
                    .map(|transport| Noise::Transport {
                        transport: Box::new(transport),
                        remote: server_key,
                        identity: None,
                        pairing: None,
                        verified: true,
                    })
            }
            Noise::Responder {
                mut handshake,
                identity,
            } => {
                if !read_handshake(&mut self.socket, &mut handshake, &mut buffer)? {
                    return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
                }
                let remote = handshake
                    .get_remote_static()
                    .and_then(|key| <[u8; 32]>::try_from(key).ok())
                    .map(PublicKey)
                    .ok_or_else(|| rejected("peer sent no static key"))?;
                let Some((psk, pairing)) = identity.psk_for(&remote)? else {
                    return Err(rejected(format!("unknown device {remote}")));
                };
                handshake
                    .set_psk(PSK_LOCATION, &psk.0)
                    .map_err(noise_error)?;
                write_handshake(&mut self.socket, &mut handshake, &mut buffer)?;
                handshake
                    .into_transport_mode()
                    .map(|transport| Noise::Transport {
                        transport: Box::new(transport),
                        remote,
                        identity: Some(identity),
                        pairing,
                        verified: false,
                    })
            }
            Noise::Transport { .. } | Noise::Failed => unreachable!(),
        };
        *noise = result.map_err(noise_error)?;
        Ok(())
    }

    fn send_record(&mut self, plaintext: &[u8]) -> io::Result<()> {
        self.ensure_transport()?;
        self.ciphertext.resize(2 + plaintext.len() + TAG_LEN, 0);
        let written = {
            let noise = Arc::clone(&self.noise);
            let mut noise = lock(&noise);
            let Noise::Transport { transport, .. } = &mut *noise else {
                return Err(rejected("handshake incomplete"));
            };
            transport
                .write_message(plaintext, &mut self.ciphertext[2..])
                .map_err(noise_error)?
        };
        self.ciphertext[..2].copy_from_slice(&(written as u16).to_be_bytes());
        self.socket.write_all(&self.ciphertext[..2 + written])
    }

    fn send_pending(&mut self) -> io::Result<()> {
        let mut pending = std::mem::take(&mut self.pending);
        let result = self.send_record(&pending);
        pending.clear();
        self.pending = pending;
        result
    }
}

impl Read for NoiseStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        while self.consumed == self.plaintext.len() {
            self.ensure_transport()?;
            let mut record = Vec::new();
            if !read_record(&mut self.socket, &mut record)? {
                return Ok(0);
            }
            self.plaintext.resize(record.len(), 0);
            let length = {
                let noise = Arc::clone(&self.noise);
                let mut noise = lock(&noise);
                let Noise::Transport {
                    transport,
                    verified,
                    ..
                } = &mut *noise
                else {
                    return Err(rejected("handshake incomplete"));
                };
                let length = transport
                    .read_message(&record, &mut self.plaintext)
                    .map_err(|error| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("undecryptable record: {error}"),
                        )
                    })?;
                *verified = true;
                length
            };
            self.plaintext.truncate(length);
            self.consumed = 0;
        }
        let available = &self.plaintext[self.consumed..];
        let length = available.len().min(buffer.len());
        buffer[..length].copy_from_slice(&available[..length]);
        self.consumed += length;
        Ok(length)
    }
}

impl Write for NoiseStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let mut remaining = buffer;
        if !self.pending.is_empty() {
            let count = remaining.len().min(MAX_PLAINTEXT - self.pending.len());
            self.pending.extend_from_slice(&remaining[..count]);
            remaining = &remaining[count..];
            if self.pending.len() == MAX_PLAINTEXT {
                self.send_pending()?;
            }
        }
        let (records, remainder) = remaining.as_chunks::<MAX_PLAINTEXT>();
        for record in records {
            self.send_record(record)?;
        }
        self.pending.extend_from_slice(remainder);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.pending.is_empty() {
            self.send_pending()?;
        }
        self.socket.flush()
    }
}

fn lock(noise: &Mutex<Noise>) -> std::sync::MutexGuard<'_, Noise> {
    noise
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn builder() -> io::Result<snow::Builder<'static>> {
    NOISE_PATTERN
        .parse()
        .map(snow::Builder::new)
        .map_err(noise_error)
}

fn write_handshake(
    socket: &mut TcpStream,
    handshake: &mut HandshakeState,
    buffer: &mut [u8],
) -> io::Result<()> {
    let length = handshake
        .write_message(&[], &mut buffer[2..])
        .map_err(noise_error)?;
    buffer[..2].copy_from_slice(&(length as u16).to_be_bytes());
    socket.write_all(&buffer[..2 + length])?;
    socket.flush()
}

/// False when the peer closed the connection instead of continuing the handshake.
fn read_handshake(
    socket: &mut TcpStream,
    handshake: &mut HandshakeState,
    buffer: &mut [u8],
) -> io::Result<bool> {
    let mut record = Vec::new();
    if !read_record(socket, &mut record)? {
        return Ok(false);
    }
    handshake
        .read_message(&record, buffer)
        .map_err(|error| rejected(format!("handshake rejected: {error}")))?;
    Ok(true)
}

/// Reads one record into `record`; false on a clean end of stream before any byte.
fn read_record(socket: &mut TcpStream, record: &mut Vec<u8>) -> io::Result<bool> {
    let mut prefix = [0; 2];
    match socket.read(&mut prefix[..1])? {
        0 => return Ok(false),
        _ => socket.read_exact(&mut prefix[1..])?,
    }
    let length = usize::from(u16::from_be_bytes(prefix));
    if length < TAG_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Noise record shorter than its authentication tag",
        ));
    }
    record.resize(length, 0);
    socket.read_exact(record)?;
    Ok(true)
}

fn noise_error(error: snow::Error) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, format!("Noise: {error}"))
}

fn rejected(reason: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, reason.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    fn pair(
        identity: &Arc<ServerIdentity>,
        client: &StaticKey,
        invite: Option<&Secret>,
    ) -> (NoiseStream, thread::JoinHandle<io::Result<NoiseStream>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let responder = Arc::clone(identity);
        let server = thread::spawn(move || {
            let (socket, _) = listener.accept()?;
            NoiseStream::responder(socket, responder)
        });
        let socket = TcpStream::connect(address).unwrap();
        let client =
            NoiseStream::initiator(socket, &identity.public_key(), client, invite).unwrap();
        (client, server)
    }

    fn identity() -> (Arc<ServerIdentity>, StaticKey) {
        let client = StaticKey::generate().unwrap();
        let identity = ServerIdentity::ephemeral()
            .unwrap()
            .with_authorized(client.public());
        (Arc::new(identity), client)
    }

    #[test]
    fn authorized_peer_round_trips_records_larger_than_one_noise_message() {
        let (identity, client) = identity();
        let (mut client, server) = pair(&identity, &client, None);
        let payload = (0..2_000_000_u32).map(|i| i as u8).collect::<Vec<_>>();
        let expected = payload.clone();
        let echo = thread::spawn(move || {
            let mut server = server.join().unwrap().unwrap();
            let mut received = vec![0; expected.len()];
            server.read_exact(&mut received).unwrap();
            assert_eq!(received, expected);
            server.write_all(b"ok").unwrap();
            server.flush().unwrap();
        });
        client.write_all(&payload[..7]).unwrap();
        client.write_all(&payload[7..1_500_000]).unwrap();
        for fragment in payload[1_500_000..].chunks(997) {
            client.write_all(fragment).unwrap();
        }
        assert!(client.pending.capacity() <= 2 * MAX_PLAINTEXT);
        client.flush().unwrap();
        let mut reply = [0; 2];
        client.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"ok");
        echo.join().unwrap();
    }

    #[test]
    fn revoke_requires_a_unique_prefix_without_changing_ambiguous_or_missing_keys() {
        let directory = std::env::temp_dir().join(format!(
            "condr-noise-revoke-{}-{}",
            std::process::id(),
            now()
        ));
        let first = PublicKey::parse(&format!("aa01{}", "00".repeat(30))).unwrap();
        let second = PublicKey::parse(&format!("aa02{}", "00".repeat(30))).unwrap();
        let clients = [first, second].map(|key| AuthorizedClient {
            key,
            paired_at: 1,
            last_seen: 2,
            name: "device".into(),
        });
        write_authorized(&directory, &clients).unwrap();
        let before = fs::read(directory.join(AUTHORIZED_FILE)).unwrap();
        assert_eq!(
            revoke(&directory, "AA").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(fs::read(directory.join(AUTHORIZED_FILE)).unwrap(), before);
        assert_eq!(revoke(&directory, "bb").unwrap(), None);
        assert_eq!(revoke(&directory, "").unwrap(), None);
        assert_eq!(fs::read(directory.join(AUTHORIZED_FILE)).unwrap(), before);
        assert_eq!(revoke(&directory, "AA01").unwrap(), Some(first));
        assert_eq!(
            read_authorized(&directory).unwrap(),
            vec![clients[1].clone()]
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unknown_peer_without_an_invite_is_refused_before_any_payload() {
        let (identity, _) = identity();
        let stranger = StaticKey::generate().unwrap();
        let (mut client, server) = pair(&identity, &stranger, None);
        // The handshake runs on first use, so the Server side reads on its own thread.
        let server = serve(server, |server| server.read(&mut [0; 16]).map(drop));
        let error = client
            .write_all(b"hello")
            .and_then(|()| client.flush())
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        let error = server.join().unwrap().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    /// Runs `f` on the Server side of a pair once its connection is accepted.
    fn serve<T: Send + 'static>(
        server: thread::JoinHandle<io::Result<NoiseStream>>,
        f: impl FnOnce(&mut NoiseStream) -> io::Result<T> + Send + 'static,
    ) -> thread::JoinHandle<io::Result<T>> {
        thread::spawn(move || f(&mut server.join().unwrap()?))
    }

    #[test]
    fn wrong_server_key_fails_the_client_handshake() {
        let (identity, client) = identity();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            let mut server = NoiseStream::responder(socket, identity).unwrap();
            server.read(&mut [0; 16])
        });
        let impostor = StaticKey::generate().unwrap();
        let mut stream = NoiseStream::initiator(
            TcpStream::connect(address).unwrap(),
            &impostor.public(),
            &client,
            None,
        )
        .unwrap();
        let error = stream
            .write_all(b"hello")
            .and_then(|()| stream.flush())
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(server.join().unwrap().is_err());
    }

    #[test]
    fn an_invite_pairs_a_new_device_once_and_a_bad_invite_does_not() {
        let directory = std::env::temp_dir().join(format!(
            "condr-noise-pairing-{}-{}",
            std::process::id(),
            now()
        ));
        let identity = Arc::new(ServerIdentity::load_or_create(&directory).unwrap());
        let device = StaticKey::generate().unwrap();

        let invite = create_invite(&directory).unwrap();
        let wrong = Secret::generate().unwrap();
        let (mut client, server) = pair(&identity, &device, Some(&wrong));
        let server = serve(server, |server| {
            // The Client cannot decrypt the second message and hangs up; the Server saw no
            // transport record, so the invite is not proven and nothing is recorded.
            let read = server.read(&mut [0; 16]).unwrap_or(0);
            let paired = server.complete_pairing("laptop");
            Ok((read, paired.is_err()))
        });
        let error = client
            .write_all(b"hello")
            .and_then(|()| client.flush())
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        drop(client);
        assert_eq!(server.join().unwrap().unwrap(), (0, true));
        assert!(read_authorized(&directory).unwrap().is_empty());

        let (mut client, server) = pair(&identity, &device, Some(&invite.secret));
        let server = serve(server, |server| {
            let mut received = [0; 5];
            server.read_exact(&mut received)?;
            assert_eq!(&received, b"hello");
            server.complete_pairing("laptop")
        });
        client.write_all(b"hello").unwrap();
        client.flush().unwrap();
        server.join().unwrap().unwrap();
        let clients = read_authorized(&directory).unwrap();
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].key, device.public());
        assert_eq!(clients[0].name, "laptop");
        assert!(
            read_invite(&directory).unwrap().is_none(),
            "invite is one-time"
        );

        // A second device that finished its handshake with the same invite loses the
        // race: the invite is gone by the time it proves itself.
        let invite = create_invite(&directory).unwrap();
        let first = StaticKey::generate().unwrap();
        let second = StaticKey::generate().unwrap();
        let (mut first_client, first_server) = pair(&identity, &first, Some(&invite.secret));
        let (mut second_client, second_server) = pair(&identity, &second, Some(&invite.secret));
        let first_server = serve(first_server, |server| {
            server.read_exact(&mut [0; 2])?;
            server.complete_pairing("first")
        });
        let second_server = serve(second_server, |server| {
            server.read_exact(&mut [0; 2])?;
            server.complete_pairing("second")
        });
        // Both handshakes complete while the invite is still pending; only the first
        // device to prove itself gets recorded.
        first_client.handshake().unwrap();
        second_client.handshake().unwrap();
        first_client.write_all(b"hi").unwrap();
        first_client.flush().unwrap();
        first_server.join().unwrap().unwrap();
        second_client.write_all(b"hi").unwrap();
        second_client.flush().unwrap();
        assert_eq!(
            second_server.join().unwrap().unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        let paired = read_authorized(&directory).unwrap();
        assert!(paired.iter().any(|client| client.key == first.public()));
        assert!(!paired.iter().any(|client| client.key == second.public()));

        // A handshake made with an older invite cannot pair after a newer invite replaced
        // it, and does not consume the newer one.
        let stale = create_invite(&directory).unwrap();
        let late = StaticKey::generate().unwrap();
        let (mut late_client, late_server) = pair(&identity, &late, Some(&stale.secret));
        let late_server = serve(late_server, |server| {
            server.read_exact(&mut [0; 2])?;
            server.complete_pairing("late")
        });
        late_client.handshake().unwrap();
        let fresh = create_invite(&directory).unwrap();
        late_client.write_all(b"hi").unwrap();
        late_client.flush().unwrap();
        assert_eq!(
            late_server.join().unwrap().unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(read_invite(&directory).unwrap(), Some(fresh));
        let _ = fs::remove_file(directory.join(INVITE_FILE));

        // Paired: connects with the zero PSK, no invite needed.
        let (mut client, server) = pair(&identity, &device, None);
        let server = serve(server, |server| {
            let mut received = [0; 5];
            server.read_exact(&mut received)?;
            Ok(received)
        });
        client.write_all(b"again").unwrap();
        client.flush().unwrap();
        assert_eq!(&server.join().unwrap().unwrap(), b"again");

        assert_eq!(
            revoke(&directory, &device.public().to_hex()[..8]).unwrap(),
            Some(device.public())
        );
        let (mut client, server) = pair(&identity, &device, None);
        let server = serve(server, |server| server.read(&mut [0; 16]).map(drop));
        assert!(
            client
                .write_all(b"x")
                .and_then(|()| client.flush())
                .is_err()
        );
        assert!(server.join().unwrap().is_err());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn authorized_clients_keep_last_seen_and_reject_legacy_lines() {
        let directory =
            std::env::temp_dir().join(format!("condr-noise-seen-{}-{}", std::process::id(), now()));
        fs::create_dir_all(&directory).unwrap();
        let old = StaticKey::generate().unwrap().public();
        let new = StaticKey::generate().unwrap().public();
        fs::write(
            directory.join(AUTHORIZED_FILE),
            format!("{old} 100 old laptop\n"),
        )
        .unwrap();
        assert!(read_authorized(&directory).is_err());
        fs::write(
            directory.join(AUTHORIZED_FILE),
            format!("{old} 100 100 old laptop\n{new} 200 300 new laptop\n"),
        )
        .unwrap();

        let clients = read_authorized(&directory).unwrap();
        assert_eq!((clients[0].paired_at, clients[0].last_seen), (100, 100));
        assert_eq!(clients[0].name, "old laptop");
        assert_eq!((clients[1].paired_at, clients[1].last_seen), (200, 300));
        assert_eq!(clients[1].name, "new laptop");

        let identity = ServerIdentity::load_or_create(&directory).unwrap();
        identity.record_seen(&old).unwrap();
        let clients = read_authorized(&directory).unwrap();
        assert!(clients[0].last_seen >= now() - 5);
        assert_eq!(clients[0].name, "old laptop", "rewriting keeps the name");
        assert_eq!(clients[1].last_seen, 300, "other devices are untouched");
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn keys_and_secrets_round_trip_through_hex_and_files() {
        let key = StaticKey::generate().unwrap();
        assert_eq!(
            PublicKey::parse(&key.public().to_hex()).unwrap(),
            key.public()
        );
        assert!(PublicKey::parse("abc").is_err());
        assert!(key.public().matches_prefix(&key.public().to_hex()[..6]));
        assert!(!key.public().matches_prefix(""));

        let path =
            std::env::temp_dir().join(format!("condr-noise-key-{}-{}", std::process::id(), now()));
        let stored = StaticKey::load_or_create(&path).unwrap();
        assert_eq!(StaticKey::load_or_create(&path).unwrap(), stored);
        assert!(!format!("{stored:?}").contains(&hex_encode(&stored.private)));
        let _ = fs::remove_file(path);
    }
}
