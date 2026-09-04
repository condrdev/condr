use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(windows)]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[cfg(windows)]
use atomicwrites::{AtomicFile, DisallowOverwrite};
use interprocess::local_socket::traits::Listener as _;

use crate::noise::{NoiseStream, PublicKey, Secret, ServerIdentity, StaticKey};

pub type LocalListener = interprocess::local_socket::Listener;
pub type LocalStream = interprocess::local_socket::Stream;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Endpoint {
    Local(PathBuf),
    Tcp(TcpEndpoint),
}

/// Everything a Client needs to reach one TCP Server: where it listens, whose static key
/// it must present, which device key to speak with, and the invite that pairs a device
/// the Server does not know yet. The Server binds with the same value, so its own
/// `server_key` and `client_key` are the host identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TcpEndpoint {
    pub address: SocketAddr,
    pub server_key: PublicKey,
    pub client_key: StaticKey,
    pub invite: Option<Secret>,
}

pub enum EndpointListener {
    Local(LocalEndpointListener),
    Tcp {
        listener: TcpListener,
        identity: Arc<ServerIdentity>,
    },
}

pub struct LocalEndpointListener {
    listener: LocalListener,
    path: PathBuf,
    _bind_lock: File,
    ownership: LocalEndpointOwnership,
}

#[derive(Debug, Eq, PartialEq)]
enum LocalEndpointOwnership {
    #[cfg(unix)]
    Unix { device: u64, inode: u64 },
    #[cfg(windows)]
    Windows { marker: Vec<u8> },
}

pub enum EndpointStream {
    Local(LocalStream),
    Tcp(NoiseStream),
}

impl TcpEndpoint {
    /// Parses `<server key>[.<invite>]@host:port`, the text an invite hands to a person.
    pub fn parse(text: &str, client_key: StaticKey) -> io::Result<Self> {
        let invalid = |reason: &str| io::Error::new(io::ErrorKind::InvalidInput, reason.to_owned());
        let (credentials, address) = text
            .trim()
            .rsplit_once('@')
            .ok_or_else(|| invalid("expected <server key>[.<invite>]@host:port"))?;
        let address = address
            .parse::<SocketAddr>()
            .map_err(|_| invalid("invalid host:port"))?;
        let (server_key, invite) = match credentials.split_once('.') {
            Some((key, invite)) => (key, Some(Secret::parse(invite)?)),
            None => (credentials, None),
        };
        Ok(Self {
            address,
            server_key: PublicKey::parse(server_key)?,
            client_key,
            invite,
        })
    }

    /// The text a paired Client stores or a Pane program reads: `<server key>@host:port`.
    pub fn locator(&self) -> String {
        format!("{}@{}", self.server_key, self.address)
    }

    pub fn with_address(mut self, address: SocketAddr) -> Self {
        self.address = address;
        self
    }

    pub fn without_invite(mut self) -> Self {
        self.invite = None;
        self
    }
}

impl Endpoint {
    pub fn local(path: impl Into<PathBuf>) -> Self {
        Self::Local(path.into())
    }

    pub fn tcp(endpoint: TcpEndpoint) -> Self {
        Self::Tcp(endpoint)
    }

    /// Binds the listener. A TCP Server answers with `identity`, whose public key must be
    /// the one the endpoint advertises.
    pub fn bind(&self, identity: Option<Arc<ServerIdentity>>) -> io::Result<EndpointListener> {
        match self {
            Self::Local(path) => bind_local(path).map(EndpointListener::Local),
            Self::Tcp(tcp) => {
                let identity = identity.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "a TCP endpoint needs a Server identity",
                    )
                })?;
                if identity.public_key() != tcp.server_key {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "the TCP endpoint advertises a different Server key than the identity",
                    ));
                }
                Ok(EndpointListener::Tcp {
                    listener: TcpListener::bind(tcp.address)?,
                    identity,
                })
            }
        }
    }

    pub fn connect(&self) -> io::Result<EndpointStream> {
        match self {
            Self::Local(path) => connect_local(path).map(EndpointStream::Local),
            Self::Tcp(tcp) => NoiseStream::initiator(
                TcpStream::connect(tcp.address)?,
                &tcp.server_key,
                &tcp.client_key,
                tcp.invite.as_ref(),
            )
            .map(EndpointStream::Tcp),
        }
    }

    /// The value a Pane's `CONDR_SOCKET_PATH` carries: the socket or pipe path, or
    /// `tcp://<server key>@host:port` for a TCP Server.
    pub fn env_value(&self) -> String {
        match self {
            Self::Local(path) => path.to_string_lossy().into_owned(),
            Self::Tcp(tcp) => format!("tcp://{}", tcp.locator()),
        }
    }

    /// The inverse of [`Self::env_value`]: how a Pane program finds its own Server. A TCP
    /// locator needs `client_key`, the host device key the Server always accepts.
    pub fn from_env_value(
        value: &str,
        client_key: impl FnOnce() -> io::Result<StaticKey>,
    ) -> io::Result<Self> {
        match value.strip_prefix("tcp://") {
            Some(locator) => TcpEndpoint::parse(locator, client_key()?).map(Self::Tcp),
            None if value.is_empty() => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "empty endpoint",
            )),
            None => Ok(Self::local(value)),
        }
    }

    pub fn as_local_path(&self) -> Option<&Path> {
        match self {
            Self::Local(path) => Some(path),
            Self::Tcp(_) => None,
        }
    }

    pub fn tcp_address(&self) -> Option<SocketAddr> {
        match self {
            Self::Local(_) => None,
            Self::Tcp(tcp) => Some(tcp.address),
        }
    }
}

