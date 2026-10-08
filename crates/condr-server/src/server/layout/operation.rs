use super::*;

/// A connection admits one external Layout operation at a time. The operation owns its
/// lifecycle guard and finishes even if the requesting connection leaves.
#[derive(Clone)]
pub(in crate::server) struct ClientLayouts {
    client_id: u64,
    state: Arc<Mutex<RuntimeState>>,
    lifecycle: Arc<ServerLifecycle>,
    outbound: ClientWriter,
    external_running: Arc<Mutex<bool>>,
}

#[derive(Clone, Copy)]
struct LayoutRequest {
    server_id: ServerId,
    session_id: SessionId,
    request_id: u64,
}

impl LayoutRequest {
    fn reject(self, reason: impl Into<String>) -> ServerMessage {
        ServerMessage::LayoutRejected {
            server_id: self.server_id,
            session_id: self.session_id,
            request_id: self.request_id,
            reason: reason.into(),
        }
    }

    fn validate(self, state: &RuntimeState, stopping: bool) -> Result<(), String> {
        layout_authority_error(state, self.server_id, self.session_id, stopping)
            .map_or(Ok(()), |reason| Err(reason.into()))
    }
}

impl ClientLayouts {
    pub(in crate::server) fn new(
        client_id: u64,
        state: Arc<Mutex<RuntimeState>>,
        lifecycle: Arc<ServerLifecycle>,
        outbound: ClientWriter,
    ) -> Self {
        Self {
            client_id,
            state,
            lifecycle,
            outbound,
            external_running: Arc::new(Mutex::new(false)),
        }
    }

    pub(in crate::server) fn handle(
        &self,
        server_id: ServerId,
        session_id: SessionId,
        request_id: u64,
        command: LayoutCommand,
    ) -> bool {
        let mut running = self.external_running.lock().expect("layout lock poisoned");
        let state = self.state.lock().expect("server state lock poisoned");
        let request = LayoutRequest {
            server_id: state.server_id,
            session_id: state.session_id,
            request_id,
        };
        if let Some(reason) =
            layout_authority_error(&state, server_id, session_id, self.lifecycle.is_stopping())
        {
            return queue_message(&self.outbound, request.reject(reason));
        }
        if *running {
            return queue_message(
                &self.outbound,
                request.reject("another layout operation is in progress on this connection"),
            );
        }
        let plan = match external_layout_plan(&state, &command) {
            Ok(plan) => plan,
            Err(reason) => return queue_message(&self.outbound, request.reject(reason)),
        };
        let launch = state.shell_launch();
        drop(state);
        if let Some(plan) = plan {
            let Some(operation) = self.lifecycle.begin_operation() else {
                return queue_message(&self.outbound, request.reject("Server is stopping"));
            };
            *running = true;
            let client = self.clone();
            let span = tracing::Span::current();
            thread::spawn(move || {
                let _span = span.enter();
                let _operation = operation;
                let response = client
                    .execute_external(request, plan, &launch)
                    .unwrap_or_else(|reason| request.reject(reason));
                // A Client may send its next Layout as soon as it reads this response.
                // Queue it and release the slot under one lock, after all resource work.
                let mut running = client
                    .external_running
                    .lock()
                    .expect("layout lock poisoned");
                queue_message(&client.outbound, response);
                *running = false;
            });
            false
        } else {
            drop(running);
            let response = self
                .execute_local(request, command)
                .unwrap_or_else(|reason| request.reject(reason));
            queue_message(&self.outbound, response)
        }
    }

    pub(in crate::server) fn snapshot(&self, session_id: SessionId) -> bool {
        // A capture may materialize outside the Session lock. Do not let this
        // connection's Layout commit overtake its older Bootstrap, even when the
        // requester has not subscribed. Git preparation never holds this fence.
        let _fence = self.external_running.lock().expect("layout lock poisoned");
        queue_runtime_bootstrap(&self.state, self.client_id, session_id, &self.outbound)
    }

    fn execute_local(
        &self,
        request: LayoutRequest,
        command: LayoutCommand,
    ) -> Result<ServerMessage, String> {
        let probes = {
            let state = self.state.lock().expect("server state lock poisoned");
            request.validate(&state, self.lifecycle.is_stopping())?;
            state.terminal_cwd_probes_for_layout(&command)
        };
        let observations = observe_terminal_cwds(probes);
        let mut state = self.state.lock().expect("server state lock poisoned");
        request.validate(&state, self.lifecycle.is_stopping())?;
        state.record_terminal_cwd_observations(observations);
        let effect = apply_layout_command(&mut state, command)?;
        Ok(self.publish_effect(state, request, effect))
    }

