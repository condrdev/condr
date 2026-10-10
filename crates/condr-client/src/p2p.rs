//! Peer-to-peer connections (ADR 0026): QUIC dialled by Device key through `iroh`,
//! hole-punched when possible and forwarded by Condr's relay when not.
//!
//! A key may live in only one endpoint at a time (ADR 0025). On a computer that endpoint is
//! the Server's: it accepts remote Devices when `[server.p2p] enabled` is set, and dials
//! for the GUI and CLI on its machine, which ask over the local socket with a `Tunnel`
//! frame. On a phone it is the Companion's own, which only dials (ADR 0040). `iroh` is
//! named only here; everything a person sees says p2p.

use crate::{Authority, PublicKey, Secret};
use condr_core::protocol::ClientHandshake;
use condr_core::{ProxyMode, ProxySetting};
use iroh::address_lookup::{DnsAddressLookup, PkarrPublisher};
use iroh::endpoint::{Connection, Incoming, RecvStream, SendStream, presets};
use iroh::{Endpoint as IrohEndpoint, EndpointAddr, RelayMode, RelayUrl, SecretKey, TransportAddr};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::runtime::Handle;

/// The lookup [`Network::Loopback`] shares between endpoints in tests.
pub use iroh::address_lookup::MemoryLookup;

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

/// What an accepting endpoint does with a connection a remote Device opened.
pub type Accept = Box<dyn Fn(P2pStream) -> io::Result<()> + Send + Sync>;

/// A Device reached Peer-to-peer by its key alone (ADR 0026). A desktop Client asks its
/// own Server to dial it through a `Tunnel` (ADR 0025); the Companion dials it with its own
/// [`P2pNode`] (ADR 0040).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct P2pEndpoint {
    pub device: PublicKey,
    pub invite: Option<Secret>,
}

impl P2pEndpoint {
    /// Parses `p2p://<id>[.<invite>]`, as printed by an invite.
    pub fn parse(text: &str) -> io::Result<Self> {
        let invalid = |reason: &str| io::Error::new(io::ErrorKind::InvalidInput, reason.to_owned());
        let credentials = text
            .trim()
            .strip_prefix("p2p://")
            .ok_or_else(|| invalid("expected p2p://<id>[.<invite>]"))?
            .trim_end_matches('/');
        let (device, invite) = match credentials.split_once('.') {
            Some((device, invite)) => (device, Some(Secret::parse(invite)?)),
            None => (credentials, None),
        };
        Ok(Self {
            device: PublicKey::parse(device)?,
            invite,
        })
    }

    pub fn without_invite(mut self) -> Self {
        self.invite = None;
        self
    }
}

/// This machine's Peer-to-peer endpoint, bound on demand.
pub struct P2pNode {
    handle: Handle,
    authority: Arc<dyn Authority>,
    /// `[server.p2p] enabled`: accept remote Devices and publish the relay to DNS. Without
    /// it the endpoint only dials, publishes nothing, and goes away when idle.
    accept: Option<Accept>,
    network: Network,
    /// `[network.proxy]` as this process started with it (ADR 0038).
    proxy: ProxySetting,
    bound: Mutex<Option<Bound>>,
}

struct Bound {
    endpoint: IrohEndpoint,
    connections: Arc<AtomicUsize>,
    accept_task: Option<tokio::task::JoinHandle<()>>,
}

impl P2pNode {
    /// `accept` receives the connections remote Devices open; `None` only dials. The key
    /// comes from `authority` when the endpoint first binds, and so do the answers an
    /// accepted Device's credential and pairing need.
    pub fn new(
        handle: Handle,
        authority: Arc<dyn Authority>,
        network: Network,
        proxy: ProxySetting,
        accept: Option<Accept>,
    ) -> Arc<Self> {
        Arc::new(Self {
            handle,
            authority,
            accept,
            network,
            proxy,
            bound: Mutex::new(None),
        })
    }

    pub fn enabled(&self) -> bool {
        self.accept.is_some()
    }

    pub fn proxy(&self) -> &ProxySetting {
        &self.proxy
    }

    /// Binds now, so an enabled Server is reachable from the start.
    pub fn start(self: &Arc<Self>) -> io::Result<()> {
        self.endpoint().map(drop)
    }

