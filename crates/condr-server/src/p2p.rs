//! Peer-to-peer connections (ADR 0026): QUIC dialled by Device key through `iroh`,
//! hole-punched when possible and forwarded by Condr's relay when not.
//!
//! The Server is its machine's only Peer-to-peer endpoint (ADR 0025). It accepts remote
//! Devices when `[server.p2p] enabled` is set, and it dials for the GUI and CLI on this
//! machine, which ask over the local socket with a `Tunnel` frame and get their bytes
//! spliced onto the QUIC stream. `iroh` is named only here; everything a person sees says
//! p2p.

use crate::endpoint::EndpointStream;
use crate::noise::{PublicKey, Secret, ServerIdentity};
use condr_core::protocol::ClientHandshake;
use iroh::address_lookup::{DnsAddressLookup, MemoryLookup, PkarrPublisher};
use iroh::endpoint::{Connection, Incoming, RecvStream, SendStream, presets};
use iroh::{Endpoint as IrohEndpoint, EndpointAddr, RelayMode, RelayUrl, SecretKey, TransportAddr};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::Duration;
use tokio::runtime::Handle;

/// Condr's relay and DNS, compiled in; self-hosting is out of scope for now (ADR 0026).
pub const RELAY_URL: &str = "https://relay.condr.dev";
pub const DNS_ORIGIN: &str = "dns.condr.dev";
pub const PKARR_URL: &str = "https://dns.condr.dev/pkarr";
const ALPN: &[u8] = b"condr/1";
/// A bound endpoint that accepts nothing is released this long after its last connection.
const IDLE_RELEASE: Duration = Duration::from_secs(60);
/// Shorter than the 15 seconds a tunnelled Client waits for `Welcome`, so a failed dial
/// still reaches it as one `Error` frame (ADR 0025).
const DIAL_TIMEOUT: Duration = Duration::from_secs(10);

/// Where an endpoint finds the relay and other Devices.
#[derive(Clone, Debug)]
pub enum Network {
    /// Condr's relay and DNS.
    Condr,
    /// Tests: loopback sockets only, Devices registered in one shared lookup.
    Loopback(MemoryLookup),
}

/// This machine's Peer-to-peer endpoint, bound on demand.
pub struct P2pNode {
    handle: Handle,
    identity: Mutex<Option<Arc<ServerIdentity>>>,
    /// `[server.p2p] enabled`: accept remote Devices and publish the relay to DNS. Without
    /// it the endpoint only dials, publishes nothing, and goes away when idle.
    enabled: bool,
    network: Network,
    accepted: Sender<EndpointStream>,
    bound: Mutex<Option<Bound>>,
}

struct Bound {
    endpoint: IrohEndpoint,
    connections: Arc<AtomicUsize>,
    accept_task: Option<tokio::task::JoinHandle<()>>,
}