    fn execute_external(
        &self,
        request: LayoutRequest,
        plan: ExternalLayoutPlan,
        launch: &ShellLaunch,
    ) -> Result<ServerMessage, String> {
        let mut prepared = prepare_external_layout(plan)?;
        if let Err(reason) = self.approve_prepared(request, &mut prepared) {
            return Err(cancel_with_reason(prepared, reason));
        }
        let prepared = match finish_external_layout(prepared, launch) {
            Ok(prepared) => prepared,
            Err(failure) => return Err(self.restore_failed_removal(failure)),
        };
        self.commit_prepared(request, prepared)
    }

    fn commit_prepared(
        &self,
        request: LayoutRequest,
        prepared: PreparedExternalLayout,
    ) -> Result<ServerMessage, String> {
        let fence = self.external_running.lock().expect("layout lock poisoned");
        let mut state = self.state.lock().expect("server state lock poisoned");
        // Git removal cannot be rolled back. Its corresponding Session close must
        // commit even when shutdown began while the terminals and worktree were closing.
        let stopping = self.lifecycle.is_stopping()
            && !matches!(&prepared, PreparedExternalLayout::RemoveWorktree { .. });
        if let Err(reason) = request.validate(&state, stopping) {
            drop(state);
            drop(fence);
            return Err(cancel_with_reason(prepared, reason));
        }
        let created_worktree = match &prepared {
            PreparedExternalLayout::CreateWorktree { parent, child, .. } => {
                Some((parent.clone(), child.clone()))
            }
            _ => None,
        };
        let effect = match apply_prepared_external_layout(&mut state, prepared) {
            Ok(effect) => effect,
            Err(reason) => {
                drop(state);
                drop(fence);
                return Err(match created_worktree {
                    Some((parent, child)) => prepared_worktree_failure(&parent, &child, reason),
                    None => reason,
                });
            }
        };
        Ok(self.publish_effect(state, request, effect))
    }

    fn approve_prepared(
        &self,
        request: LayoutRequest,
        prepared: &mut PreparedExternalLayout,
    ) -> Result<(), String> {
        let probes = {
            let state = self.state.lock().expect("server state lock poisoned");
            request.validate(&state, self.lifecycle.is_stopping())?;
            match prepared {
                PreparedExternalLayout::RemoveWorktreeReady { workspace_id, .. } => {
                    state.terminal_cwd_probes_for_workspace(*workspace_id)
                }
                _ => Vec::new(),
            }
        };
        let observations = observe_terminal_cwds(probes);
        let mut state = self.state.lock().expect("server state lock poisoned");
        request.validate(&state, self.lifecycle.is_stopping())?;
        state.record_terminal_cwd_observations(observations);
        approve_external_layout(&mut state, prepared)
    }

    fn publish_effect(
        &self,
        mut state: std::sync::MutexGuard<'_, RuntimeState>,
        request: LayoutRequest,
        effect: LayoutEffect,
    ) -> ServerMessage {
        let LayoutEffect {
            result,
            event,
            started_terminals,
            removed_terminals,
        } = effect;
        state.publish_event(event, Some((self.client_id, &self.outbound)));
        let response = ServerMessage::LayoutApplied {
            server_id: state.server_id,
            session_id: state.session_id,
            request_id: request.request_id,
            sequence: state.sequence,
            result,
        };
        drop(state);
        self.start_monitors(started_terminals);
        drop(removed_terminals);
        response
    }

    fn restore_failed_removal(&self, failure: ExternalLayoutFinishError) -> String {
        let ExternalLayoutFinishError {
            message,
            restarted,
            exited,
            cwds,
        } = failure;
        let mut started = Vec::new();
        let mut discarded = Vec::new();
        let mut state = self.state.lock().expect("server state lock poisoned");
        state.record_terminal_cwds(cwds);
        if self.lifecycle.is_stopping() {
            drop(state);
            // Shutdown owns the final Snapshot; it must not acquire fresh terminals.
            drop(restarted);
            drop(exited);
            return message;
        }
        let mut cleared_agents = Vec::new();
        for (pane_id, runtime, updates) in restarted {
            if state.session.pane(pane_id).is_none() || state.terminals.contains_key(&pane_id) {
                discarded.push(runtime);
                continue;
            }
            state.exited_terminals.remove(&pane_id);
            if state.agents.remove(&pane_id).is_some() {
                cleared_agents.push(pane_id);
            }
            started.push(state.install_terminal(pane_id, runtime, updates));
        }
        for pane_id in cleared_agents {
            state.publish_background(SessionEvent::AgentChanged {
                pane_id,
                agent: None,
            });
        }
        for (pane_id, runtime) in exited {
            state.restore_exited_terminal(pane_id, runtime);
        }
        let clients = state.subscribers.keys().copied().collect::<Vec<_>>();
        drop(state);
        self.start_monitors(started);
        drop(discarded);
        for client_id in clients {
            flush_terminal_render(&self.state, client_id);
        }
        message
    }

