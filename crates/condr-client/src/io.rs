//! A connection's reader and writer threads, shared by every Client that keeps one open
//! (ADR 0039). The reader turns the Server's stream into [`Incoming`] in order, except
//! terminal frames, which merge into the one [`TerminalVisualSlot`] (ADR 0004); the writer
//! sends what its owner queues and a `Ping` when the connection has been idle (ADR 0036).

use crate::ServerStream;
use crate::frames::{
    TerminalVisualSlot, assemble_terminal_frame_chunk, enforce_terminal_chunk_reliable_fence,
    read_bootstrap_batches, terminal_chunk_identity_matches,
};
use condr_core::protocol::{
    ClientMessage, ServerId, ServerMessage, SessionBootstrap, SessionId, TerminalFrameBatch,
    UnknownMessage,
};
use std::io;
use std::sync::{Arc, mpsc};
use std::thread;

/// What the reader hands its owner, in the order the Server sent it.
pub enum Incoming {
    Bootstrap(SessionBootstrap),
    Message(ServerMessage),
    /// The visual slot holds frames of this generation; take them from it.
    VisualReady(u64),
    /// Frames were lost or could not apply: the owner asks for one new Bootstrap.
    TerminalResync,
    Disconnected(String),
}

/// The handles an owner keeps to a running connection.
pub struct ConnectionIo {
    pub outgoing: mpsc::Sender<ClientMessage>,
    pub visual: Arc<TerminalVisualSlot>,
}