    /// The bound endpoint, binding it first when needed.
    fn endpoint(self: &Arc<Self>) -> io::Result<IrohEndpoint> {
        let mut bound = lock(&self.bound);
        if let Some(bound) = &*bound {
            return Ok(bound.endpoint.clone());
        }
        let key = self.authority.device_key()?;
        let secret = SecretKey::from_bytes(key.seed());
        let enabled = self.enabled();
        let network = self.network.clone();
        let proxy = matches!(network, Network::Condr)
            .then(|| relay_proxy(&self.proxy))
            .flatten();
        let proxy_address = proxy
            .as_ref()
            .map(|url| url[url::Position::BeforeHost..url::Position::AfterPort].to_owned());
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
                    if let Some(proxy) = proxy {
                        builder = builder.proxy_url(proxy);
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
        tracing::info!(
            id = %key.public(),
            accepting = enabled,
            proxy = proxy_address.as_deref(),
            "p2p endpoint bound"
        );
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
        let stream = P2pStream::new(
            conn,
            send,
            recv,
            self.handle.clone(),
            Peer::Server {
                remote,
                authority: Arc::clone(&self.authority),
                pairing: None,
            },
            self.guard(),
        );
        match &self.accept {
            Some(accept) => accept(stream),
            None => Err(rejected("this endpoint accepts nothing")),
        }
    }

    /// Closes the endpoint now, whatever still uses it; the next dial binds it again. A
    /// Companion does this when it leaves the screen (ADR 0040).
    pub fn close(&self) {
        let Some(bound) = lock(&self.bound).take() else {
            return;
        };
        if let Some(task) = bound.accept_task {
            task.abort();
        }
        self.handle
            .block_on(async move { bound.endpoint.close().await });
    }

    /// Tells the endpoint the network changed, which iroh cannot see for itself on
    /// Android (ADR 0040).
    pub fn network_changed(&self) {
        let endpoint = lock(&self.bound)
            .as_ref()
            .map(|bound| bound.endpoint.clone());
        if let Some(endpoint) = endpoint {
            self.handle
                .block_on(async move { endpoint.network_change().await });
        }
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
        if node.enabled() {
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
        authority: Arc<dyn Authority>,
        /// The invite the peer redeems, until the pairing is recorded.
        pairing: Option<Secret>,
    },
}

/// One QUIC stream as blocking `Read + Write`, driven from the caller's threads. Clones
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
                authority,
                pairing: None,
            } => authority
                .is_authorized(remote)
                .map(|authorized| !authorized),
            _ => Ok(false),
        }
    }

    /// Server side: accepts the invite an unknown peer presents, or refuses the peer.
    pub fn present_invite(&self, invite: &Secret) -> io::Result<()> {
        let mut peer = lock(&self.peer);
        let Peer::Server {
            authority, pairing, ..
        } = &mut *peer
        else {
            return Err(rejected("unexpected credential"));
        };
        // Compared in constant time: a Device the store does not know redeems its invite
        // this way (ADR 0026).
        let matches = authority.pending_invite()?.is_some_and(|pending| {
            pending
                .as_bytes()
                .iter()
                .zip(invite.as_bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
        });
        if matches {
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
            authority,
            pairing: Some(invite),
        } = &*peer
        else {
            return Ok(());
        };
        authority.complete_pairing(*remote, name, invite)?;
        if let Peer::Server { pairing, .. } = &mut *peer {
            *pairing = None;
        }
        Ok(())
    }

    pub fn peer_authorized(&self) -> io::Result<bool> {
        match &*lock(&self.peer) {
            Peer::Server {
                remote, authority, ..
            } => authority.is_authorized(remote),
            Peer::Client => Ok(true),
        }
    }

    pub fn record_seen(&self) -> io::Result<()> {
        match &*lock(&self.peer) {
            Peer::Server {
                remote, authority, ..
            } => authority.record_seen(remote),
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

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The proxy the relay connection goes through (ADR 0038).
fn relay_proxy(setting: &ProxySetting) -> Option<url::Url> {
    match setting.mode {
        ProxyMode::None => None,
        ProxyMode::Manual => setting.manual_url(),
        ProxyMode::System => system_proxy(
            &hyper_util::client::proxy::matcher::Matcher::from_system(),
            RELAY_URL,
        ),
    }
}

/// What `matcher` names for `target`. From the system, that is the `HTTPS_PROXY` family
/// and `NO_PROXY`, then the Windows registry or macOS system configuration, as the GUI's
/// reqwest reads them.
fn system_proxy(
    matcher: &hyper_util::client::proxy::matcher::Matcher,
    target: &str,
) -> Option<url::Url> {
    use base64::Engine as _;
    let intercept = matcher.intercept(&target.parse().ok()?)?;
    let mut proxy: url::Url = intercept.uri().to_string().parse().ok()?;
    // The matcher moves credentials into a header; iroh reads them from the URL.
    let credentials = intercept
        .basic_auth()
        .and_then(|header| {
            header
                .to_str()
                .ok()?
                .strip_prefix("Basic ")
                .map(str::to_owned)
        })
        .and_then(|encoded| {
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .ok()
        })
        .and_then(|decoded| String::from_utf8(decoded).ok());
    if let Some((user, password)) = credentials.as_deref().and_then(|pair| pair.split_once(':')) {
        let _ = proxy.set_username(user);
        let _ = proxy.set_password(Some(password));
    }
    Some(proxy)
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

    #[test]
    fn the_relay_proxy_follows_the_mode_and_keeps_system_credentials() {
        use hyper_util::client::proxy::matcher::Matcher;
        let setting = |mode, url: &str| ProxySetting {
            mode,
            url: url.into(),
        };
        assert_eq!(
            relay_proxy(&setting(ProxyMode::None, "http://proxy:8080")),
            None
        );
        assert_eq!(
            relay_proxy(&setting(ProxyMode::Manual, "http://proxy:8080")).map(String::from),
            Some("http://proxy:8080/".into())
        );
        assert_eq!(relay_proxy(&setting(ProxyMode::Manual, "")), None);

        let matcher = Matcher::builder()
            .all("http://user:p%40ss@proxy:8080")
            .build();
        let proxy = system_proxy(&matcher, RELAY_URL).unwrap();
        assert_eq!(proxy.host_str(), Some("proxy"));
        assert_eq!(proxy.port(), Some(8080));
        assert_eq!(
            proxy.username(),
            "user",
            "iroh reads the credentials from the URL"
        );
        assert_eq!(proxy.password(), Some("p%40ss"));
        let bypassed = Matcher::builder()
            .all("http://proxy:8080")
            .no("relay.condr.dev")
            .build();
        assert_eq!(
            system_proxy(&bypassed, RELAY_URL),
            None,
            "NO_PROXY is honoured"
        );
    }
}