    fn start_monitors(&self, started: Vec<StartedTerminal>) {
        for (pane_id, instance_id, updates, agent_probe, cwd_probe, notice_probe, view_source) in
            started
        {
            monitor_terminal(TerminalMonitor {
                pane_id,
                instance_id,
                updates,
                agent_probe,
                cwd_probe,
                notice_probe,
                view_source,
                state: Arc::clone(&self.state),
                lifecycle: Arc::clone(&self.lifecycle),
            });
        }
    }
}

fn cancel_with_reason(prepared: PreparedExternalLayout, reason: String) -> String {
    match cancel_prepared_external_layout(prepared) {
        Ok(()) => reason,
        Err(cleanup) => {
            tracing::warn!("{cleanup}");
            format!("{reason}; {cleanup}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layout_commit_cannot_overtake_its_connections_bootstrap() {
        for subscribed in [false, true] {
            let root = std::env::temp_dir();
            let mut state = RuntimeState::new(&root.join("condr-layout-fence.sock"));
            let parent = state.session.create_workspace(root.clone()).unwrap();
            let child = state.session.create_workspace(root.join("child")).unwrap();
            assert!(
                state
                    .session
                    .associate_worktree(child, parent, root, true, None)
            );
            let request = LayoutRequest {
                server_id: state.server_id,
                session_id: state.session_id,
                request_id: 1,
            };
            let (outbound, receiver) = ClientWriter::channel();
            if subscribed {
                state.ensure_subscriber(1, outbound.clone());
            }
            let state = Arc::new(Mutex::new(state));
            let client = ClientLayouts::new(
                1,
                Arc::clone(&state),
                Arc::new(ServerLifecycle::default()),
                outbound,
            );

            // Hold the same fence snapshot() holds while it captures and encodes.
            // A removal's final commit has no filesystem work left to delay the test.
            let fence = client.external_running.lock().unwrap();
            let worker = client.clone();
            let (started_tx, started_rx) = mpsc::channel();
            let (finished_tx, finished_rx) = mpsc::channel();
            let operation = thread::spawn(move || {
                started_tx.send(()).unwrap();
                let response = worker
                    .commit_prepared(
                        request,
                        PreparedExternalLayout::RemoveWorktree {
                            workspace_id: child,
                        },
                    )
                    .unwrap();
                queue_message(&worker.outbound, response);
                finished_tx.send(()).unwrap();
            });
            started_rx.recv().unwrap();
            assert!(matches!(
                finished_rx.recv_timeout(Duration::from_millis(50)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            assert_eq!(state.lock().unwrap().session.workspaces().len(), 2);
            assert!(!queue_runtime_bootstrap(
                &state,
                1,
                request.session_id,
                &client.outbound,
            ));
            drop(fence);
            operation.join().unwrap();

            let Some(ClientWriteItem::ReliableBatch(bootstrap)) = receiver.recv() else {
                panic!("Bootstrap must be queued before this connection's Layout event");
            };
            let ServerMessage::Bootstrap(header) =
                condr_core::protocol::read_message(&mut std::io::Cursor::new(&bootstrap[0]))
                    .unwrap()
            else {
                panic!("Bootstrap begins with its header");
            };
            assert_eq!(
                Session::restore(header.snapshot)
                    .unwrap()
                    .workspaces()
                    .len(),
                2
            );
            let Some(ClientWriteItem::Reliable(event)) = receiver.recv() else {
                panic!("Layout event follows Bootstrap");
            };
            assert!(matches!(
                condr_core::protocol::read_message(&mut std::io::Cursor::new(event)).unwrap(),
                ServerMessage::Event {
                    sequence: 1,
                    event: SessionEvent::LayoutChanged { .. },
                    ..
                }
            ));
            let Some(ClientWriteItem::Reliable(applied)) = receiver.recv() else {
                panic!("LayoutApplied follows its event");
            };
            assert!(matches!(
                condr_core::protocol::read_message(&mut std::io::Cursor::new(applied)).unwrap(),
                ServerMessage::LayoutApplied {
                    request_id: 1,
                    sequence: 1,
                    ..
                }
            ));
        }
    }
}
