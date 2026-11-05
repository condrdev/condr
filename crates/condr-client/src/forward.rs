//! Forwards (ADR 0061): a listener on this machine's loopback for one port of one Device,
//! each accepted connection one Stream to the Device's own loopback; and [`StreamLink`],
//! which keeps a GUI's Stream connection to a Device open while forwards need it.

use crate::Refused;
use crate::streams::{Stream, StreamConnection, StreamFailure};
use condr_core::protocol::{MAX_STREAM_CHUNK, StreamError, StreamTarget};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

/// How long a port may take to answer before it counts as free: a listener on loopback
/// answers at once.
const PROBE_TIMEOUT: Duration = Duration::from_millis(200);
/// The ports above the Device's own that a forward without a local port tries.
const NEARBY_PORTS: u16 = 20;
/// The first wait before a lost Stream connection is dialled again, and the longest.
const RETRY_FIRST: Duration = Duration::from_secs(2);
const RETRY_LONGEST: Duration = Duration::from_secs(30);

/// Copies between `stream` and `socket` until both directions finished; each end's FIN
/// becomes the other's, so a response delimited by closing arrives whole. Both ends of
/// a forward use it. A panic in either direction resets the Stream alone (ADR 0053).
pub fn splice(stream: &Arc<Stream>, mut socket: TcpStream) {
    let _ = socket.set_nodelay(true);
    let upstream = match socket.try_clone() {
        Ok(socket) => socket,
        Err(error) => {
            stream.reset(StreamError::Io, error.to_string());
            return;
        }
    };
    let reader_stream = Arc::clone(stream);
    let reader = thread::Builder::new()
        .name("condr-forward-read".into())
        .spawn(move || isolated(&reader_stream, |stream| socket_to_stream(stream, upstream)));
    let Ok(reader) = reader else {
        stream.reset(StreamError::Io, "could not start the forward's reader");
        return;
    };
    isolated(stream, |stream| {
        loop {
            match stream.read() {
                Ok(Some(data)) => {
                    if socket.write_all(&data).is_err() {
                        stream.reset(StreamError::Io, "the forwarded connection closed");
                        // Wakes the reader, which may be blocked on this socket.
                        let _ = socket.shutdown(Shutdown::Both);
                        break;
                    }
                }
                Ok(None) => {
                    let _ = socket.shutdown(Shutdown::Write);
                    break;
                }
                Err(_) => {
                    let _ = socket.shutdown(Shutdown::Both);
                    break;
                }
            }
        }
    });
    let _ = reader.join();
}

/// Runs a Stream's worker so that its panic resets that Stream and nothing else.
pub fn isolated(stream: &Arc<Stream>, work: impl FnOnce(&Arc<Stream>)) {
    if catch_unwind(AssertUnwindSafe(|| work(stream))).is_err() {
        stream.reset(StreamError::Other, "the worker for this stream failed");
    }
}

fn socket_to_stream(stream: &Arc<Stream>, mut socket: TcpStream) {
    let mut buffer = vec![0; MAX_STREAM_CHUNK];
    loop {
        match socket.read(&mut buffer) {
            Ok(0) => {
                stream.finish();
                return;
            }
            Ok(read) => {
                if stream.write(&buffer[..read]).is_err() {
                    let _ = socket.shutdown(Shutdown::Both);
                    return;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                stream.reset(StreamError::Io, error.to_string());
                return;
            }
        }
    }
}

/// Where a forward opens its Streams: the CLI's one Stream connection, or a GUI's
/// [`StreamLink`], which may be reconnecting.
pub trait StreamOpener: Send + Sync {
    fn open(&self, target: StreamTarget) -> Result<Stream, StreamFailure>;
}

impl StreamOpener for StreamConnection {
    fn open(&self, target: StreamTarget) -> Result<Stream, StreamFailure> {
        StreamConnection::open(self, target)
    }
}

/// Which local port a forward takes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalPort {
    /// The GUI's rule: the Device's port when free, else the next free one up to twenty
    /// above, else one the system assigns.
    Near,
    /// Exactly this one, failing when taken, as a script needs; 0 lets the system choose.
    Exact(u16),
}