impl P2pNode {
    /// `identity` may be `None` for a Server that neither listens on TCP nor accepts
    /// Peer-to-peer; it is loaded from the identity directory on the first `Tunnel`.
    pub fn new(
        handle: Handle,
        identity: Option<Arc<ServerIdentity>>,
        enabled: bool,
        network: Network,
        accepted: Sender<EndpointStream>,
    ) -> Arc<Self> {
        Arc::new(Self {
            handle,
            identity: Mutex::new(identity),
            enabled,
            network,
            accepted,
            bound: Mutex::new(None),
        })
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Binds now, so an enabled Server is reachable from the start.
    pub fn start(self: &Arc<Self>) -> io::Result<()> {
        self.endpoint().map(drop)
    }

    fn identity(&self) -> io::Result<Arc<ServerIdentity>> {
        let mut identity = lock(&self.identity);
        if let Some(identity) = &*identity {
            return Ok(Arc::clone(identity));
        }
        let loaded = Arc::new(ServerIdentity::load_or_create(
            &crate::noise::identity_directory()?,
        )?);
        *identity = Some(Arc::clone(&loaded));
        Ok(loaded)
    }

    /// The bound endpoint, binding it first when needed.
    fn endpoint(self: &Arc<Self>) -> io::Result<IrohEndpoint> {
        let mut bound = lock(&self.bound);
        if let Some(bound) = &*bound {
            return Ok(bound.endpoint.clone());
        }
        let identity = self.identity()?;
        let secret = SecretKey::from_bytes(identity.device_key().seed());
        let enabled = self.enabled;
        let network = self.network.clone();
        let endpoint = self.handle.block_on(async move {
            let mut builder = IrohEndpoint::builder(presets::Minimal).secret_key(secret);
            if enabled {
                builder = builder.alpns(vec![ALPN.to_vec()]);
            }
            builder = match &network {
                Network::Condr => {
                    let relay: RelayUrl = RELAY_URL.parse().map_err(other)?;
                    let mut builder = builder
                        .relay_mode(RelayMode::custom([relay]))
                        .address_lookup(DnsAddressLookup::builder(DNS_ORIGIN.to_owned()));
                    if enabled {
                        let pkarr: url::Url = PKARR_URL.parse().map_err(other)?;
                        builder = builder.address_lookup(PkarrPublisher::builder(pkarr));
                    }
                    builder
                }
                Network::Loopback(lookup) => builder
                    .relay_mode(RelayMode::Disabled)
                    .clear_ip_transports()
                    .bind_addr("127.0.0.1:0")
                    .map_err(other)?
                    .address_lookup(lookup.clone()),
            };
            builder.bind().await.map_err(other)
        })?;
        if let Network::Loopback(lookup) = &self.network {
            lookup.add_endpoint_info(EndpointAddr::from_parts(
                endpoint.id(),
                endpoint.bound_sockets().into_iter().map(TransportAddr::Ip),
            ));
        }
        tracing::info!(id = %identity.public_key(), accepting = enabled, "p2p endpoint bound");
        let accept_task = enabled.then(|| {
            let endpoint = endpoint.clone();
            let node = Arc::downgrade(self);
            self.handle.spawn(async move {
                while let Some(incoming) = endpoint.accept().await {
                    let Some(node) = node.upgrade() else { return };
                    tokio::spawn(async move {
                        if let Err(error) = node.accept_connection(incoming).await {
                            tracing::warn!("p2p connection not accepted: {error}");
                        }
                    });
                }
            })
        });
        *bound = Some(Bound {
            endpoint: endpoint.clone(),
            connections: Arc::new(AtomicUsize::new(0)),
            accept_task,
        });
        Ok(endpoint)
    }

    async fn accept_connection(self: Arc<Self>, incoming: Incoming) -> io::Result<()> {
        let conn = incoming.await.map_err(other)?;
        let remote = PublicKey::from_bytes(*conn.remote_id().as_bytes());
        let (send, recv) = conn.accept_bi().await.map_err(other)?;
        let identity = self.identity()?;
        let stream = P2pStream::new(
            conn,
            send,
            recv,
            self.handle.clone(),
            Peer::Server {
                remote,
                identity,
                pairing: None,
            },
            self.guard(),
        );
        self.accepted
            .send(EndpointStream::P2p(stream))
            .map_err(|_| io::Error::other("the Server stopped accepting"))
    }

    /// Dials `device` and presents `invite` when this machine is not paired with it yet.
    pub fn dial(
        self: &Arc<Self>,
        device: PublicKey,
        invite: Option<&Secret>,
    ) -> io::Result<P2pStream> {
        let endpoint = self.endpoint()?;
        let id = iroh::EndpointId::from_bytes(device.as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid device id"))?;
        let mut addr = EndpointAddr::new(id);
        if let Network::Condr = self.network {
            // Every Device uses Condr's relay, so name it as well as looking it up: the
            // dial then works while the DNS server is down (ADR 0026).
            addr = addr.with_relay_url(RELAY_URL.parse().map_err(other)?);
        }
        let credential = invite.map(|invite| {
            let mut frame = Vec::new();
            condr_core::protocol::write_message(
                &mut frame,
                &ClientHandshake::Credential {
                    invite: *invite.as_bytes(),
                },
            )
            .map(|()| frame)
        });
        let (conn, send, recv) = self.handle.block_on(async move {
            let conn = tokio::time::timeout(DIAL_TIMEOUT, endpoint.connect(addr, ALPN))
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "the device did not answer"))?
                .map_err(other)?;
            let (mut send, recv) = conn.open_bi().await.map_err(other)?;
            if let Some(frame) = credential {
                let frame = frame.map_err(other)?;
                send.write_all(&frame).await.map_err(other)?;
            }
            Ok::<_, io::Error>((conn, send, recv))
        })?;
        Ok(P2pStream::new(
            conn,
            send,
            recv,
            self.handle.clone(),
            Peer::Client,
            self.guard(),
        ))
    }

