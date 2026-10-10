use super::*;

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

impl From<condr_client::Incoming> for Incoming {
    fn from(incoming: condr_client::Incoming) -> Self {
        match incoming {
            condr_client::Incoming::Bootstrap(bootstrap) => Self::Bootstrap(bootstrap),
            condr_client::Incoming::Message(message) => Self::Message(message),
            condr_client::Incoming::VisualReady(generation) => Self::VisualReady(generation),
            condr_client::Incoming::TerminalResync => Self::TerminalResync,
            condr_client::Incoming::Disconnected(reason) => Self::Disconnected(reason),
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
        let (incoming_tx, incoming_rx) = async_channel::bounded(SERVER_EVENT_BUFFER_CAPACITY);
        let deliver = {
            let incoming_tx = incoming_tx.clone();
            move |incoming: condr_client::Incoming| {
                incoming_tx
                    .send_blocking(Incoming::from(incoming))
                    .map_err(drop)
            }
        };
        let prepare_events = incoming_tx.clone();
        // A clipboard image is converted on the writer thread, off the UI: one that cannot
        // be sent becomes an error toast, and its "Pasting image…" indicator comes down.
        let prepare = move |message: &mut ClientMessage| {
            let ClientMessage::PasteImage {
                pane_id,
                format,
                bytes,
                ..
            } = message
            else {
                return Ok(true);
            };
            match super::terminal_input::prepare_clipboard_image(format, bytes) {
                Ok(()) => Ok(true),
                Err(message) => {
                    let closed = prepare_events
                        .send_blocking(Incoming::Message(ServerMessage::Error { message }))
                        .is_err()
                        || prepare_events
                            .send_blocking(Incoming::ImageSent(*pane_id))
                            .is_err();
                    if closed { Err(()) } else { Ok(false) }
                }
            }
        };
        let written_events = incoming_tx;
        let written = move |message: &ClientMessage| {
            // Nothing applies what comes back any more: the connection is over, and
            // dropping this half of the stream closes it, which a local pipe's shutdown
            // does not. The next send then finds the Client disconnected.
            if written_events.is_closed() {
                return Err(());
            }
            // ponytail: "sent" is the socket accepting the last byte, not the Server
            // pasting the path; add a Server ack if staging ever gets slow.
            if let ClientMessage::PasteImage { pane_id, .. } = message {
                written_events
                    .send_blocking(Incoming::ImageSent(*pane_id))
                    .map_err(drop)?;
            }
            Ok(())
        };
        let condr_client::ConnectionIo {
            outgoing,
            visual: visual_slot,
        } = condr_client::ConnectionIo::start(
            connection.into_stream(),
            initial_server_id,
            initial_session_id,
            deliver,
            prepare,
            written,
        )?;

        // Applies everything the Server sends in the window it was spawned in, the one that
        // owns this connection. `WeakEntity::update_in` would look up the window that last
        // rendered `Condr` instead, and the Settings window renders it too: once Settings
        // closed, that lookup failed until the main window drew again.
        let incoming_task = cx.spawn_in(window, async move |owner, cx| {
            while let Ok(first) = incoming_rx.recv().await {
                let mut incoming = Vec::with_capacity(SERVER_EVENT_BUFFER_CAPACITY.min(16));
                incoming.push(first);
                while let Ok(next) = incoming_rx.try_recv() {
                    incoming.push(next);
                }
                let applied = cx.update(|window, cx| {
                    owner.update(cx, |this, cx| {
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
                });
                // Only the window or `Condr` going away fails this. Ending here ends the
                // connection: the writer closes the stream behind it.
                if let Err(error) = applied.and_then(|applied| applied) {
                    tracing::warn!(
                        connection = key,
                        "stopped applying the device's messages: {error:#}"
                    );
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

pub(super) fn clear_pending_sizes_for_bootstrap(
    pending_sizes: &mut HashMap<(ConnectionKey, PaneId), TerminalSize>,
    key: ConnectionKey,
) {
    pending_sizes.retain(|(connection_key, _), _| *connection_key != key);
}
