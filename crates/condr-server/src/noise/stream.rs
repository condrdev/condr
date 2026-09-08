//! Noise handshake and encrypted, bounded transport records.

use super::*;

const NOISE_PATTERN: &str = "Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s";

/// `IKpsk2` mixes the pre-shared key into the second message.
const PSK_LOCATION: usize = 2;

const TAG_LEN: usize = 16;

const MAX_RECORD: usize = 65535;

const MAX_PLAINTEXT: usize = MAX_RECORD - TAG_LEN;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, thread};
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
}
