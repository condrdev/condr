//! The Stream connection (ADR 0061): one more connection to a Device, beside the
//! terminal connection, that multiplexes Streams, each an ordered, reliable byte stream
//! for one forwarded connection or one file transfer. Both ends run this engine: a
//! reader thread that never blocks on a consumer, a writer thread that takes the Streams
//! with data and credit in turn, and per-Stream windows whose credit returns only once
//! the consumer took the bytes.

use crate::connection::{Connector, ServerStream, handshake_exchange};
use condr_core::protocol::{
    ClientHandshake, FramingError, HEARTBEAT_INTERVAL, Hello, MAX_STREAM_CHUNK,
    MAX_STREAM_FRAME_SIZE, MAX_STREAMS, STREAM_WINDOW, StreamClientMessage, StreamError,
    StreamMessage, StreamServerMessage, StreamTarget, Welcome,
};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::io;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

/// How long either end hears nothing before it gives the connection up: the Client
/// pings when idle and the Server answers, so a live connection is never this quiet.
const SILENCE: Duration = Duration::from_secs(HEARTBEAT_INTERVAL.as_secs() * 3);

/// Why a Stream operation did not happen: the Stream was refused or reset, or the whole
/// Stream connection ended (`Other`, with its reason).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamFailure {
    pub error: StreamError,
    pub detail: String,
}

impl fmt::Display for StreamFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.detail.is_empty() {
            self.error.fmt(formatter)
        } else {
            write!(formatter, "{}: {}", self.error, self.detail)
        }
    }
}

impl std::error::Error for StreamFailure {}

impl From<StreamFailure> for io::Error {
    fn from(failure: StreamFailure) -> Self {
        io::Error::other(failure)
    }
}

type OnOpen = dyn Fn(Stream, StreamTarget) + Send + Sync;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Side {
    Client,
    Server,
}

struct Shared {
    side: Side,
    state: Mutex<State>,
    /// One condition for every change: the writer, blocked writes and blocked reads all
    /// re-check their own condition when woken.
    wake: Condvar,
    shutdown: Box<dyn Fn() + Send + Sync>,
    on_open: Option<Box<OnOpen>>,
}

#[derive(Default)]
struct State {
    streams: HashMap<u64, Entry>,
    /// The highest Stream number opened on this connection. A message for a lower number
    /// that is gone belongs to a closed Stream and is ignored; a higher one was never
    /// opened and is a protocol error.
    highest: u64,
    /// Frames that do not wait for credit, sent before any data.
    control: VecDeque<StreamMessage>,
    /// Streams with queued data or a queued `Finish`, taken in turn.
    ready: VecDeque<u64>,
    closed: Option<String>,
}

enum Outgoing {
    Data(Vec<u8>),
    Finish,
}

struct Entry {
    /// The peer's credit: this Stream's bytes may be sent up to here.
    limit: u64,
    sent: u64,
    outgoing: VecDeque<Outgoing>,
    queued: bool,
    finish_queued: bool,
    finish_sent: bool,
    inbound: VecDeque<Vec<u8>>,
    received: u64,
    consumed: u64,
    /// The credit this end last gave the peer.
    announced: u64,
    peer_finished: bool,
    /// The Client's answer to `Open`: `Some(size)` once opened.
    opened: Option<Option<u64>>,
    failure: Option<StreamFailure>,
    handle_alive: bool,
}

impl Entry {
    fn new() -> Self {
        Self {
            limit: STREAM_WINDOW,
            sent: 0,
            outgoing: VecDeque::new(),
            queued: false,
            finish_queued: false,
            finish_sent: false,
            inbound: VecDeque::new(),
            received: 0,
            consumed: 0,
            announced: STREAM_WINDOW,
            peer_finished: false,
            opened: None,
            failure: None,
            handle_alive: true,
        }
    }

