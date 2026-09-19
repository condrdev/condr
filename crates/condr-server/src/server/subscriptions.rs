use super::*;

impl RuntimeState {
    pub(super) fn zoomed_panes(&self) -> Vec<PaneId> {
        self.session
            .workspaces()
            .iter()
            .flat_map(|workspace| workspace.tabs())
            .filter_map(|tab| tab.zoomed_pane_id())
            .collect()
    }

    /// The event every structural change publishes: the new Snapshot and zoom state, so
    /// clients apply it in place rather than re-fetching a Bootstrap.
    pub(super) fn layout_changed_event(&self) -> SessionEvent {
        SessionEvent::LayoutChanged {
            snapshot: self.session.snapshot(),
            zoomed_panes: self.zoomed_panes(),
        }
    }

    pub(super) fn publish_layout_change(
        &mut self,
        origin_client_id: u64,
        origin: &ClientWriter,
        request_id: u64,
        effect: LayoutEffect,
    ) -> bool {
        let LayoutEffect { result, event, .. } = effect;
        if self.publish_event(event, Some((origin_client_id, origin))) {
            return true;
        }
        queue_message(
            origin,
            ServerMessage::LayoutApplied {
                server_id: self.server_id,
                session_id: self.session_id,
                request_id,
                sequence: self.sequence,
                result,
            },
        )
    }

    pub(super) fn publish_background(&mut self, event: SessionEvent) {
        self.publish_event(event, None);
    }

    pub(super) fn publish_event(
        &mut self,
        event: SessionEvent,
        origin: Option<(u64, &ClientWriter)>,
    ) -> bool {
        self.sequence = self.sequence.saturating_add(1);
        self.events.push_back(SequencedEvent {
            sequence: self.sequence,
            event: event.clone(),
        });
        while self.events.len() > EVENT_HISTORY_LIMIT {
            self.events.pop_front();
        }
        let message = ServerMessage::Event {
            server_id: self.server_id,
            session_id: self.session_id,
            sequence: self.sequence,
            event,
        };
        let data = frame_message(&message).expect("Session event must fit a protocol frame");
        // The event is already committed; a lagged origin merely leaves the subscriber set
        // and recovers through its Bootstrap. Only a dead writer closes the connection.
        let origin_result = origin.map(|(_, writer)| writer.send_reliable(data.clone()));
        let origin_lost = origin_result.is_some_and(|result| result.is_err());
        self.subscribers.retain(|client_id, subscriber| {
            if origin.is_some_and(|(origin_client_id, _)| *client_id == origin_client_id) {
                !origin_lost
            } else if subscriber.bootstrap_pending {
                subscriber.deferred_reliable.push_back(data.clone());
                true
            } else {
                subscriber.writer.send_reliable(data.clone()).is_ok()
            }
        });
        origin_result == Some(Err(ReliableSendError::Disconnected))
    }

    /// Fans out the latest non-replayed clipboard state to every attached client.
    pub(super) fn broadcast_clipboard(&mut self, pane_id: PaneId, text: String) {
        let Ok(data) = frame_message(&ServerMessage::TerminalClipboard { pane_id, text }) else {
            return;
        };
        self.subscribers.retain(|_, subscriber| {
            if subscriber.bootstrap_pending {
                subscriber.deferred_clipboard = Some(data.clone());
                true
            } else {
                subscriber.writer.send_clipboard(data.clone()).is_ok()
            }
        });
    }

    pub(super) fn begin_bootstrap(&mut self, client_id: u64) {
        let Some(subscriber) = self.subscribers.get_mut(&client_id) else {
            return;
        };
        subscriber.writer.clear_render();
        // Clipboard state is neither in the Bootstrap nor in event history, so a copy that is
        // still unsent must survive the fence (ADR 0007); newer copies keep replacing it and
        // go out after the Bootstrap. Only lag recovery drops it (`resume_after_lag`).
        subscriber.pending_terminals.clear();
        subscriber.terminal_baselines.clear();
        subscriber.bootstrap_pending = true;
        subscriber.deferred_reliable.clear();
        subscriber.render_generation = subscriber.render_generation.wrapping_add(1);
    }

    pub(super) fn ensure_subscriber(&mut self, client_id: u64, writer: ClientWriter) -> bool {
        match self.subscribers.entry(client_id) {
            std::collections::hash_map::Entry::Occupied(_) => false,
            std::collections::hash_map::Entry::Vacant(entry) => {
                let pending_terminals = self.terminals.keys().copied().collect();
                entry.insert(ClientSubscriber {
                    writer,
                    terminal_baselines: std::collections::HashMap::new(),
                    pending_terminals,
                    render_generation: 1,
                    bootstrap_pending: false,
                    deferred_reliable: std::collections::VecDeque::new(),
                    deferred_clipboard: None,
                });
                true
            }
        }
    }

    pub(super) fn finish_bootstrap(
        &mut self,
        client_id: u64,
        framed: FramedBootstrap,
    ) -> Result<(), ReliableSendError> {
        let Some(subscriber) = self.subscribers.get_mut(&client_id) else {
            return Err(ReliableSendError::Disconnected);
        };
        subscriber.writer.clear_render();
        subscriber.terminal_baselines = framed.baselines.into_iter().collect();
        subscriber.writer.send_reliable_batch(framed.frames)?;
        while let Some(data) = subscriber.deferred_reliable.pop_front() {
            subscriber.writer.send_reliable(data)?;
        }
        if let Some(data) = subscriber.deferred_clipboard.take() {
            subscriber.writer.send_clipboard(data)?;
        }
        subscriber.bootstrap_pending = false;
        subscriber.render_generation = subscriber.render_generation.wrapping_add(1);
        Ok(())
    }

    pub(super) fn abort_bootstrap(&mut self, client_id: u64) {
        if let Some(subscriber) = self.subscribers.get_mut(&client_id) {
            subscriber.bootstrap_pending = false;
            subscriber.deferred_reliable.clear();
            if let Some(data) = subscriber.deferred_clipboard.take() {
                let _ = subscriber.writer.send_clipboard(data);
            }
            subscriber.pending_terminals.clear();
            subscriber.render_generation = subscriber.render_generation.wrapping_add(1);
        }
    }
}
