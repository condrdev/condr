use std::fs;
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use interprocess::local_socket::traits::Listener as _;

pub type LocalListener = interprocess::local_socket::Listener;
pub type LocalStream = interprocess::local_socket::Stream;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Endpoint {
    Local(PathBuf),
    Tcp(SocketAddr),
}

pub enum EndpointListener {
    Local(LocalListener),
    Tcp(TcpListener),
}

pub enum EndpointStream {
    Local(LocalStream),
    Tcp(TcpStream),
}

impl Endpoint {
    pub fn local(path: impl Into<PathBuf>) -> Self {
        Self::Local(path.into())
    }

    pub fn tcp(address: SocketAddr) -> Self {
        Self::Tcp(address)
    }

    pub fn bind(&self) -> io::Result<EndpointListener> {
        match self {
            Self::Local(path) => bind_local(path).map(EndpointListener::Local),
            Self::Tcp(address) => TcpListener::bind(address).map(EndpointListener::Tcp),
        }
    }

    pub fn connect(&self) -> io::Result<EndpointStream> {
        match self {
            Self::Local(path) => connect_local(path).map(EndpointStream::Local),
            Self::Tcp(address) => TcpStream::connect(address).map(EndpointStream::Tcp),
        }
    }

    pub fn cleanup(&self) -> io::Result<()> {
        if let Self::Local(path) = self {
            match fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            }
        } else {
            Ok(())
        }
    }

    pub fn as_local_path(&self) -> Option<&Path> {
        match self {
            Self::Local(path) => Some(path),
            Self::Tcp(_) => None,
        }
    }
}

impl EndpointListener {
    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        match self {
            Self::Local(listener) => {
                use interprocess::local_socket::ListenerNonblockingMode;

                listener.set_nonblocking(if nonblocking {
                    ListenerNonblockingMode::Accept
                } else {
                    ListenerNonblockingMode::Both
                })
            }
            Self::Tcp(listener) => listener.set_nonblocking(nonblocking),
        }
    }

    pub fn accept(&self) -> io::Result<EndpointStream> {
        match self {
            Self::Local(listener) => listener.accept().map(EndpointStream::Local),
            Self::Tcp(listener) => {
                let (stream, _) = listener.accept()?;
                // Accepted sockets inherit nonblocking mode on Windows. Client handlers use
                // blocking framed reads, so normalize the stream on every platform.
                stream.set_nonblocking(false)?;
                Ok(EndpointStream::Tcp(stream))
            }
        }
    }

    pub fn local_addr(&self) -> io::Result<Option<SocketAddr>> {
        match self {
            Self::Local(_) => Ok(None),
            Self::Tcp(listener) => listener.local_addr().map(Some),
        }
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
            Self::Tcp(stream) => stream.set_read_timeout(timeout),
        }
    }
}

pub fn default_socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("MURMUR_SOCKET_PATH") {
        return PathBuf::from(path);
    }

    #[cfg(windows)]
    let config_dir = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("APPDATA"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("murmur");

    #[cfg(not(windows))]
    let config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("murmur");

    config_dir.join("murmur.sock")
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

fn bind_local(path: &Path) -> io::Result<LocalListener> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    prepare_local_path(path)?;

    #[cfg(unix)]
    {
        use interprocess::local_socket::{GenericFilePath, ListenerOptions, prelude::*};
        let name = path.to_fs_name::<GenericFilePath>()?;
        let listener = ListenerOptions::new()
            .name(name)
            .reclaim_name(false)
            .create_sync()?;
        restrict_permissions(path)?;
        Ok(listener)
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
        fs::write(path, b"murmur-local-endpoint\n")?;
        Ok(listener)
    }
}

fn prepare_local_path(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }

    match connect_local(path) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("Murmur server is already running at {}", path.display()),
        )),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionRefused
                    | io::ErrorKind::NotFound
                    | io::ErrorKind::TimedOut
            ) =>
        {
            fs::remove_file(path)
        }
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_endpoint_round_trips() {
        let path = std::env::temp_dir().join(format!(
            "murmur-endpoint-{}-{}.sock",
            std::process::id(),
            unique_suffix()
        ));
        let endpoint = Endpoint::local(&path);
        let listener = endpoint.bind().unwrap();
        listener.set_nonblocking(true).unwrap();
        let _ = endpoint.cleanup();
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    }
}