    /// Nothing more can happen on this Stream once its handle is gone and its last
    /// queued frame left.
    fn finished(&self) -> bool {
        !self.handle_alive && self.outgoing.is_empty()
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Nothing panics while holding this lock; a poisoned one still holds valid state.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn close(&self, state: &mut State, reason: String) {
        if state.closed.is_none() {
            state.closed = Some(reason);
            (self.shutdown)();
        }
        self.wake.notify_all();
    }

    fn failure(state: &State, id: u64) -> Option<StreamFailure> {
        if let Some(reason) = &state.closed {
            return Some(StreamFailure {
                error: StreamError::Other,
                detail: reason.clone(),
            });
        }
        state
            .streams
            .get(&id)
            .and_then(|entry| entry.failure.clone())
    }

    fn remove_if_finished(state: &mut State, id: u64) {
        if state.streams.get(&id).is_some_and(Entry::finished) {
            state.streams.remove(&id);
        }
    }
}

/// One end of a Stream connection. Dropping it closes the connection and fails every
/// Stream on it.
pub struct StreamConnection {
    shared: Arc<Shared>,
}

impl StreamConnection {
    /// Dials `connector` for a Stream connection: the same transport and authentication as
    /// a terminal connection, with `ClientHandshake::Streams` in place of `Hello`.
    pub fn connect<C: Connector>(
        connector: &C,
        hello: impl Into<Hello>,
    ) -> io::Result<(Self, Welcome)> {
        let stream = connector.connect()?;
        let (stream, welcome) = handshake_exchange(stream, ClientHandshake::Streams(hello.into()))?;
        Ok((Self::start(stream, Side::Client, None)?, welcome))
    }

    /// Runs the Server's end on a connection whose handshake was `Streams`. `on_open`
    /// receives every Stream the Client opens with a target this build knows, and answers
    /// it with [`Stream::accept`] or [`Stream::refuse`]; it runs on the reader thread, so
    /// anything slow belongs on a thread of its own.
    pub fn serve<S: ServerStream>(
        stream: S,
        on_open: impl Fn(Stream, StreamTarget) + Send + Sync + 'static,
    ) -> io::Result<Self> {
        Self::start(stream, Side::Server, Some(Box::new(on_open)))
    }

    /// A Client end on a stream whose handshake already happened, for this crate's tests.
    #[cfg(test)]
    pub(crate) fn start_client_for_tests<S: ServerStream>(stream: S) -> io::Result<Self> {
        Self::start(stream, Side::Client, None)
    }

    fn start<S: ServerStream>(
        stream: S,
        side: Side,
        on_open: Option<Box<OnOpen>>,
    ) -> io::Result<Self> {
        stream.set_read_timeout(Some(SILENCE))?;
        // A stream is `Send` but not `Sync`; the lock lets any thread shut it down.
        let closer = Mutex::new(stream.try_clone()?);
        let writer = stream.try_clone()?;
        let shared = Arc::new(Shared {
            side,
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            shutdown: Box::new(move || {
                let _ = closer
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .shutdown();
            }),
            on_open,
        });
        let reader_shared = Arc::clone(&shared);
        thread::Builder::new()
            .name("condr-stream-reader".into())
            .spawn(move || read_loop(&reader_shared, stream))?;
        let writer_shared = Arc::clone(&shared);
        thread::Builder::new()
            .name("condr-stream-writer".into())
            .spawn(move || write_loop(&writer_shared, writer))?;
        Ok(Self { shared })
    }

    /// Opens a Stream. The Client may write at once, within the initial window; the
    /// Server's answer arrives through [`Stream::opened`].
    pub fn open(&self, target: StreamTarget) -> Result<Stream, StreamFailure> {
        assert_eq!(
            self.shared.side,
            Side::Client,
            "only the Client opens Streams"
        );
        let mut state = self.shared.lock();
        if let Some(reason) = &state.closed {
            return Err(StreamFailure {
                error: StreamError::Other,
                detail: reason.clone(),
            });
        }
        if state.streams.len() >= MAX_STREAMS {
            return Err(StreamFailure {
                error: StreamError::TooManyStreams,
                detail: String::new(),
            });
        }
        state.highest += 1;
        let id = state.highest;
        state.streams.insert(id, Entry::new());
        state
            .control
            .push_back(StreamMessage::Open { stream: id, target });
        self.shared.wake.notify_all();
        Ok(Stream {
            id,
            shared: Arc::clone(&self.shared),
        })
    }