/// The endpoint without any key material: a path, or `tcp://host:port`.
impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local(path) => write!(f, "{}", path.display()),
            Self::Tcp(tcp) => write!(f, "tcp://{}", tcp.address),
        }
    }
}

impl EndpointListener {
    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        match self {
            Self::Local(local) => {
                use interprocess::local_socket::ListenerNonblockingMode;

                local.listener.set_nonblocking(if nonblocking {
                    ListenerNonblockingMode::Accept
                } else {
                    ListenerNonblockingMode::Both
                })
            }
            Self::Tcp { listener, .. } => listener.set_nonblocking(nonblocking),
        }
    }

    /// Accepts one connection. The TCP handshake runs on the stream's first use, on the
    /// client's own thread, so a slow peer cannot stall accepting.
    pub fn accept(&self) -> io::Result<EndpointStream> {
        match self {
            Self::Local(local) => local.listener.accept().map(EndpointStream::Local),
            Self::Tcp { listener, identity } => {
                let (stream, _) = listener.accept()?;
                // Accepted sockets inherit nonblocking mode on Windows. Client handlers use
                // blocking framed reads, so normalize the stream on every platform.
                stream.set_nonblocking(false)?;
                NoiseStream::responder(stream, Arc::clone(identity)).map(EndpointStream::Tcp)
            }
        }
    }

    pub fn local_addr(&self) -> io::Result<Option<SocketAddr>> {
        match self {
            Self::Local(_) => Ok(None),
            Self::Tcp { listener, .. } => listener.local_addr().map(Some),
        }
    }

    pub fn cleanup(&self) -> io::Result<()> {
        match self {
            Self::Local(local) => local.cleanup(),
            Self::Tcp { .. } => Ok(()),
        }
    }
}

impl LocalEndpointListener {
    fn cleanup(&self) -> io::Result<()> {
        remove_local_path_if_owned(&self.path, &self.ownership).map(drop)
    }
}

impl Drop for LocalEndpointListener {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

impl std::io::Read for EndpointStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Local(stream) => stream.read(buffer),
            Self::Tcp(stream) => stream.read(buffer),
        }
    }
}

impl std::io::Write for EndpointStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Local(stream) => stream.write(buffer),
            Self::Tcp(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Local(stream) => stream.flush(),
            Self::Tcp(stream) => stream.flush(),
        }
    }
}

impl EndpointStream {
    pub fn try_clone(&self) -> io::Result<Self> {
        match self {
            Self::Local(stream) => {
                use interprocess::TryClone as _;
                stream.try_clone().map(Self::Local)
            }
            Self::Tcp(stream) => stream.try_clone().map(Self::Tcp),
        }
    }

    /// Records a TCP peer that connected with an invite as an authorized device; a no-op
    /// for local and already-paired peers. Call after its first message was read.
    pub fn complete_pairing(&self, client_name: &str) -> io::Result<()> {
        match self {
            Self::Local(_) => Ok(()),
            Self::Tcp(stream) => stream.complete_pairing(client_name),
        }
    }

    /// The static key of a TCP peer; `None` for local connections.
    pub fn peer_key(&self) -> Option<PublicKey> {
        match self {
            Self::Local(_) => None,
            Self::Tcp(stream) => stream.remote_public_key(),
        }
    }

