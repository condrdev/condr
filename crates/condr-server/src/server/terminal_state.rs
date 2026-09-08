use super::*;

impl RuntimeState {
    pub(super) fn clear_controller_terminal_state(&mut self) {
        for runtime in self.terminals.values() {
            let _ = runtime.release_mouse();
            // The selection belongs to the controller (ADR 0008); a successor must not
            // inherit or copy it.
            let _ = runtime.execute(TerminalCommand::Select(None));
        }
        self.clear_terminal_focus();
        for pane_id in std::mem::take(&mut self.pending_terminal_bells) {
            self.publish_background(SessionEvent::TerminalAttentionChanged {
                pane_id,
                attention: false,
            });
        }
    }

    pub(super) fn clear_terminal_focus(&mut self) {
        if let Some(pane_id) = self.focused_terminal.take()
            && let Some(runtime) = self.terminals.get(&pane_id)
        {
            let _ = runtime.execute(TerminalCommand::Focus(false));
        }
    }

    pub(super) fn install_terminal(
        &mut self,
        pane_id: PaneId,
        runtime: TerminalRuntime,
        updates: mpsc::Receiver<TerminalUpdate>,
    ) -> StartedTerminal {
        let instance_id = self.next_terminal_instance;
        self.next_terminal_instance = self.next_terminal_instance.wrapping_add(1).max(1);
        self.forget_agent_control(pane_id);
        let probe = runtime
            .agent_probe()
            .expect("new Terminal has an agent probe");
        let cwd_probe = runtime.cwd_probe();
        let notice_probe = runtime.notice_probe();
        let view_source = runtime.view_source();
        self.clear_terminal_title(pane_id);
        self.clear_terminal_attention(pane_id);
        let mut view = runtime.view();
        let hyperlinks = TerminalHyperlinkBudget::new(&mut view);
        self.terminal_views.insert(
            pane_id,
            LatestTerminalView {
                view: Arc::new(view),
                hyperlinks,
                producer_frame: None,
            },
        );
        for subscriber in self.subscribers.values_mut() {
            subscriber.terminal_baselines.remove(&pane_id);
            subscriber.pending_terminals.insert(pane_id);
            subscriber.render_generation = subscriber.render_generation.wrapping_add(1);
        }
        self.terminals.insert(pane_id, runtime);
        self.terminal_instances.insert(pane_id, instance_id);
        self.closing_terminals.remove(&pane_id);
        (
            pane_id,
            instance_id,
            updates,
            probe,
            cwd_probe,
            notice_probe,
            view_source,
        )
    }

    /// Forgets a Terminal's reported title and tells subscribers, if there was one.
    pub(super) fn clear_terminal_title(&mut self, pane_id: PaneId) {
        if self.terminal_titles.remove(&pane_id).is_some() {
            self.publish_background(SessionEvent::TerminalTitleChanged {
                pane_id,
                title: None,
            });
        }
    }

    pub(super) fn record_terminal_focus(&mut self, pane_id: PaneId, focused: bool) {
        if focused {
            self.focused_terminal = Some(pane_id);
            self.clear_terminal_attention(pane_id);
        } else if self.focused_terminal == Some(pane_id) {
            self.focused_terminal = None;
        }
    }

    pub(super) fn clear_terminal_attention(&mut self, pane_id: PaneId) {
        if self.pending_terminal_bells.remove(&pane_id) {
            self.publish_background(SessionEvent::TerminalAttentionChanged {
                pane_id,
                attention: false,
            });
        }
    }

    pub(super) fn restore_exited_terminal(
        &mut self,
        pane_id: PaneId,
        runtime: TerminalRuntime,
    ) -> bool {
        if self.session.pane(pane_id).is_none() || self.terminals.contains_key(&pane_id) {
            return false;
        }
        let view = runtime.view();
        self.terminal_instances.remove(&pane_id);
        self.forget_agent_control(pane_id);
        self.closing_terminals.remove(&pane_id);
        self.terminals.insert(pane_id, runtime);
        self.publish_terminal(pane_id, TerminalViewFrame::Full(view));
        if self.exited_terminals.insert(pane_id) {
            self.publish_background(SessionEvent::TerminalExited { pane_id });
        }
        self.clear_terminal_title(pane_id);
        if self.agents.remove(&pane_id).is_some() {
            self.publish_background(SessionEvent::AgentChanged {
                pane_id,
                agent: None,
            });
        }
        true
    }

    pub(super) fn terminal_is_current(&self, pane_id: PaneId, instance_id: u64) -> bool {
        self.terminal_instances.get(&pane_id) == Some(&instance_id)
    }

    pub(super) fn terminal_cwd_probes(&self) -> Vec<(PaneId, u64, TerminalCwdProbe)> {
        self.terminals
            .iter()
            .filter(|(pane_id, _)| !self.exited_terminals.contains(pane_id))
            .filter(|(pane_id, _)| !self.closing_terminals.contains(pane_id))
            .filter_map(|(&pane_id, runtime)| {
                self.terminal_instances
                    .get(&pane_id)
                    .copied()
                    .map(|instance_id| (pane_id, instance_id, runtime.cwd_probe()))
            })
            .collect()
    }

    pub(super) fn terminal_cwd_probe(
        &self,
        pane_id: PaneId,
    ) -> Option<(PaneId, u64, TerminalCwdProbe)> {
        if self.exited_terminals.contains(&pane_id) || self.closing_terminals.contains(&pane_id) {
            return None;
        }
        Some((
            pane_id,
            *self.terminal_instances.get(&pane_id)?,
            self.terminals.get(&pane_id)?.cwd_probe(),
        ))
    }

    pub(super) fn terminal_cwd_probes_for_layout(
        &self,
        command: &LayoutCommand,
    ) -> Vec<(PaneId, u64, TerminalCwdProbe)> {
        if !layout_command_needs_cwd_observation(command) {
            return Vec::new();
        }
        let pane_id = match command {
            LayoutCommand::CreateTab { workspace_id, .. } => self
                .session
                .workspace(*workspace_id)
                .map(|workspace| workspace.active_tab().focused_pane().id()),
            LayoutCommand::SplitPane { pane_id, .. } => Some(*pane_id),
            _ => None,
        };
        pane_id
            .and_then(|pane_id| self.terminal_cwd_probe(pane_id))
            .into_iter()
            .collect()
    }

    pub(super) fn terminal_cwd_probes_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> Vec<(PaneId, u64, TerminalCwdProbe)> {
        self.session
            .workspace(workspace_id)
            .into_iter()
            .flat_map(|workspace| workspace.tabs())
            .flat_map(|tab| tab.panes())
            .filter_map(|pane| self.terminal_cwd_probe(pane.id()))
            .collect()
    }
}
