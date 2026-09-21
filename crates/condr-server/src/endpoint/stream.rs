//! Transport-independent stream operations and connection cancellation.

use super::*;

pub enum EndpointStream {
    Local(LocalStream),
    Tcp(NoiseStream),
    Ssh(SshStream),
}

/// Cancels a pending or established remote connection without waiting for its
/// reader/writer queue. Local connections still use protocol Detach.
#[derive(Clone, Default)]
pub struct ConnectionCancellation {
    state: Arc<Mutex<CancellationState>>,
}

#[derive(Default)]
struct CancellationState {
    cancelled: bool,
    stream: Option<EndpointStream>,
}

impl ConnectionCancellation {
    pub fn cancel(&self) {
        let mut state = self.state.lock().unwrap();
        state.cancelled = true;
        if let Some(stream) = state.stream.take() {
            let _ = stream.shutdown();
        }
    }

    pub(crate) fn connect(&self, endpoint: &Endpoint) -> io::Result<EndpointStream> {
        if self.state.lock().unwrap().cancelled {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "connection cancelled",
            ));
        }
        let stream = endpoint.connect()?;
        self.attach(&stream)?;
        Ok(stream)
    }

    pub(crate) fn attach(&self, stream: &EndpointStream) -> io::Result<()> {
        let mut state = self.state.lock().unwrap();
        if state.cancelled {
            let _ = stream.shutdown();
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "connection cancelled",
            ));
        }
        state.stream = match stream {
            EndpointStream::Local(_) => None,
            _ => Some(stream.try_clone()?),
        };
        Ok(())
    }

    pub(crate) fn clear(&self) {
        if let Some(stream) = self.state.lock().unwrap().stream.take() {
            let _ = stream.shutdown();
        }
    }
}

impl std::io::Read for EndpointStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Local(stream) => stream.read(buffer),
            Self::Tcp(stream) => stream.read(buffer),
            Self::Ssh(stream) => stream.read(buffer),
        }
    }
}

impl std::io::Write for EndpointStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Local(stream) => stream.write(buffer),
            Self::Tcp(stream) => stream.write(buffer),
            Self::Ssh(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Local(stream) => stream.flush(),
            Self::Tcp(stream) => stream.flush(),
            Self::Ssh(stream) => stream.flush(),
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
            Self::Ssh(stream) => stream.try_clone().map(Self::Ssh),
        }
    }

    /// Records a TCP peer that connected with an invite as an authorized device; a no-op
    /// for local and already-paired peers. Call after its first message was read.
    pub fn complete_pairing(&self, client_name: &str) -> io::Result<()> {
        match self {
            Self::Local(_) | Self::Ssh(_) => Ok(()),
            Self::Tcp(stream) => stream.complete_pairing(client_name),
        }
    }

    /// The Device key of a TCP peer; `None` for local connections.
    pub fn peer_key(&self) -> Option<PublicKey> {
        match self {
            Self::Local(_) | Self::Ssh(_) => None,
            Self::Tcp(stream) => stream.remote_public_key(),
        }
    }

    /// Whether the store still authorizes a TCP peer; local peers always are. Checked
    /// again after the handshake so a device revoked meanwhile never gets served.
    pub fn peer_authorized(&self) -> io::Result<bool> {
        match self {
            Self::Local(_) | Self::Ssh(_) => Ok(true),
            Self::Tcp(stream) => stream.peer_authorized(),
        }
    }

    /// Records that a TCP peer connected now, for `condr server clients`.
    pub fn record_peer_seen(&self) -> io::Result<()> {
        match self {
            Self::Local(_) | Self::Ssh(_) => Ok(()),
            Self::Tcp(stream) => stream.record_seen(),
        }
    }

    /// Whether the peer may administer the Server. Local and SSH bridge connections execute
    /// on the Server host; TCP clients are deliberately read-only.
    pub fn may_administer(&self) -> bool {
        matches!(self, Self::Local(_) | Self::Ssh(_))
    }

    /// The transport's name for log spans.
    pub fn transport(&self) -> &'static str {
        match self {
            Self::Local(_) => "local",
            Self::Tcp(_) => "tcp",
            Self::Ssh(_) => "ssh",
        }
    }

    /// Closes TCP/SSH for all clones; local streams close on drop.
    pub fn shutdown(&self) -> io::Result<()> {
        match self {
            Self::Local(_) => Ok(()),
            Self::Tcp(stream) => stream.shutdown(),
            Self::Ssh(stream) => stream.shutdown(),
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
            Self::Ssh(stream) => stream.set_handshake_timeout(timeout),
        }
    }
}