    /// Whether the store still authorizes a TCP peer; local peers always are. Checked
    /// again after the handshake so a device revoked meanwhile never gets served.
    pub fn peer_authorized(&self) -> io::Result<bool> {
        match self {
            Self::Local(_) => Ok(true),
            Self::Tcp(stream) => stream.peer_authorized(),
        }
    }

    /// Whether the peer may administer the Server: every local connection, and a TCP
    /// peer speaking with the Server host's own device key.
    pub fn may_administer(&self) -> bool {
        match self {
            Self::Local(_) => true,
            Self::Tcp(stream) => stream.peer_is_host(),
        }
    }

    /// Closes a TCP connection for all of its clones; local streams close on drop.
    pub fn shutdown(&self) -> io::Result<()> {
        match self {
            Self::Local(_) => Ok(()),
            Self::Tcp(stream) => stream.shutdown(),
        }
    }

    pub fn set_handshake_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            Self::Local(stream) => {
                use interprocess::local_socket::traits::Stream as _;
                match stream.set_recv_timeout(timeout) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == io::ErrorKind::Unsupported => Ok(()),
                    Err(error) => Err(error),
                }
            }
            Self::Tcp(stream) => stream.socket().set_read_timeout(timeout),
        }
    }
}

pub fn default_socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("CONDR_SOCKET_PATH") {
        return PathBuf::from(path);
    }

    platform_socket_path()
        .expect("no platform runtime directory is available; set CONDR_SOCKET_PATH")
}

fn platform_socket_path() -> Option<PathBuf> {
    condr_core::runtime_directory().map(|root| root.join("condr.sock"))
}

fn connect_local(path: &Path) -> io::Result<LocalStream> {
    #[cfg(unix)]
    {
        use interprocess::local_socket::{GenericFilePath, prelude::*};
        let name = path.to_fs_name::<GenericFilePath>()?;
        LocalStream::connect(name)
    }

    #[cfg(windows)]
    {
        use interprocess::local_socket::{GenericNamespaced, prelude::*};
        let name = path.to_string_lossy().to_string();
        let name = name.to_ns_name::<GenericNamespaced>()?;
        LocalStream::connect(name)
    }
}

fn bind_local(path: &Path) -> io::Result<LocalEndpointListener> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bind_lock = acquire_local_bind_lock(path)?;
    prepare_local_path(path)?;

    #[cfg(unix)]
    {
        use interprocess::local_socket::{GenericFilePath, ListenerOptions, prelude::*};
        let name = path.to_fs_name::<GenericFilePath>()?;
        let listener = ListenerOptions::new()
            .name(name)
            .reclaim_name(false)
            .create_sync()?;
        let (device, inode) = unix_socket_identity(path)?;
        let ownership = LocalEndpointOwnership::Unix { device, inode };
        if let Err(error) = restrict_permissions(path) {
            drop(listener);
            let _ = remove_local_path_if_owned(path, &ownership);
            return Err(error);
        }
        Ok(LocalEndpointListener {
            listener,
            path: path.to_path_buf(),
            _bind_lock: bind_lock,
            ownership,
        })
    }

    #[cfg(windows)]
    {
        use interprocess::local_socket::{GenericNamespaced, ListenerOptions, prelude::*};
        use interprocess::os::windows::{
            local_socket::ListenerOptionsExt as _, security_descriptor::SecurityDescriptor,
        };

        let name = path.to_string_lossy().to_string();
        let name = name.to_ns_name::<GenericNamespaced>()?;
        // Protected DACL: generic-all access for the object owner and LocalSystem only.
        let sddl = widestring::U16CString::from_str("D:P(A;;GA;;;OW)(A;;GA;;;SY)")
            .map_err(io::Error::other)?;
        let security_descriptor = SecurityDescriptor::deserialize(&sddl)?;
        let listener = ListenerOptions::new()
            .name(name)
            .reclaim_name(false)
            .security_descriptor(security_descriptor)
            .create_sync()?;
        let marker = local_endpoint_marker();
        let ownership = LocalEndpointOwnership::Windows {
            marker: marker.clone(),
        };
        if let Err(error) = write_local_endpoint_marker(path, &marker) {
            drop(listener);
            let _ = remove_local_path_if_owned(path, &ownership);
            return Err(error);
        }
        Ok(LocalEndpointListener {
            listener,
            path: path.to_path_buf(),
            _bind_lock: bind_lock,
            ownership,
        })
    }
}