    fn guard(self: &Arc<Self>) -> Arc<ConnectionGuard> {
        let connections = lock(&self.bound)
            .as_ref()
            .map(|bound| Arc::clone(&bound.connections))
            .unwrap_or_default();
        connections.fetch_add(1, Ordering::AcqRel);
        Arc::new(ConnectionGuard {
            connections,
            node: Arc::downgrade(self),
        })
    }

    /// Releases the endpoint when nothing has used it since the guard that scheduled this.
    fn release_if_idle(self: &Arc<Self>) {
        let released = {
            let mut bound = lock(&self.bound);
            match &*bound {
                Some(state) if state.connections.load(Ordering::Acquire) == 0 => bound.take(),
                _ => None,
            }
        };
        if let Some(released) = released {
            if let Some(task) = released.accept_task {
                task.abort();
            }
            tracing::info!("p2p endpoint released after idling");
            self.handle
                .spawn(async move { released.endpoint.close().await });
        }
    }
}

/// Counts a connection for the idle release; clones of one stream share one guard.
struct ConnectionGuard {
    connections: Arc<AtomicUsize>,
    node: Weak<P2pNode>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        if self.connections.fetch_sub(1, Ordering::AcqRel) != 1 {
            return;
        }
        let Some(node) = self.node.upgrade() else {
            return;
        };
        if node.enabled {
            return;
        }
        let weak = self.node.clone();
        node.handle.spawn(async move {
            tokio::time::sleep(IDLE_RELEASE).await;
            if let Some(node) = weak.upgrade() {
                node.release_if_idle();
            }
        });
    }
}

enum Peer {
    Client,
    Server {
        remote: PublicKey,
        identity: Arc<ServerIdentity>,
        /// The invite the peer redeems, until the pairing is recorded.
        pairing: Option<Secret>,
    },
}

/// One QUIC stream as blocking `Read + Write`, driven from the Server's threads. Clones
/// share the streams, so one clone may read while another writes.
pub struct P2pStream {
    conn: Connection,
    send: Arc<Mutex<SendStream>>,
    recv: Arc<Mutex<RecvStream>>,
    handle: Handle,
    read_timeout: Arc<Mutex<Option<Duration>>>,
    peer: Arc<Mutex<Peer>>,
    _guard: Arc<ConnectionGuard>,
}