/// What a forward tells its holder as connections come and go.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ForwardEvent {
    /// The connection count or the last connection's outcome changed.
    Changed,
    /// The Device's port refused a connection; the browser's connection was closed.
    Refused(String),
}

/// One forward: listeners on `127.0.0.1` and, where the system has one, `[::1]`, for one
/// port of one Device. Dropping it stops listening, the port free once it returns;
/// connections through it run on.
pub struct Forward {
    remote: u16,
    local: u16,
    shared: Arc<ForwardShared>,
    /// Each listener's address and the thread that owns it.
    accepters: Vec<(SocketAddr, thread::JoinHandle<()>)>,
}

struct ForwardShared {
    stopped: AtomicBool,
    connections: AtomicUsize,
    last_refused: AtomicBool,
    on_event: Box<dyn Fn(ForwardEvent) + Send + Sync>,
}

impl Forward {
    /// Binds the local listeners and starts accepting. A connection accepted while
    /// `opener` cannot open a Stream is closed at once rather than queued.
    pub fn bind(
        remote: u16,
        local: LocalPort,
        opener: Arc<dyn StreamOpener>,
        on_event: impl Fn(ForwardEvent) + Send + Sync + 'static,
    ) -> io::Result<Self> {
        let listeners = match local {
            LocalPort::Exact(port) => bind_loopback(port)?,
            LocalPort::Near => near_port(remote)?,
        };
        let local = listeners[0].local_addr()?.port();
        let shared = Arc::new(ForwardShared {
            stopped: AtomicBool::new(false),
            connections: AtomicUsize::new(0),
            last_refused: AtomicBool::new(false),
            on_event: Box::new(on_event),
        });
        let mut accepters = Vec::new();
        for listener in listeners {
            let address = listener.local_addr()?;
            let shared = Arc::clone(&shared);
            let opener = Arc::clone(&opener);
            let accepter = thread::Builder::new()
                .name("condr-forward-accept".into())
                .spawn(move || accept_loop(&listener, remote, &opener, &shared))?;
            accepters.push((address, accepter));
        }
        Ok(Self {
            remote,
            local,
            shared,
            accepters,
        })
    }

    pub fn remote_port(&self) -> u16 {
        self.remote
    }

    pub fn local_port(&self) -> u16 {
        self.local
    }

    /// Connections through this forward now.
    pub fn connections(&self) -> usize {
        self.shared.connections.load(Ordering::Acquire)
    }

    /// Whether the Device's port refused the latest connection.
    pub fn last_refused(&self) -> bool {
        self.shared.last_refused.load(Ordering::Acquire)
    }
}

impl Drop for Forward {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Release);
        // A blocked accept returns for this connection and sees the flag; its thread
        // owns the listener, so waiting for it frees the port before this returns.
        for (address, accepter) in std::mem::take(&mut self.accepters) {
            if TcpStream::connect_timeout(&address, PROBE_TIMEOUT).is_ok() {
                let _ = accepter.join();
            }
        }
    }
}

fn accept_loop(
    listener: &TcpListener,
    remote: u16,
    opener: &Arc<dyn StreamOpener>,
    shared: &Arc<ForwardShared>,
) {
    for socket in listener.incoming() {
        if shared.stopped.load(Ordering::Acquire) {
            return;
        }
        let Ok(socket) = socket else {
            // Out of descriptors and the like: wait for one to free up rather than spin.
            thread::sleep(Duration::from_millis(50));
            continue;
        };
        let stream = match opener.open(StreamTarget::Forward {
            port: remote.into(),
        }) {
            Ok(stream) => Arc::new(stream),
            // Reconnecting or gone: the browser sees a reset, not a hang.
            Err(_) => {
                let _ = socket.shutdown(Shutdown::Both);
                continue;
            }
        };
        let shared = Arc::clone(shared);
        let spawned = thread::Builder::new()
            .name("condr-forward".into())
            .spawn(move || {
                shared.connections.fetch_add(1, Ordering::AcqRel);
                (shared.on_event)(ForwardEvent::Changed);
                // `Open` is not a round trip: the browser's request follows it at once,
                // and a refusal ends the splice like any reset.
                splice(&stream, socket);
                match stream.opened() {
                    Ok(_) => {
                        if shared.last_refused.swap(false, Ordering::AcqRel) {
                            (shared.on_event)(ForwardEvent::Changed);
                        }
                    }
                    Err(failure) if failure.error == StreamError::ConnectionRefused => {
                        shared.last_refused.store(true, Ordering::Release);
                        (shared.on_event)(ForwardEvent::Refused(failure.to_string()));
                    }
                    Err(_) => {}
                }
                shared.connections.fetch_sub(1, Ordering::AcqRel);
                (shared.on_event)(ForwardEvent::Changed);
            });
        if spawned.is_err() {
            continue;
        }
    }
}