fn acquire_local_bind_lock(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let lock = options.open(local_bind_lock_path(path))?;
    match lock.try_lock() {
        Ok(()) => Ok(lock),
        Err(std::fs::TryLockError::WouldBlock) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!(
                "Condr server is already starting or running at {}",
                path.display()
            ),
        )),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}

fn local_bind_lock_path(path: &Path) -> PathBuf {
    let mut lock_path = path.as_os_str().to_os_string();
    lock_path.push(".bind.lock");
    PathBuf::from(lock_path)
}

fn prepare_local_path(path: &Path) -> io::Result<()> {
    let Some(ownership) = local_path_ownership_if_present(path)? else {
        return Ok(());
    };

    match connect_local(path) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("Condr server is already running at {}", path.display()),
        )),
        Err(error) if stale_local_endpoint_error(&error) => {
            if remove_local_path_if_owned(path, &ownership)? {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("local endpoint changed while probing {}", path.display()),
                ))
            }
        }
        Err(error) => Err(error),
    }
}

fn remove_local_path_if_owned(path: &Path, ownership: &LocalEndpointOwnership) -> io::Result<bool> {
    let Some(current) = local_path_ownership_if_present(path)? else {
        return Ok(true);
    };
    if current != *ownership {
        return Ok(false);
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn local_path_ownership_if_present(path: &Path) -> io::Result<Option<LocalEndpointOwnership>> {
    unix_socket_identity_if_present(path).map(|identity| {
        identity.map(|(device, inode)| LocalEndpointOwnership::Unix { device, inode })
    })
}

#[cfg(windows)]
fn local_path_ownership_if_present(path: &Path) -> io::Result<Option<LocalEndpointOwnership>> {
    let marker = match fs::read(path) {
        Ok(marker) => marker,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    validate_local_endpoint_marker(path, &marker)?;
    Ok(Some(LocalEndpointOwnership::Windows { marker }))
}

#[cfg(unix)]
fn stale_local_endpoint_error(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::ConnectionRefused
}

#[cfg(windows)]
fn stale_local_endpoint_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
    )
}

#[cfg(unix)]
fn unix_socket_identity(path: &Path) -> io::Result<(u64, u64)> {
    unix_socket_identity_if_present(path)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("local endpoint does not exist: {}", path.display()),
        )
    })
}

#[cfg(unix)]
fn unix_socket_identity_if_present(path: &Path) -> io::Result<Option<(u64, u64)>> {
    use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};

    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_socket() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "refusing to replace non-socket local endpoint path {}",
                path.display()
            ),
        ));
    }
    Ok(Some((metadata.dev(), metadata.ino())))
}

#[cfg(windows)]
fn local_endpoint_marker() -> Vec<u8> {
    static NEXT_MARKER: AtomicU64 = AtomicU64::new(1);
    format!(
        "condr-local-endpoint\n{}\n{}\n",
        std::process::id(),
        NEXT_MARKER.fetch_add(1, Ordering::Relaxed)
    )
    .into_bytes()
}

#[cfg(windows)]
fn write_local_endpoint_marker(path: &Path, marker: &[u8]) -> io::Result<()> {
    AtomicFile::new(path, DisallowOverwrite)
        .write(|file| {
            std::io::Write::write_all(file, marker)?;
            file.sync_all()
        })
        .map_err(Into::into)
}

#[cfg(windows)]
fn validate_local_endpoint_marker(path: &Path, marker: &[u8]) -> io::Result<()> {
    let text = std::str::from_utf8(marker).map_err(|_| invalid_local_endpoint_marker(path))?;
    let mut lines = text.lines();
    let valid = lines.next() == Some("condr-local-endpoint")
        && lines.next().is_some_and(|pid| pid.parse::<u32>().is_ok())
        && lines
            .next()
            .is_some_and(|token| token.parse::<u64>().is_ok())
        && lines.next().is_none();
    if valid {
        Ok(())
    } else {
        Err(invalid_local_endpoint_marker(path))
    }
}