impl ConnectionIo {
    /// Starts both threads on `stream`, whose Server and Session are the ones its
    /// handshake named. `deliver` receives what the reader reads and returns `Err` once
    /// nothing applies it any more. Before each message the writer calls `prepare`, which
    /// may rewrite it or drop it (`Ok(false)`); after the stream took one it calls
    /// `written`. Either returning `Err` ends the writer, and closing the stream behind it
    /// ends the connection.
    pub fn start<S: ServerStream>(
        stream: S,
        initial_server_id: ServerId,
        initial_session_id: SessionId,
        deliver: impl Fn(Incoming) -> Result<(), ()> + Clone + Send + 'static,
        mut prepare: impl FnMut(&mut ClientMessage) -> Result<bool, ()> + Send + 'static,
        mut written: impl FnMut(&ClientMessage) -> Result<(), ()> + Send + 'static,
    ) -> io::Result<Self> {
        let mut writer = stream.try_clone()?;
        // Buffered here, where nothing hands the stream on any more (ADR 0028).
        let mut reader = io::BufReader::new(stream);
        let (outgoing, outgoing_rx) = mpsc::channel();
        let visual = Arc::new(TerminalVisualSlot::default());
        let writer_deliver = deliver.clone();
        thread::Builder::new()
            .name("condr-client-writer".into())
            .spawn(move || {
                loop {
                    let mut message =
                        match outgoing_rx.recv_timeout(condr_core::protocol::HEARTBEAT_INTERVAL) {
                            Ok(message) => message,
                            // Idle, not gone: the Server drops a subscriber that stays silent
                            // (ADR 0036). Here, not on the owner's thread, so a stalled frame
                            // cannot cost the connection.
                            Err(mpsc::RecvTimeoutError::Timeout) => ClientMessage::Ping {
                                server_id: initial_server_id,
                                nonce: 0,
                            },
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        };
                    match prepare(&mut message) {
                        Ok(true) => {}
                        Ok(false) => continue,
                        Err(()) => break,
                    }
                    if let Err(error) =
                        condr_core::protocol::write_client_message(&mut writer, &message)
                    {
                        let _ = writer_deliver(Incoming::Disconnected(disconnect_reason(&error)));
                        break;
                    }
                    if written(&message).is_err() {
                        break;
                    }
                }
                let _ = writer.shutdown();
            })?;
        let reader_visual = Arc::clone(&visual);
        thread::Builder::new()
            .name("condr-client-reader".into())
            .spawn(move || {
                let visual = reader_visual;
                let mut server_id = initial_server_id;
                let mut session_id = initial_session_id;
                let mut resync_pending = false;
                let mut terminal_chunk_assembly = None;
                loop {
                    let message = match condr_core::protocol::read_message(&mut reader) {
                        Ok(message) => message,
                        Err(error) => {
                            let _ = deliver(Incoming::Disconnected(disconnect_reason(&error)));
                            break;
                        }
                    };
                    if let Err(error) = enforce_terminal_chunk_reliable_fence(
                        &mut terminal_chunk_assembly,
                        &message,
                    ) {
                        match &message {
                            ServerMessage::Bootstrap(_) | ServerMessage::ServerStopping => {}
                            ServerMessage::BootstrapBatch(_) => {
                                let _ = deliver(Incoming::Disconnected(error));
                                break;
                            }
                            _ => {
                                if request_terminal_resync(&visual, &deliver, &mut resync_pending)
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        }
                    }
                    match message {
                        // A frame this build cannot read: the Server already counts it as
                        // delivered, so the next delta would not apply. Drop what is
                        // pending and resynchronize, then let the UI count it (ADR 0028).
                        message @ ServerMessage::Unknown(UnknownMessage::TerminalFrame) => {
                            terminal_chunk_assembly = None;
                            if request_terminal_resync(&visual, &deliver, &mut resync_pending)
                                .is_err()
                                || deliver(Incoming::Message(message)).is_err()
                            {
                                break;
                            }
                        }
                        ServerMessage::TerminalFrame(batch) => {
                            if resync_pending {
                                continue;
                            }
                            if terminal_chunk_assembly.is_some() {
                                terminal_chunk_assembly = None;
                                if request_terminal_resync(&visual, &deliver, &mut resync_pending)
                                    .is_err()
                                {
                                    break;
                                }
                                continue;
                            }
                            if batch.server_id != server_id || batch.session_id != session_id {
                                continue;
                            }
                            if publish_terminal_batch(&visual, &deliver, &mut resync_pending, batch)
                                .is_err()
                            {
                                break;
                            }
                        }
                        ServerMessage::TerminalFrameChunk(chunk) => {
                            if resync_pending {
                                continue;
                            }
                            match terminal_chunk_identity_matches(
                                &mut terminal_chunk_assembly,
                                &chunk,
                                server_id,
                                session_id,
                            ) {
                                Ok(true) => {}
                                Ok(false) => continue,
                                Err(_) => {
                                    if request_terminal_resync(
                                        &visual,
                                        &deliver,
                                        &mut resync_pending,
                                    )
                                    .is_err()
                                    {
                                        break;
                                    }
                                    continue;
                                }
                            }
                            match assemble_terminal_frame_chunk(&mut terminal_chunk_assembly, chunk)
                            {
                                Ok(Some(pane)) => {
                                    if publish_terminal_batch(
                                        &visual,
                                        &deliver,
                                        &mut resync_pending,
                                        TerminalFrameBatch {
                                            server_id,
                                            session_id,
                                            panes: vec![pane],
                                        },
                                    )
                                    .is_err()
                                    {
                                        break;
                                    }
                                }
                                Ok(None) => {}
                                Err(_) => {
                                    terminal_chunk_assembly = None;
                                    if request_terminal_resync(
                                        &visual,
                                        &deliver,
                                        &mut resync_pending,
                                    )
                                    .is_err()
                                    {
                                        break;
                                    }
                                }
                            }
                        }
                        ServerMessage::Bootstrap(header) => {
                            let bootstrap = match read_bootstrap_batches(&mut reader, header) {
                                Ok(bootstrap) => bootstrap,
                                Err(error) => {
                                    let _ = deliver(Incoming::Disconnected(error));
                                    break;
                                }
                            };
                            server_id = bootstrap.server_id;
                            session_id = bootstrap.session_id;
                            resync_pending = false;
                            terminal_chunk_assembly = None;
                            visual.advance();
                            if deliver(Incoming::Bootstrap(bootstrap)).is_err() {
                                break;
                            }
                        }
                        ServerMessage::BootstrapBatch(_) => {
                            let _ = deliver(Incoming::Disconnected(
                                "unexpected Bootstrap batch without a header".into(),
                            ));
                            break;
                        }
                        message => {
                            if deliver(Incoming::Message(message)).is_err() {
                                break;
                            }
                        }
                    }
                }
                let _ = reader.get_ref().shutdown();
            })?;
        Ok(Self { outgoing, visual })
    }
}

/// Why the connection ended, for the sidebar. A closed socket is the normal way a
/// Server goes away and is said plainly; only genuine protocol faults keep their detail.
pub fn disconnect_reason(error: &condr_core::protocol::FramingError) -> String {
    use condr_core::protocol::FramingError;
    match error {
        FramingError::UnexpectedEof => "the device closed the connection".into(),
        FramingError::Io(error) => match error.kind() {
            std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::NotConnected => "the device closed the connection".into(),
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                "the device stopped answering".into()
            }
            std::io::ErrorKind::PermissionDenied => {
                format!("the device refused this connection: {error}")
            }
            _ => format!("connection error: {error}"),
        },
        FramingError::Oversized { .. } | FramingError::Codec(_) => {
            format!("protocol error: {error}")
        }
    }
}

fn publish_terminal_batch(
    visual_slot: &TerminalVisualSlot,
    deliver: &impl Fn(Incoming) -> Result<(), ()>,
    resync_pending: &mut bool,
    batch: TerminalFrameBatch,
) -> Result<(), ()> {
    match visual_slot.publish(batch) {
        Ok(Some(generation)) => deliver(Incoming::VisualReady(generation)),
        Ok(None) => Ok(()),
        Err(()) => request_terminal_resync(visual_slot, deliver, resync_pending),
    }
}

fn request_terminal_resync(
    visual_slot: &TerminalVisualSlot,
    deliver: &impl Fn(Incoming) -> Result<(), ()>,
    resync_pending: &mut bool,
) -> Result<(), ()> {
    visual_slot.advance();
    *resync_pending = true;
    deliver(Incoming::TerminalResync)
}