/// The GUI's choice of local port (ADR 0061).
fn near_port(remote: u16) -> io::Result<Vec<TcpListener>> {
    for port in (remote..=remote.saturating_add(NEARBY_PORTS)).filter(|port| *port != 0) {
        match bind_loopback(port) {
            Ok(listeners) => return Ok(listeners),
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => {}
            Err(error) => return Err(error),
        }
    }
    bind_loopback(0)
}

/// Both loopbacks on one port. A port is free only when nothing answers on it and both
/// binds succeed; `[::1]` may be missing only on a system without IPv6 loopback, since a
/// browser opening `localhost` would otherwise reach whatever holds it.
pub fn bind_loopback(port: u16) -> io::Result<Vec<TcpListener>> {
    if port != 0 && answers(port) {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("localhost:{port} is in use on this computer"),
        ));
    }
    // A port the system chose for IPv4 may be taken on IPv6; choose again then.
    for _ in 0..8 {
        let v4 = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).map_err(|error| {
            if error.kind() == io::ErrorKind::AddrInUse {
                io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("localhost:{port} is in use on this computer"),
                )
            } else {
                error
            }
        })?;
        let chosen = v4.local_addr()?.port();
        match TcpListener::bind((Ipv6Addr::LOCALHOST, chosen)) {
            Ok(v6) => return Ok(vec![v4, v6]),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::AddrNotAvailable | io::ErrorKind::Unsupported
                ) =>
            {
                return Ok(vec![v4]);
            }
            Err(error) if error.kind() == io::ErrorKind::AddrInUse && port == 0 => {}
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("localhost:{port} is in use on this computer"),
                ));
            }
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AddrInUse,
        "no free port on both loopbacks",
    ))
}

/// Whether something on this machine already answers on `port`, which a bind alone
/// does not always say: a wildcard listener lets a specific address bind on macOS.
fn answers(port: u16) -> bool {
    [
        SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        SocketAddr::from((Ipv6Addr::LOCALHOST, port)),
    ]
    .iter()
    .any(|address| TcpStream::connect_timeout(address, PROBE_TIMEOUT).is_ok())
}

/// How a GUI's Stream connection to one Device stands, for the Ports page's cards.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LinkStatus {
    /// Nothing needs it, or the terminal connection is down.
    Idle,
    Connected,
    /// Lost or failing; dialled again after a backoff. Says why the last attempt failed.
    Reconnecting(String),
    /// The Server refused it, an older one or a revoked Device; no more attempts until
    /// the person forwards again.
    Refused(String),
}

/// A GUI's Stream connection to one Device (ADR 0061): opened when the first forward or
/// transfer starts, closed when the last ends and with the terminal connection, and,
/// while forwards remain and the terminal connection is up, reopened by itself after a
/// loss: at once after the terminal connection's Bootstrap, else after 2 s, doubling to
/// 30 s. A refusal ends the retries.
pub struct StreamLink {
    shared: Arc<LinkShared>,
}

type Dial = dyn Fn() -> io::Result<StreamConnection> + Send + Sync;

struct LinkShared {
    /// The Device's name, for what a person reads.
    device: String,
    dial: Box<Dial>,
    on_change: Box<dyn Fn() + Send + Sync>,
    state: Mutex<LinkState>,
    wake: Condvar,
}

