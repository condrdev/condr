mod terminal_frames;

use super::*;

pub(super) use terminal_frames::*;

pub(super) enum Incoming {
    Bootstrap(SessionBootstrap),
    Message(ServerMessage),
    VisualReady(u64),
    TerminalResync,
    /// A `PasteImage` for this Pane has left the writer, or failed before it could: the
    /// "Pasting image…" indicator comes down. Client-local, not a Server reply.
    ImageSent(PaneId),
    Disconnected(String),
}

#[derive(Default)]
pub(super) struct IncomingEffect {
    pub(super) rebuild: bool,
    pub(super) rebuild_active: bool,
    pub(super) notify: bool,
}

pub(super) struct ClientIo {
    pub(super) outgoing: mpsc::Sender<ClientMessage>,
    pub(super) _incoming_task: Task<()>,
}

/// Why the connection ended, for the sidebar. A closed socket is the normal way a
/// Server goes away and is said plainly; only genuine protocol faults keep their detail.
fn disconnect_reason(error: &condr_core::protocol::FramingError) -> String {
    use condr_core::protocol::FramingError;
    match error {
        FramingError::UnexpectedEof => "the Server closed the connection".into(),
        FramingError::Io(error) => match error.kind() {
            std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::NotConnected => "the Server closed the connection".into(),
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                "the Server stopped answering".into()
            }
            std::io::ErrorKind::PermissionDenied => {
                format!("the Server refused this device: {error}")
            }
            _ => format!("connection error: {error}"),
        },
        FramingError::Oversized { .. } | FramingError::Codec(_) => {
            format!("protocol error: {error}")
        }
    }
}