impl P2pStream {
    fn new(
        conn: Connection,
        send: SendStream,
        recv: RecvStream,
        handle: Handle,
        peer: Peer,
        guard: Arc<ConnectionGuard>,
    ) -> Self {
        Self {
            conn,
            send: Arc::new(Mutex::new(send)),
            recv: Arc::new(Mutex::new(recv)),
            handle,
            read_timeout: Arc::new(Mutex::new(None)),
            peer: Arc::new(Mutex::new(peer)),
            _guard: guard,
        }
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            conn: self.conn.clone(),
            send: Arc::clone(&self.send),
            recv: Arc::clone(&self.recv),
            handle: self.handle.clone(),
            read_timeout: Arc::clone(&self.read_timeout),
            peer: Arc::clone(&self.peer),
            _guard: Arc::clone(&self._guard),
        })
    }

    /// The remote Device's key on the Server side.
    pub fn remote_public_key(&self) -> Option<PublicKey> {
        match &*lock(&self.peer) {
            Peer::Server { remote, .. } => Some(*remote),
            Peer::Client => None,
        }
    }

    /// Server side: whether the peer is unknown and has not redeemed an invite yet, so
    /// its first frame must be a `Credential`.
    pub fn needs_credential(&self) -> io::Result<bool> {
        match &*lock(&self.peer) {
            Peer::Server {
                remote,
                identity,
                pairing: None,
            } => identity.is_authorized(remote).map(|authorized| !authorized),
            _ => Ok(false),
        }
    }

    /// Server side: accepts the invite an unknown peer presents, or refuses the peer.
    pub fn present_invite(&self, invite: &Secret) -> io::Result<()> {
        let mut peer = lock(&self.peer);
        let Peer::Server {
            identity, pairing, ..
        } = &mut *peer
        else {
            return Err(rejected("unexpected credential"));
        };
        if identity.pending_invite_matches(invite)? {
            *pairing = Some(invite.clone());
            Ok(())
        } else {
            Err(rejected("invite unknown, used or expired"))
        }
    }

    /// Records the peer as paired once its `Hello` has been read, as the TCP path does.
    pub fn complete_pairing(&self, name: &str) -> io::Result<()> {
        let mut peer = lock(&self.peer);
        let Peer::Server {
            remote,
            identity,
            pairing: Some(invite),
        } = &*peer
        else {
            return Ok(());
        };
        identity.complete_pairing(*remote, name, invite)?;
        if let Peer::Server { pairing, .. } = &mut *peer {
            *pairing = None;
        }
        Ok(())
    }

    pub fn peer_authorized(&self) -> io::Result<bool> {
        match &*lock(&self.peer) {
            Peer::Server {
                remote, identity, ..
            } => identity.is_authorized(remote),
            Peer::Client => Ok(true),
        }
    }

    pub fn record_seen(&self) -> io::Result<()> {
        match &*lock(&self.peer) {
            Peer::Server {
                remote, identity, ..
            } => identity.record_seen(remote),
            Peer::Client => Ok(()),
        }
    }

    pub fn set_read_timeout(&self, timeout: Option<Duration>) {
        *lock(&self.read_timeout) = timeout;
    }

    /// Closes the connection for every clone; readers see end of stream.
    pub fn shutdown(&self) -> io::Result<()> {
        self.conn.close(0u32.into(), b"");
        Ok(())
    }
}

impl Read for P2pStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let timeout = *lock(&self.read_timeout);
        let mut recv = lock(&self.recv);
        self.handle.block_on(async {
            let read = recv.read(buffer);
            let result = match timeout {
                Some(timeout) => tokio::time::timeout(timeout, read)
                    .await
                    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "p2p read timed out"))?,
                None => read.await,
            };
            match result {
                Ok(Some(length)) => Ok(length),
                Ok(None) => Ok(0),
                Err(error) => Err(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    error.to_string(),
                )),
            }
        })
    }
}

impl Write for P2pStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let mut send = lock(&self.send);
        self.handle.block_on(async {
            send.write_all(buffer)
                .await
                .map_err(|error| io::Error::new(io::ErrorKind::ConnectionReset, error.to_string()))
        })?;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Carries bytes between a local Client and the remote Device until either side closes.