struct LinkState {
    users: usize,
    terminal_up: bool,
    connection: Option<Arc<StreamConnection>>,
    status: LinkStatus,
    worker: bool,
    /// A `retain` is dialling; the others wait for its result rather than dial too.
    dialing: bool,
    /// Bumped when a `retain`'s dial ends, with its error if it failed, so the callers
    /// that waited on it share the outcome.
    dials: u64,
    dial_error: Option<String>,
    /// Bumped whenever the terminal connection comes back, so a waiting retry runs now.
    nudge: u64,
}

impl StreamLink {
    /// `dial` opens the Stream connection to `device`; `on_change` runs whenever the
    /// status changes.
    pub fn new(
        device: impl Into<String>,
        dial: impl Fn() -> io::Result<StreamConnection> + Send + Sync + 'static,
        on_change: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self {
            shared: Arc::new(LinkShared {
                device: device.into(),
                dial: Box::new(dial),
                on_change: Box::new(on_change),
                state: Mutex::new(LinkState {
                    users: 0,
                    terminal_up: true,
                    connection: None,
                    status: LinkStatus::Idle,
                    worker: false,
                    dialing: false,
                    dials: 0,
                    dial_error: None,
                    nudge: 0,
                }),
                wake: Condvar::new(),
            }),
        }
    }

    pub fn status(&self) -> LinkStatus {
        self.shared.lock().status.clone()
    }

    /// One more forward or transfer needs the connection. The first dials here, on the
    /// caller's thread, so a Device that cannot be reached says so to the person at once;
    /// that failure leaves nothing held. Callers that arrive while it dials wait for it
    /// and share its outcome, so a batch of transfers logs in once.
    pub fn retain(&self) -> Result<(), String> {
        let mut state = self.shared.lock();
        while state.dialing {
            let dials = state.dials;
            while state.dials == dials {
                state = self
                    .shared
                    .wake
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            if state.users == 0
                && let Some(error) = &state.dial_error
            {
                return Err(error.clone());
            }
        }
        if state.users > 0 && state.connection.is_some() {
            state.users += 1;
            return Ok(());
        }
        if state.users > 0 {
            // Reconnecting or refused: forwarding again is the person asking anew.
            state.users += 1;
            state.status = LinkStatus::Reconnecting("connecting".into());
            state.nudge += 1;
            self.shared.wake.notify_all();
            return Ok(());
        }
        state.dialing = true;
        drop(state);
        let dialed = (self.shared.dial)().map_err(|error| describe(&self.shared.device, &error));
        let mut state = self.shared.lock();
        state.dialing = false;
        state.dials += 1;
        state.dial_error = dialed.as_ref().err().cloned();
        self.shared.wake.notify_all();
        let connection = dialed?;
        state.users += 1;
        state.connection = Some(Arc::new(connection));
        state.status = LinkStatus::Connected;
        if !state.worker {
            state.worker = true;
            let shared = Arc::clone(&self.shared);
            thread::Builder::new()
                .name("condr-stream-link".into())
                .spawn(move || shared.keep())
                .map_err(|error| error.to_string())?;
        }
        drop(state);
        (self.shared.on_change)();
        Ok(())
    }

    /// A forward or transfer ended; the last one closes the connection.
    pub fn release(&self) {
        let mut state = self.shared.lock();
        state.users = state.users.saturating_sub(1);
        if state.users == 0 {
            if let Some(connection) = state.connection.take() {
                connection.close();
            }
            state.status = LinkStatus::Idle;
            self.shared.wake.notify_all();
            drop(state);
            (self.shared.on_change)();
        }
    }

    /// The terminal connection was lost or came back with its Bootstrap.
    pub fn set_terminal_connected(&self, up: bool) {
        let mut state = self.shared.lock();
        state.terminal_up = up;
        if up {
            state.nudge += 1;
        } else if let Some(connection) = state.connection.take() {
            connection.close();
        }
        let changed = !up && LinkShared::set_status(&mut state, LinkStatus::Idle);
        self.shared.wake.notify_all();
        drop(state);
        if changed {
            (self.shared.on_change)();
        }
    }
}

impl StreamOpener for StreamLink {
    fn open(&self, target: StreamTarget) -> Result<Stream, StreamFailure> {
        let connection = self.shared.lock().connection.clone();
        match connection {
            Some(connection) if !connection.is_closed() => connection.open(target),
            _ => Err(StreamFailure {
                error: StreamError::Other,
                detail: "the Stream connection is reconnecting".into(),
            }),
        }
    }
}

impl Drop for StreamLink {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.users = 0;
        if let Some(connection) = state.connection.take() {
            connection.close();
        }
        self.shared.wake.notify_all();
    }
}