impl ClientIo {
    pub(super) fn start(
        connection: ClientConnection,
        key: ConnectionKey,
        connection_generation: u64,
        initial_server_id: ServerId,
        initial_session_id: SessionId,
        window: &Window,
        cx: &Context<Condr>,
    ) -> std::io::Result<Self> {
        let mut reader = connection.into_stream();
        let mut writer = reader.try_clone()?;
        let (outgoing, outgoing_rx) = mpsc::channel();
        let (incoming_tx, incoming_rx) = async_channel::bounded(SERVER_EVENT_BUFFER_CAPACITY);
        let writer_events = incoming_tx.clone();
        let visual_slot = Arc::new(TerminalVisualSlot::default());

        thread::Builder::new()
            .name("condr-client-writer".into())
            .spawn(move || {
                while let Ok(mut message) = outgoing_rx.recv() {
                    let image_pane = match &mut message {
                        ClientMessage::PasteImage {
                            pane_id,
                            format,
                            bytes,
                            ..
                        } => {
                            if let Err(message) =
                                super::terminal_input::prepare_clipboard_image(format, bytes)
                            {
                                if writer_events
                                    .send_blocking(Incoming::Message(ServerMessage::Error {
                                        message,
                                    }))
                                    .is_err()
                                    || writer_events
                                        .send_blocking(Incoming::ImageSent(*pane_id))
                                        .is_err()
                                {
                                    break;
                                }
                                continue;
                            }
                            Some(*pane_id)
                        }
                        _ => None,
                    };
                    if let Err(error) =
                        condr_core::protocol::write_client_message(&mut writer, &message)
                    {
                        let _ = writer_events
                            .send_blocking(Incoming::Disconnected(disconnect_reason(&error)));
                        break;
                    }
                    // ponytail: "sent" is the socket accepting the last byte, not the Server
                    // pasting the path; add a Server ack if staging ever gets slow.
                    if let Some(pane_id) = image_pane
                        && writer_events
                            .send_blocking(Incoming::ImageSent(pane_id))
                            .is_err()
                    {
                        break;
                    }
                }
                let _ = writer.shutdown();
            })?;
        let reader_visual_slot = Arc::clone(&visual_slot);
        thread::Builder::new()
            .name("condr-client-reader".into())
            .spawn(move || {
                let mut server_id = initial_server_id;
                let mut session_id = initial_session_id;
                let mut resync_pending = false;
                let mut terminal_chunk_assembly = None;
                loop {
                    let message = match condr_core::protocol::read_message(&mut reader) {
                        Ok(message) => message,
                        Err(error) => {
                            let _ = incoming_tx
                                .send_blocking(Incoming::Disconnected(disconnect_reason(&error)));
                            break;
                        }
                    };
                    if let Err(error) = enforce_terminal_chunk_reliable_fence(
                        &mut terminal_chunk_assembly,
                        &message,
                    ) {
                        match &message {
                            ServerMessage::Bootstrap(_) | ServerMessage::ServerStopping => {}
                            ServerMessage::Welcome { .. } | ServerMessage::BootstrapBatch(_) => {
                                let _ = incoming_tx.send_blocking(Incoming::Disconnected(error));
                                break;
                            }
                            _ => {
                                if request_terminal_resync(
                                    &reader_visual_slot,
                                    &incoming_tx,
                                    &mut resync_pending,
                                )
                                .is_err()
                                {
                                    break;
                                }
                            }
                        }
                    }
                    match message {
                        ServerMessage::TerminalFrame(batch) => {
                            if resync_pending {
                                continue;
                            }
                            if terminal_chunk_assembly.is_some() {
                                terminal_chunk_assembly = None;
                                if request_terminal_resync(
                                    &reader_visual_slot,
                                    &incoming_tx,
                                    &mut resync_pending,
                                )
                                .is_err()
                                {
                                    break;
                                }
                                continue;
                            }
                            if batch.server_id != server_id || batch.session_id != session_id {
                                continue;
                            }
                            if publish_terminal_batch(
                                &reader_visual_slot,
                                &incoming_tx,
                                &mut resync_pending,
                                batch,
                            )
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
                                        &reader_visual_slot,
                                        &incoming_tx,
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
                                        &reader_visual_slot,
                                        &incoming_tx,
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
                                        &reader_visual_slot,
                                        &incoming_tx,
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
                                    let _ =
                                        incoming_tx.send_blocking(Incoming::Disconnected(error));
                                    break;
                                }
                            };
                            server_id = bootstrap.server_id;
                            session_id = bootstrap.session_id;
                            resync_pending = false;
                            terminal_chunk_assembly = None;
                            reader_visual_slot.advance();
                            if incoming_tx
                                .send_blocking(Incoming::Bootstrap(bootstrap))
                                .is_err()
                            {
                                break;
                            }
                        }
                        ServerMessage::BootstrapBatch(_) => {
                            let _ = incoming_tx.send_blocking(Incoming::Disconnected(
                                "unexpected Bootstrap batch without a header".into(),
                            ));
                            break;
                        }
                        message => {
                            if incoming_tx
                                .send_blocking(Incoming::Message(message))
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                }
                let _ = reader.shutdown();
            })?;

        let incoming_task = cx.spawn_in(window, async move |owner, cx| {
            while let Ok(first) = incoming_rx.recv().await {
                let mut incoming = Vec::with_capacity(SERVER_EVENT_BUFFER_CAPACITY.min(16));
                incoming.push(first);
                while let Ok(next) = incoming_rx.try_recv() {
                    incoming.push(next);
                }
                if owner
                    .update_in(cx, |this, window, cx| {
                        let mut effect = IncomingEffect::default();
                        for incoming in incoming {
                            let incoming = match incoming {
                                Incoming::VisualReady(generation) => {
                                    let Some(batch) = visual_slot.take(generation) else {
                                        continue;
                                    };
                                    Incoming::Message(ServerMessage::TerminalFrame(batch))
                                }
                                incoming => incoming,
                            };
                            let next =
                                this.handle_incoming(key, connection_generation, incoming, cx);
                            effect.rebuild |= next.rebuild;
                            effect.rebuild_active |= next.rebuild_active;
                            effect.notify |= next.notify;
                        }
                        if effect.rebuild_active
                            || (effect.rebuild && key == this.active_connection)
                        {
                            this.rebuild_dock(window, cx);
                        }
                        this.sync_terminal_focus(window, cx);
                        if effect.notify || effect.rebuild || effect.rebuild_active {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        Ok(Self {
            outgoing,
            _incoming_task: incoming_task,
        })
    }
}
