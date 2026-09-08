//! Local socket ownership, permissions and endpoint listeners.

use super::*;

pub type LocalListener = interprocess::local_socket::Listener;

pub type LocalStream = interprocess::local_socket::Stream;

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

impl EndpointListener {
    /// The private local socket every Server answers on.
    pub fn local(path: &Path) -> io::Result<Self> {
        bind_local(path).map(Self::Local)
    }

    /// The optional TCP listener, answering with `identity`.
    pub fn tcp(address: SocketAddr, identity: Arc<ServerIdentity>) -> io::Result<Self> {
        Ok(Self::Tcp {
            listener: TcpListener::bind(address)?,
            identity,
        })
    }

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

pub(super) fn connect_local(path: &Path) -> io::Result<LocalStream> {
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

pub(crate) fn acquire_local_bind_lock(path: &Path) -> io::Result<File> {
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
        let _listener = EndpointListener::local(&path).unwrap();
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
                "endpoint::local::tests::local_endpoint_bind_child_process",
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
        let error = match EndpointListener::local(endpoint.as_local_path().unwrap()) {
            Ok(_) => panic!("a second process unexpectedly acquired the bind lock"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        assert!(endpoint.connect().is_ok());
        assert!(local_bind_lock_path(&path).exists());

        child.terminate();
        assert!(path.exists(), "crashed owner should leave a stale endpoint");
        let replacement = EndpointListener::local(endpoint.as_local_path().unwrap()).unwrap();
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
        let listener = EndpointListener::local(endpoint.as_local_path().unwrap()).unwrap();
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
        let listener = EndpointListener::local(&stale_path).unwrap();
        assert!(stale_endpoint.connect().is_ok());
        drop(listener);
        cleanup_test_artifacts(&stale_path);

        let file_path = test_path("regular-file");
        fs::write(&file_path, b"keep me").unwrap();
        let error = match EndpointListener::local(&file_path) {
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
        let listener = EndpointListener::local(endpoint.as_local_path().unwrap()).unwrap();
        let marker = fs::read(&path).unwrap();

        let error = match EndpointListener::local(endpoint.as_local_path().unwrap()) {
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
        let listener = EndpointListener::local(endpoint.as_local_path().unwrap()).unwrap();

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

        let replacement = EndpointListener::local(endpoint.as_local_path().unwrap()).unwrap();
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

        let error = match EndpointListener::local(endpoint.as_local_path().unwrap()) {
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
                std::thread::spawn(move || {
                    sender
                        .send(EndpointListener::local(endpoint.as_local_path().unwrap()))
                        .unwrap()
                })
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
        let original = EndpointListener::local(endpoint.as_local_path().unwrap()).unwrap();
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

        let error = match EndpointListener::local(endpoint.as_local_path().unwrap()) {
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