impl LinkShared {
    fn lock(&self) -> MutexGuard<'_, LinkState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether the status changed; the caller runs `on_change` once it let go of the lock,
    /// so a callback may read the status.
    fn set_status(state: &mut LinkState, status: LinkStatus) -> bool {
        let changed = state.status != status;
        state.status = status;
        changed
    }

    /// The worker: waits for the connection to end, then dials again while it is needed.
    fn keep(self: Arc<Self>) {
        let mut backoff = RETRY_FIRST;
        let mut immediate = false;
        loop {
            let connection = {
                let mut state = self.lock();
                loop {
                    if state.users == 0 {
                        state.worker = false;
                        return;
                    }
                    if let Some(connection) = &state.connection {
                        break Some(Arc::clone(connection));
                    }
                    if state.terminal_up && !matches!(state.status, LinkStatus::Refused(_)) {
                        break None;
                    }
                    let nudge = state.nudge;
                    state = self
                        .wake
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                    if state.nudge != nudge {
                        immediate = true;
                        if matches!(state.status, LinkStatus::Refused(_)) && state.users > 0 {
                            state.status = LinkStatus::Reconnecting("connecting".into());
                        }
                    }
                }
            };
            if let Some(connection) = connection {
                let reason = connection.wait_closed();
                let mut state = self.lock();
                if state
                    .connection
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &connection))
                {
                    state.connection = None;
                }
                let changed = state.users > 0
                    && state.terminal_up
                    && Self::set_status(&mut state, LinkStatus::Reconnecting(reason));
                drop(state);
                if changed {
                    (self.on_change)();
                }
                immediate = false;
                backoff = RETRY_FIRST;
                continue;
            }
            if !immediate {
                let mut state = self.lock();
                let nudge = state.nudge;
                let (next, _) = self
                    .wake
                    .wait_timeout_while(state, backoff, |state| {
                        state.nudge == nudge && state.users > 0 && state.terminal_up
                    })
                    .unwrap_or_else(PoisonError::into_inner);
                state = next;
                if state.users == 0 || !state.terminal_up {
                    continue;
                }
            }
            immediate = false;
            match (self.dial)() {
                Ok(connection) => {
                    let mut state = self.lock();
                    // A `retain` may have dialled meanwhile; keep the one in place.
                    if state.users == 0 || !state.terminal_up || state.connection.is_some() {
                        connection.close();
                        continue;
                    }
                    state.connection = Some(Arc::new(connection));
                    let changed = Self::set_status(&mut state, LinkStatus::Connected);
                    drop(state);
                    if changed {
                        (self.on_change)();
                    }
                    backoff = RETRY_FIRST;
                }
                Err(error) => {
                    let mut state = self.lock();
                    let status = if Refused::from_error(&error).is_some() {
                        LinkStatus::Refused(describe(&self.device, &error))
                    } else {
                        LinkStatus::Reconnecting(describe(&self.device, &error))
                    };
                    let changed = Self::set_status(&mut state, status);
                    drop(state);
                    if changed {
                        (self.on_change)();
                    }
                    backoff = (backoff * 2).min(RETRY_LONGEST);
                }
            }
        }
    }
}

/// What a person reads when a Stream connection to `device` cannot open: an older
/// Server's refusal of the handshake names the cause (ADR 0061).
pub fn describe(device: &str, error: &io::Error) -> String {
    match Refused::from_error(error) {
        Some(refused) if matches!(refused.refusal, condr_core::protocol::Refusal::Malformed(_)) => {
            format!(
                "{device} runs a Condr too old to forward ports or transfer files (build {}); update it",
                refused.server_build
            )
        }
        _ => error.to_string(),
    }
}

#[cfg(test)]
mod tests;