    /// Blocks until the connection ends, and says why.
    pub fn wait_closed(&self) -> String {
        let mut state = self.shared.lock();
        loop {
            if let Some(reason) = &state.closed {
                return reason.clone();
            }
            state = self
                .shared
                .wake
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    pub fn is_closed(&self) -> bool {
        self.shared.lock().closed.is_some()
    }

    /// The Streams open on this connection.
    pub fn stream_count(&self) -> usize {
        self.shared.lock().streams.len()
    }

    /// Ends the connection and every Stream on it.
    pub fn close(&self) {
        let mut state = self.shared.lock();
        self.shared.close(&mut state, "closed".into());
    }
}

impl Drop for StreamConnection {
    fn drop(&mut self) {
        self.close();
    }
}

/// One Stream. Writes block while the peer's credit is used up; reads block until bytes
/// arrive or the peer finished. Dropping it before both directions finished resets it.
pub struct Stream {
    id: u64,
    shared: Arc<Shared>,
}

impl Stream {
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The Client waits for the Server's answer to `Open`: a download's size once opened.
    pub fn opened(&self) -> Result<Option<u64>, StreamFailure> {
        let mut state = self.shared.lock();
        loop {
            if let Some(entry) = state.streams.get(&self.id)
                && let Some(size) = entry.opened
            {
                return Ok(size);
            }
            if let Some(failure) = Shared::failure(&state, self.id) {
                return Err(failure);
            }
            state = self
                .shared
                .wake
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// The Server answers `Open` with success.
    pub fn accept(&self, size: Option<u64>) {
        let mut state = self.shared.lock();
        if state.closed.is_some() {
            return;
        }
        state.control.push_back(StreamMessage::Opened {
            stream: self.id,
            size,
        });
        self.shared.wake.notify_all();
    }

    /// The Server answers `Open` with a failure; the Stream ends.
    pub fn refuse(&self, error: StreamError, detail: impl Into<String>) {
        let detail = detail.into();
        let mut state = self.shared.lock();
        if state.closed.is_some() {
            return;
        }
        state.control.push_back(StreamMessage::OpenFailed {
            stream: self.id,
            error,
            detail: detail.clone(),
        });
        if let Some(entry) = state.streams.get_mut(&self.id) {
            entry.outgoing.clear();
            entry.failure = Some(StreamFailure { error, detail });
        }
        self.shared.wake.notify_all();
    }

    /// Sends `data`, waiting for credit as needed.
    pub fn write(&self, data: &[u8]) -> Result<(), StreamFailure> {
        let mut rest = data;
        while !rest.is_empty() {
            let mut state = self.shared.lock();
            let available = loop {
                if let Some(failure) = Shared::failure(&state, self.id) {
                    return Err(failure);
                }
                let entry = state
                    .streams
                    .get(&self.id)
                    .expect("a live handle has its entry");
                if entry.finish_queued {
                    return Err(StreamFailure {
                        error: StreamError::Other,
                        detail: "written after finish".into(),
                    });
                }
                let available = entry.limit.saturating_sub(entry.sent);
                if available > 0 {
                    break available;
                }
                state = self
                    .shared
                    .wake
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            };
            let take = rest
                .len()
                .min(MAX_STREAM_CHUNK)
                .min(usize::try_from(available).unwrap_or(usize::MAX));
            let entry = state.streams.get_mut(&self.id).expect("checked above");
            entry
                .outgoing
                .push_back(Outgoing::Data(rest[..take].to_vec()));
            entry.sent += take as u64;
            if !entry.queued {
                entry.queued = true;
                state.ready.push_back(self.id);
            }
            self.shared.wake.notify_all();
            rest = &rest[take..];
        }
        Ok(())
    }

    /// The next bytes from the peer, or `None` once the peer finished its direction.
    pub fn read(&self) -> Result<Option<Vec<u8>>, StreamFailure> {
        let mut state = self.shared.lock();
        loop {
            let entry = state
                .streams
                .get_mut(&self.id)
                .expect("a live handle has its entry");
            if let Some(chunk) = entry.inbound.pop_front() {
                entry.consumed += chunk.len() as u64;
                // Credit returns once a quarter window was consumed, so a sender never
                // waits on a consumer that keeps up and frames stay few.
                if !entry.peer_finished
                    && entry.consumed + STREAM_WINDOW >= entry.announced + STREAM_WINDOW / 4
                {
                    entry.announced = entry.consumed + STREAM_WINDOW;
                    let credit = StreamMessage::Credit {
                        stream: self.id,
                        max_offset: entry.announced,
                    };
                    state.control.push_back(credit);
                    self.shared.wake.notify_all();
                }
                return Ok(Some(chunk));
            }
            if entry.peer_finished {
                return Ok(None);
            }
            if let Some(failure) = Shared::failure(&state, self.id) {
                return Err(failure);
            }
            state = self
                .shared
                .wake
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Ends this direction once the data written before it is sent.
    pub fn finish(&self) {
        let mut state = self.shared.lock();
        let Some(entry) = state.streams.get_mut(&self.id) else {
            return;
        };
        if entry.finish_queued || entry.failure.is_some() {
            return;
        }
        entry.finish_queued = true;
        entry.outgoing.push_back(Outgoing::Finish);
        if !entry.queued {
            entry.queued = true;
            state.ready.push_back(self.id);
        }
        self.shared.wake.notify_all();
    }

    /// Ends both directions at once.
    pub fn reset(&self, error: StreamError, detail: impl Into<String>) {
        let detail = detail.into();
        let mut state = self.shared.lock();
        reset_entry(&self.shared, &mut state, self.id, error, detail);
    }
}

fn reset_entry(shared: &Shared, state: &mut State, id: u64, error: StreamError, detail: String) {
    let Some(entry) = state.streams.get_mut(&id) else {
        return;
    };
    if entry.failure.is_some() {
        return;
    }
    entry.outgoing.clear();
    entry.failure = Some(StreamFailure {
        error,
        detail: detail.clone(),
    });
    if state.closed.is_none() {
        state.control.push_back(StreamMessage::Reset {
            stream: id,
            error,
            detail,
        });
    }
    shared.wake.notify_all();
}

impl Drop for Stream {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        let Some(entry) = state.streams.get(&self.id) else {
            return;
        };
        let complete = entry.finish_queued && entry.peer_finished;
        if !complete && entry.failure.is_none() {
            reset_entry(
                &self.shared,
                &mut state,
                self.id,
                StreamError::Cancelled,
                String::new(),
            );
        }
        if let Some(entry) = state.streams.get_mut(&self.id) {
            entry.handle_alive = false;
            entry.inbound.clear();
        }
        Shared::remove_if_finished(&mut state, self.id);
        self.shared.wake.notify_all();
    }
}

fn write_loop<S: ServerStream>(shared: &Shared, mut stream: S) {
    loop {
        let message = {
            let mut state = shared.lock();
            loop {
                if state.closed.is_some() {
                    return;
                }
                if let Some(message) = state.control.pop_front() {
                    break message;
                }
                if let Some(message) = next_data(&mut state) {
                    break message;
                }
                let (next, timeout) = shared
                    .wake
                    .wait_timeout(state, HEARTBEAT_INTERVAL)
                    .unwrap_or_else(PoisonError::into_inner);
                state = next;
                if timeout.timed_out() && shared.side == Side::Client {
                    break StreamMessage::Ping;
                }
            }
        };
        let result = match shared.side {
            Side::Client => condr_core::protocol::write_message_with_limit(
                &mut stream,
                &StreamClientMessage(message),
                MAX_STREAM_FRAME_SIZE,
            ),
            Side::Server => condr_core::protocol::write_message_with_limit(
                &mut stream,
                &StreamServerMessage(message),
                MAX_STREAM_FRAME_SIZE,
            ),
        };
        if let Err(error) = result {
            let mut state = shared.lock();
            shared.close(&mut state, format!("write failed: {error}"));
            return;
        }
    }
}

/// One item from the next ready Stream, which goes to the back of the line if it has more.
fn next_data(state: &mut State) -> Option<StreamMessage> {
    while let Some(id) = state.ready.pop_front() {
        let Some(entry) = state.streams.get_mut(&id) else {
            continue;
        };
        let Some(item) = entry.outgoing.pop_front() else {
            entry.queued = false;
            continue;
        };
        if entry.outgoing.is_empty() {
            entry.queued = false;
        } else {
            state.ready.push_back(id);
        }
        let message = match item {
            Outgoing::Data(data) => StreamMessage::Data { stream: id, data },
            Outgoing::Finish => {
                entry.finish_sent = true;
                StreamMessage::Finish { stream: id }
            }
        };
        Shared::remove_if_finished(state, id);
        return Some(message);
    }
    None
}

fn read_loop<S: ServerStream>(shared: &Arc<Shared>, stream: S) {
    let mut reader = io::BufReader::new(stream);
    loop {
        let message =
            match shared.side {
                Side::Client => condr_core::protocol::read_message_with_limit::<
                    _,
                    StreamServerMessage,
                >(&mut reader, MAX_STREAM_FRAME_SIZE)
                .map(|(message, _)| message.0),
                Side::Server => condr_core::protocol::read_message_with_limit::<
                    _,
                    StreamClientMessage,
                >(&mut reader, MAX_STREAM_FRAME_SIZE)
                .map(|(message, _)| message.0),
            };
        let message = match message {
            Ok(message) => message,
            Err(error) => {
                let reason = match error {
                    FramingError::UnexpectedEof => "the connection closed".to_owned(),
                    FramingError::Io(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                        ) =>
                    {
                        "the device stopped answering".to_owned()
                    }
                    error => error.to_string(),
                };
                let mut state = shared.lock();
                shared.close(&mut state, reason);
                return;
            }
        };
        if let Err(reason) = receive(shared, message) {
            tracing::warn!("closing the Stream connection: {reason}");
            let mut state = shared.lock();
            shared.close(&mut state, format!("protocol error: {reason}"));
            return;
        }
    }
}

/// Applies one frame from the peer. An error is a protocol violation.
fn receive(shared: &Arc<Shared>, message: StreamMessage) -> Result<(), String> {
    let mut state = shared.lock();
    match (shared.side, message) {
        (_, StreamMessage::Unknown) | (Side::Client, StreamMessage::Pong) => {}
        (Side::Server, StreamMessage::Ping) => {
            state.control.push_back(StreamMessage::Pong);
            shared.wake.notify_all();
        }
        (Side::Server, StreamMessage::Open { stream, target }) => {
            if stream <= state.highest {
                return Err(format!("stream {stream} was opened before"));
            }
            state.highest = stream;
            let refusal = match target {
                StreamTarget::Unknown => Some(StreamError::Unsupported),
                _ if state.streams.len() >= MAX_STREAMS => Some(StreamError::TooManyStreams),
                _ => None,
            };
            if let Some(error) = refusal {
                state.control.push_back(StreamMessage::OpenFailed {
                    stream,
                    error,
                    detail: String::new(),
                });
                shared.wake.notify_all();
                return Ok(());
            }
            state.streams.insert(stream, Entry::new());
            drop(state);
            let handle = Stream {
                id: stream,
                shared: Arc::clone(shared),
            };
            if let Some(on_open) = &shared.on_open {
                on_open(handle, target);
            }
        }
        (Side::Client, StreamMessage::Opened { stream, size }) => {
            if let Some(entry) = live_entry(&mut state, stream)? {
                entry.opened = Some(size);
                shared.wake.notify_all();
            }
        }
        (
            Side::Client,
            StreamMessage::OpenFailed {
                stream,
                error,
                detail,
            },
        ) => {
            if let Some(entry) = live_entry(&mut state, stream)? {
                entry.outgoing.clear();
                entry.failure.get_or_insert(StreamFailure { error, detail });
                Shared::remove_if_finished(&mut state, stream);
                shared.wake.notify_all();
            }
        }
        (_, StreamMessage::Data { stream, data }) => {
            if let Some(entry) = live_entry(&mut state, stream)? {
                if entry.failure.is_some() || entry.peer_finished {
                    return Ok(());
                }
                let end = entry.received + data.len() as u64;
                if end > entry.announced {
                    return Err(format!("stream {stream} sent beyond its credit"));
                }
                entry.received = end;
                if entry.handle_alive {
                    entry.inbound.push_back(data);
                }
                shared.wake.notify_all();
            }
        }
        (_, StreamMessage::Credit { stream, max_offset }) => {
            if let Some(entry) = live_entry(&mut state, stream)? {
                entry.limit = entry.limit.max(max_offset);
                shared.wake.notify_all();
            }
        }
        (_, StreamMessage::Finish { stream }) => {
            if let Some(entry) = live_entry(&mut state, stream)? {
                entry.peer_finished = true;
                shared.wake.notify_all();
            }
        }
        (
            _,
            StreamMessage::Reset {
                stream,
                error,
                detail,
            },
        ) => {
            if let Some(entry) = live_entry(&mut state, stream)? {
                entry.outgoing.clear();
                entry.failure.get_or_insert(StreamFailure { error, detail });
                Shared::remove_if_finished(&mut state, stream);
                shared.wake.notify_all();
            }
        }
        (side, message) => return Err(format!("{side:?} received {message:?}")),
    }
    Ok(())
}

/// The entry for `stream`, `None` for a Stream that closed, an error for one never opened.
fn live_entry(state: &mut State, stream: u64) -> Result<Option<&mut Entry>, String> {
    if stream > state.highest || stream == 0 {
        return Err(format!("stream {stream} was never opened"));
    }
    Ok(state.streams.get_mut(&stream))
}

#[cfg(test)]
mod tests;