/// The local socket has no shutdown that reaches a clone on another thread, so the
/// upstream copy polls with a timeout and leaves when the downstream copy has ended.
pub fn splice(local: EndpointStream, remote: P2pStream) -> io::Result<()> {
    let done = Arc::new(AtomicBool::new(false));
    let mut local_reader = local.try_clone()?;
    let mut remote_writer = remote.try_clone()?;
    let upstream_done = Arc::clone(&done);
    let upstream = thread::spawn(move || {
        let _ = local_reader.set_handshake_timeout(Some(Duration::from_millis(500)));
        let mut buffer = [0; 16 * 1024];
        loop {
            match local_reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(length) => {
                    if remote_writer.write_all(&buffer[..length]).is_err() {
                        break;
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    if upstream_done.load(Ordering::Acquire) {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = remote_writer.shutdown();
    });
    let mut remote_reader = remote;
    let mut local_writer = local;
    let result = io::copy(&mut remote_reader, &mut local_writer).map(drop);
    done.store(true, Ordering::Release);
    let _ = remote_reader.shutdown();
    drop(local_writer);
    let _ = upstream.join();
    result
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn other(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

fn rejected(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, reason.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::{create_invite, revoke};
    use condr_core::protocol::{Hello, ServerMessage, read_message, write_message};
    use std::sync::mpsc::Receiver;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "condr-p2p-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_node(
        handle: &Handle,
        identity: Arc<ServerIdentity>,
        enabled: bool,
        lookup: &MemoryLookup,
    ) -> (Arc<P2pNode>, Receiver<EndpointStream>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let node = P2pNode::new(
            handle.clone(),
            Some(identity),
            enabled,
            Network::Loopback(lookup.clone()),
            tx,
        );
        (node, rx)
    }

    fn frame(message: &ClientHandshake) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_message(&mut bytes, message).unwrap();
        bytes
    }

    /// The whole Peer-to-peer contract over loopback (ADR 0026): an unknown Device dials,
    /// redeems its invite, exchanges frames both ways, is recorded, and a revoke drops it.
    #[test]
    fn an_invite_pairs_a_device_and_a_revoke_drops_it() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let handle = runtime.handle().clone();
        let lookup = MemoryLookup::new();

        let server_dir = temp_dir("server");
        let server_identity = Arc::new(ServerIdentity::load_or_create(&server_dir).unwrap());
        let invite = create_invite(&server_dir).unwrap().secret;
        let client_dir = temp_dir("client");
        let client_identity = Arc::new(ServerIdentity::load_or_create(&client_dir).unwrap());
        let client_public = client_identity.public_key();
        let server_id = server_identity.public_key();

        let (server_node, accepted) = make_node(&handle, server_identity.clone(), true, &lookup);
        server_node.start().unwrap();
        let (client_node, _client_rx) = make_node(&handle, client_identity, false, &lookup);

        let client = std::thread::spawn(move || {
            let mut stream = client_node.dial(server_id, Some(&invite)).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(10)));
            stream
                .write_all(&frame(&ClientHandshake::Hello(Hello::new("laptop"))))
                .unwrap();
            match read_message::<_, ServerMessage>(&mut stream).unwrap() {
                ServerMessage::Error { message } => message,
                other => panic!("expected the server's reply, got {other:?}"),
            }
        });

        let mut peer = match accepted
            .recv_timeout(Duration::from_secs(10))
            .expect("server accepts the connection")
        {
            EndpointStream::P2p(peer) => peer,
            other => panic!("expected a p2p stream, got {}", other.transport()),
        };
        peer.set_read_timeout(Some(Duration::from_secs(10)));
        assert!(peer.needs_credential().unwrap(), "the device is unknown");
        match read_message::<_, ClientHandshake>(&mut peer).unwrap() {
            ClientHandshake::Credential { invite } => {
                peer.present_invite(&Secret::from_bytes(invite)).unwrap();
            }
            other => panic!("expected a credential, got {other:?}"),
        }
        match read_message::<_, ClientHandshake>(&mut peer).unwrap() {
            ClientHandshake::Hello(hello) => assert_eq!(hello.client_name, "laptop"),
            other => panic!("expected Hello, got {other:?}"),
        }
        peer.complete_pairing("laptop").unwrap();
        assert!(peer.peer_authorized().unwrap(), "recorded once paired");
        write_message(
            &mut peer,
            &ServerMessage::Error {
                message: "welcome".into(),
            },
        )
        .unwrap();

        assert_eq!(client.join().unwrap(), "welcome", "frames flow both ways");
        assert!(
            server_identity.is_authorized(&client_public).unwrap(),
            "the invite recorded the device"
        );

        // A revoke closes the door: the still-open connection is no longer authorized.
        assert_eq!(
            revoke(&server_dir, &client_public.to_string()[..8]).unwrap(),
            Some(client_public)
        );
        assert!(!peer.peer_authorized().unwrap(), "revoked device refused");

        let _ = std::fs::remove_dir_all(&server_dir);
        let _ = std::fs::remove_dir_all(&client_dir);
    }
}