#[cfg(windows)]
fn invalid_local_endpoint_marker(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "refusing to replace non-Condr local endpoint marker {}",
            path.display()
        ),
    )
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHILD_ENDPOINT_ENV: &str = "CONDR_ENDPOINT_BIND_CHILD_PATH";
    const CHILD_READY_ENV: &str = "CONDR_ENDPOINT_BIND_CHILD_READY";

    #[test]
    fn socket_is_in_the_platform_runtime_directory() {
        assert_eq!(
            platform_socket_path(),
            condr_core::runtime_directory().map(|root| root.join("condr.sock"))
        );
    }

    #[test]
    fn local_endpoint_bind_child_process() {
        let Some(path) = std::env::var_os(CHILD_ENDPOINT_ENV).map(PathBuf::from) else {
            return;
        };
        let ready = PathBuf::from(
            std::env::var_os(CHILD_READY_ENV).expect("bind child has a ready-file path"),
        );
        let _listener = Endpoint::local(path).bind(None).unwrap();
        fs::write(ready, b"ready").unwrap();
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    #[test]
    fn bind_lock_survives_a_crashed_owner_and_serializes_processes() {
        use std::process::{Child, Command, Stdio};

        struct ChildGuard(Option<Child>);

        impl ChildGuard {
            fn terminate(&mut self) {
                if let Some(mut child) = self.0.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        }

        impl Drop for ChildGuard {
            fn drop(&mut self) {
                self.terminate();
            }
        }

        let path = test_path("process-lock");
        let ready = path.with_extension("ready");
        cleanup_test_artifacts(&path);
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "endpoint::tests::local_endpoint_bind_child_process",
                "--nocapture",
            ])
            .env(CHILD_ENDPOINT_ENV, &path)
            .env(CHILD_READY_ENV, &ready)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut child = ChildGuard(Some(child));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !ready.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "bind child did not become ready"
            );
            if let Some(status) = child.0.as_mut().unwrap().try_wait().unwrap() {
                panic!("bind child exited before becoming ready: {status}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        let endpoint = Endpoint::local(&path);
        let error = match endpoint.bind(None) {
            Ok(_) => panic!("a second process unexpectedly acquired the bind lock"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        assert!(endpoint.connect().is_ok());
        assert!(local_bind_lock_path(&path).exists());

        child.terminate();
        assert!(path.exists(), "crashed owner should leave a stale endpoint");
        let replacement = endpoint.bind(None).unwrap();
        assert!(endpoint.connect().is_ok());
        assert!(local_bind_lock_path(&path).exists());

        drop(replacement);
        let _ = fs::remove_file(ready);
        cleanup_test_artifacts(&path);
    }

    #[test]
    fn local_endpoint_round_trips() {
        let path = test_path("round-trip");
        let endpoint = Endpoint::local(&path);
        let listener = endpoint.bind(None).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stream = endpoint.connect().unwrap();

        drop(stream);
        drop(listener);
        assert!(!path.exists());
        assert!(local_bind_lock_path(&path).exists());
        cleanup_test_artifacts(&path);
    }

    #[cfg(windows)]
    #[test]
    fn windows_marker_is_published_complete_without_overwriting() {
        let path = test_path("atomic-marker");
        cleanup_test_artifacts(&path);
        let marker = local_endpoint_marker();

        write_local_endpoint_marker(&path, &marker).unwrap();
        assert_eq!(fs::read(&path).unwrap(), marker);

        let replacement = local_endpoint_marker();
        let error = write_local_endpoint_marker(&path, &replacement).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&path).unwrap(), marker);
        cleanup_test_artifacts(&path);
    }

    #[cfg(windows)]
    #[test]
    fn stale_windows_marker_is_replaced_but_other_files_are_preserved() {
        let stale_path = test_path("stale-marker");
        fs::write(&stale_path, local_endpoint_marker()).unwrap();
        let stale_endpoint = Endpoint::local(&stale_path);
        let listener = stale_endpoint.bind(None).unwrap();
        assert!(stale_endpoint.connect().is_ok());
        drop(listener);
        cleanup_test_artifacts(&stale_path);

        let file_path = test_path("regular-file");
        fs::write(&file_path, b"keep me").unwrap();
        let file_endpoint = Endpoint::local(&file_path);
        let error = match file_endpoint.bind(None) {
            Ok(_) => panic!("a non-Condr marker was unexpectedly replaced"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(fs::read(&file_path).unwrap(), b"keep me");
        cleanup_test_artifacts(&file_path);
    }

    #[cfg(windows)]
    #[test]
    fn windows_bind_lock_rejects_a_second_live_owner() {
        let path = test_path("live-lock");
        let endpoint = Endpoint::local(&path);
        let listener = endpoint.bind(None).unwrap();
        let marker = fs::read(&path).unwrap();

        let error = match endpoint.bind(None) {
            Ok(_) => panic!("a second listener unexpectedly acquired the bind lock"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        assert_eq!(fs::read(&path).unwrap(), marker);
        assert!(endpoint.connect().is_ok());

        drop(listener);
        cleanup_test_artifacts(&path);
    }

    #[cfg(unix)]
    #[test]
    fn local_endpoint_is_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;

        let path = test_path("permissions");
        let endpoint = Endpoint::local(&path);
        let listener = endpoint.bind(None).unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let lock_mode = fs::metadata(local_bind_lock_path(&path))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(lock_mode, 0o600);

        drop(listener);
        cleanup_test_artifacts(&path);
    }

    #[cfg(unix)]
    #[test]
    fn stale_local_endpoint_is_replaced() {
        use std::os::unix::net::UnixListener;

        let path = test_path("stale");
        let endpoint = Endpoint::local(&path);
        let stale_listener = UnixListener::bind(&path).unwrap();
        drop(stale_listener);
        assert!(path.exists());

        let replacement = endpoint.bind(None).unwrap();
        assert!(path.exists());
        let stream = endpoint.connect().unwrap();

        drop(stream);
        drop(replacement);
        assert!(!path.exists());
        cleanup_test_artifacts(&path);
    }

    #[cfg(unix)]
    #[test]
    fn live_local_endpoint_is_not_removed() {
        use std::os::unix::fs::MetadataExt as _;
        use std::os::unix::net::UnixListener;

        let path = test_path("live");
        let endpoint = Endpoint::local(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let inode = fs::metadata(&path).unwrap().ino();

        let error = match endpoint.bind(None) {
            Ok(_) => panic!("a second listener unexpectedly replaced the live endpoint"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
        let stream = endpoint.connect().unwrap();

        drop(stream);
        drop(listener);
        assert!(path.exists());
        cleanup_test_artifacts(&path);
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_binders_cannot_both_replace_one_stale_endpoint() {
        use std::os::unix::net::UnixListener;
        use std::sync::mpsc;

        let path = test_path("concurrent-stale");
        let stale_listener = UnixListener::bind(&path).unwrap();
        drop(stale_listener);

        let endpoint = Endpoint::local(&path);
        let (sender, receiver) = mpsc::channel();
        let threads = (0..2)
            .map(|_| {
                let sender = sender.clone();
                let endpoint = endpoint.clone();
                std::thread::spawn(move || sender.send(endpoint.bind(None)).unwrap())
            })
            .collect::<Vec<_>>();
        drop(sender);

        let mut owner = None;
        for result in receiver {
            match result {
                Ok(listener) => assert!(owner.replace(listener).is_none()),
                Err(error) => assert_eq!(error.kind(), io::ErrorKind::AddrInUse),
            }
        }
        for thread in threads {
            thread.join().unwrap();
        }
        let owner = owner.expect("exactly one concurrent binder owns the endpoint");
        let stream = endpoint.connect().unwrap();

        drop(stream);
        drop(owner);
        cleanup_test_artifacts(&path);
    }

    #[cfg(unix)]
    #[test]
    fn old_listener_cleanup_preserves_an_external_replacement() {
        use std::os::unix::fs::MetadataExt as _;
        use std::os::unix::net::UnixListener;

        let path = test_path("ownership");
        let endpoint = Endpoint::local(&path);
        let original = endpoint.bind(None).unwrap();
        let original_inode = fs::metadata(&path).unwrap().ino();

        fs::remove_file(&path).unwrap();
        let replacement = UnixListener::bind(&path).unwrap();
        let replacement_inode = fs::metadata(&path).unwrap().ino();
        assert_ne!(replacement_inode, original_inode);

        original.cleanup().unwrap();
        drop(original);
        assert_eq!(fs::metadata(&path).unwrap().ino(), replacement_inode);
        let stream = endpoint.connect().unwrap();

        drop(stream);
        drop(replacement);
        cleanup_test_artifacts(&path);
    }

    #[cfg(unix)]
    #[test]
    fn non_socket_endpoint_path_is_never_replaced() {
        let path = test_path("regular-file");
        fs::write(&path, b"keep me").unwrap();
        let endpoint = Endpoint::local(&path);

        let error = match endpoint.bind(None) {
            Ok(_) => panic!("a regular file was unexpectedly replaced"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(fs::read(&path).unwrap(), b"keep me");
        cleanup_test_artifacts(&path);
    }

    fn test_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "condr-endpoint-{label}-{}-{}.sock",
            std::process::id(),
            unique_suffix()
        ))
    }

    fn cleanup_test_artifacts(path: &Path) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(local_bind_lock_path(path));
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    }
}
