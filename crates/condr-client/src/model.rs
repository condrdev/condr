//! What a Client keeps of one connection (ADR 0039): the last validated Session, the
//! terminals, titles, bells and Agents in it, kept current by the Bootstrap, layout events,
//! Agent events and terminal frames. Which Workspace a Client shows, and how, is its own.

use crate::frames::{ClientTerminal, apply_terminal_frame_batch};
use condr_core::protocol::{
    PaneTerminalFrame, RuntimeEpoch, ServerId, ServerSettings, SessionBootstrap, SessionId,
    WorkspaceGitSnapshot,
};
use condr_core::{
    AgentDisplayState, AgentSnapshot, AgentState, PaneId, Session, SessionSnapshot,
    TerminalHyperlinkBudget, TerminalViewFrame, Workspace, WorkspaceId,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Default)]
pub struct SessionModel {
    pub server_id: Option<ServerId>,
    pub runtime_epoch: Option<RuntimeEpoch>,
    pub session_id: Option<SessionId>,
    pub sequence: u64,
    /// The last validated structure; absent until the first Bootstrap.
    session: Option<Session>,
    /// The terminals this Client holds a view of: every Pane's, or only the watched ones'
    /// (ADR 0041).
    pub terminals: HashMap<PaneId, ClientTerminal>,
    /// Panes whose terminal exited, whether this Client holds their view or not.
    pub exited: HashSet<PaneId>,
    /// The Panes this Client asked frames for with `WatchTerminals`; `None` is every Pane.
    watched: Option<HashSet<PaneId>>,
    pub terminal_titles: HashMap<PaneId, String>,
    pub terminal_hyperlinks: HashMap<PaneId, TerminalHyperlinkBudget>,
    pub agents: HashMap<PaneId, AgentSnapshot>,
    /// Panes whose `Done` no Client has looked at yet, as the Server keeps them (ADR 0052).
    pub agent_unseen: HashSet<PaneId>,
    /// Panes that rang BEL while no Client had them focused.
    pub attention: HashSet<PaneId>,
    pub workspace_git: HashMap<WorkspaceId, WorkspaceGitSnapshot>,
    pub zoomed_panes: HashSet<PaneId>,
    /// Server-owned preferences from the Bootstrap, kept current by events.
    pub settings: ServerSettings,
}

/// What a Bootstrap changed beyond the model itself.
pub struct BootstrapChange {
    /// A different Server, runtime or Session: whatever a Client cached for the old one
    /// is stale, and its subscription with it.
    pub authority_changed: bool,
    /// The structure the Bootstrap replaced, for the Client to carry its view across.
    pub previous: Option<Session>,
}

/// What an `AgentChanged` meant.
pub struct AgentChange {
    pub previous: Option<AgentSnapshot>,
    /// A new Agent process in the Pane: none before, another kind, or an `Unknown` that
    /// the Server publishes only for a new process.
    pub started: bool,
}

impl SessionModel {
    pub fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }

    /// Replaces everything with the Server's authoritative state. Validates first, so an
    /// invalid Bootstrap leaves the model as it was.
    pub fn apply_bootstrap(
        &mut self,
        bootstrap: SessionBootstrap,
    ) -> Result<BootstrapChange, String> {
        let session = Session::restore(bootstrap.snapshot)
            .map_err(|error| format!("invalid Session snapshot: {error}"))?;
        let authority_changed = self.server_id != Some(bootstrap.server_id)
            || self.runtime_epoch != Some(bootstrap.runtime_epoch)
            || self.session_id != Some(bootstrap.session_id);
        self.server_id = Some(bootstrap.server_id);
        self.runtime_epoch = Some(bootstrap.runtime_epoch);
        self.session_id = Some(bootstrap.session_id);
        self.sequence = bootstrap.sequence;
        let previous = self.session.replace(session);
        // Server-authoritative, like the rest of the Bootstrap.
        self.attention = bootstrap
            .terminals
            .iter()
            .filter_map(|terminal| terminal.attention.then_some(terminal.pane_id))
            .collect();
        self.terminals.clear();
        self.exited.clear();
        self.terminal_titles.clear();
        self.terminal_hyperlinks.clear();
        for terminal in bootstrap.terminals {
            let pane_id = terminal.pane_id;
            if let Some(title) = terminal.title {
                self.terminal_titles.insert(pane_id, title);
            }
            if terminal.exited {
                self.exited.insert(pane_id);
            }
            // A view for a Pane the set dropped while the Bootstrap was on its way is stale:
            // the Server keeps no baseline for it.
            if let Some(mut view) = terminal.view
                && self.watches(pane_id)
            {
                self.terminal_hyperlinks
                    .insert(pane_id, TerminalHyperlinkBudget::new(&mut view));
                self.terminals.insert(
                    pane_id,
                    ClientTerminal {
                        view: Arc::new(view),
                    },
                );
            }
        }
        self.agent_unseen = bootstrap
            .agents
            .iter()
            .filter(|agent| agent.unseen)
            .map(|agent| agent.pane_id)
            .collect();
        self.agents = bootstrap
            .agents
            .into_iter()
            .map(|agent| (agent.pane_id, agent.agent))
            .collect();
        self.workspace_git = bootstrap
            .workspace_git
            .into_iter()
            .map(|git| (git.workspace_id, git))
            .collect();
        self.zoomed_panes = bootstrap.zoomed_panes.into_iter().collect();
        self.settings = bootstrap.settings;
        Ok(BootstrapChange {
            authority_changed,
            previous,
        })
    }

    /// Replaces the structure atomically while keeping live terminals, and returns the one
    /// it replaced. What belonged to a Pane or Workspace that is gone goes with it: the
    /// Server sends no event of its own for those.
    pub fn apply_layout(
        &mut self,
        snapshot: SessionSnapshot,
        zoomed_panes: Vec<PaneId>,
    ) -> Result<Option<Session>, String> {
        let session = Session::restore(snapshot)
            .map_err(|error| format!("invalid Session snapshot: {error}"))?;
        let live: HashSet<PaneId> = session
            .workspaces()
            .iter()
            .flat_map(|workspace| workspace.tabs())
            .flat_map(|tab| tab.panes())
            .map(|pane| pane.id())
            .collect();
        let live_workspaces: HashSet<WorkspaceId> = session
            .workspaces()
            .iter()
            .map(|workspace| workspace.id())
            .collect();
        let previous = self.session.replace(session);
        self.zoomed_panes = zoomed_panes.into_iter().collect();
        self.terminals.retain(|pane_id, _| live.contains(pane_id));
        self.exited.retain(|pane_id| live.contains(pane_id));
        self.terminal_hyperlinks
            .retain(|pane_id, _| live.contains(pane_id));
        self.terminal_titles
            .retain(|pane_id, _| live.contains(pane_id));
        self.agents.retain(|pane_id, _| live.contains(pane_id));
        self.agent_unseen.retain(|pane_id| live.contains(pane_id));
        self.attention.retain(|pane_id| live.contains(pane_id));
        self.workspace_git
            .retain(|workspace_id, _| live_workspaces.contains(workspace_id));
        Ok(previous)
    }

    /// Limits this Client's terminals to `panes`, the set it sends in `WatchTerminals`
    /// (ADR 0041). A Pane that leaves the set loses its view, since the Server drops its
    /// baseline; one that joins has none until its full frame arrives.
    pub fn watch_terminals(&mut self, panes: HashSet<PaneId>) {
        self.terminals.retain(|pane_id, _| panes.contains(pane_id));
        self.terminal_hyperlinks
            .retain(|pane_id, _| panes.contains(pane_id));
        self.watched = Some(panes);
    }

    /// The set `watch_terminals` last took; `None` is every Pane.
    pub fn watched(&self) -> Option<&HashSet<PaneId>> {
        self.watched.as_ref()
    }

    /// Whether this Client wants `pane_id`'s frames.
    pub fn watches(&self, pane_id: PaneId) -> bool {
        self.watched
            .as_ref()
            .is_none_or(|watched| watched.contains(&pane_id))
    }

    /// Applies one batch of terminal frames, every Pane or none; the Panes it changed.
    /// `Err(())` asks for a new Bootstrap, as [`crate::frames`] explains. With a watched
    /// set, frames for other Panes are old ones still on the wire, and so is a delta for a
    /// watched Pane whose full frame has not arrived: both are ignored, not a gap.
    #[allow(clippy::result_unit_err)]
    pub fn apply_terminal_frames(
        &mut self,
        mut panes: Vec<PaneTerminalFrame>,
    ) -> Result<Vec<PaneId>, ()> {
        if self.watched.is_some() {
            panes.retain(|pane| {
                self.watches(pane.pane_id)
                    && (self.terminals.contains_key(&pane.pane_id)
                        || matches!(pane.frame, TerminalViewFrame::Full(_)))
            });
        }
        apply_terminal_frame_batch(&mut self.terminals, &mut self.terminal_hyperlinks, panes)
    }

    /// Applies an `AgentChanged`.
    pub fn apply_agent(&mut self, pane_id: PaneId, agent: Option<AgentSnapshot>) -> AgentChange {
        let Some(agent) = agent else {
            return AgentChange {
                previous: self.agents.remove(&pane_id),
                started: false,
            };
        };
        let previous = self.agents.insert(pane_id, agent.clone());
        // A new process arrives after a `Cleared` or as another kind. Unknown alone is
        // not one: an agent that cleared its record, or lost it to a shell prompt or a
        // reset, is the same process (ADR 0044).
        let started = previous
            .as_ref()
            .is_none_or(|previous| previous.kind != agent.kind);
        AgentChange { previous, started }
    }

    /// Applies an `AgentUnseenChanged` and reports whether it changed anything. The Pane
    /// `in_front` of the person is looked at, whoever reported focus last (ADR 0052).
    pub fn apply_agent_unseen(&mut self, pane_id: PaneId, unseen: bool, in_front: bool) -> bool {
        if unseen && !in_front {
            self.agent_unseen.insert(pane_id)
        } else {
            self.agent_unseen.remove(&pane_id)
        }
    }

    /// The person looked at the Pane: its Done goes here at once, ahead of the Server's
    /// answer to the focus this Client reports.
    pub fn mark_seen(&mut self, pane_id: PaneId) {
        self.agent_unseen.remove(&pane_id);
    }

    /// How the Pane's Agent shows: its state, or Idle for a Done someone looked at.
    pub fn agent_display_state(&self, pane_id: PaneId) -> Option<AgentDisplayState> {
        let agent = self.agents.get(&pane_id)?;
        Some(AgentDisplayState::of(
            agent.state,
            self.agent_unseen.contains(&pane_id),
        ))
    }

    /// What a Pane is called: the title its program set, else its Agent's name.
    pub fn pane_title(&self, pane_id: PaneId) -> Option<String> {
        self.terminal_titles.get(&pane_id).cloned().or_else(|| {
            self.agents
                .get(&pane_id)
                .map(|agent| agent.kind.label().to_owned())
        })
    }

    /// The "Needs you" entries of this connection (ADR 0024, ADR 0044): every Agent that
    /// needs the person, Blocked on an answer or halted on an Error, with the Workspace
    /// it runs in, in Session order.
    pub fn agents_needing_you(&self) -> Vec<(&Workspace, PaneId, &AgentSnapshot)> {
        let Some(session) = &self.session else {
            return Vec::new();
        };
        session
            .workspaces()
            .iter()
            .flat_map(|workspace| {
                workspace
                    .tabs()
                    .iter()
                    .flat_map(|tab| tab.panes())
                    .map(move |pane| (workspace, pane.id()))
            })
            .filter_map(|(workspace, pane_id)| {
                let agent = self.agents.get(&pane_id).filter(|agent| {
                    matches!(agent.state, AgentState::Blocked | AgentState::Error)
                })?;
                Some((workspace, pane_id, agent))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use condr_core::{
        TerminalCell, TerminalCellRun, TerminalColor, TerminalMouseTracking, TerminalSize,
        TerminalView, TerminalViewDelta,
    };

    fn full(pane_id: PaneId, revision: u64) -> PaneTerminalFrame {
        PaneTerminalFrame {
            pane_id,
            frame: TerminalViewFrame::Full(TerminalView {
                selection: None,
                revision,
                size: TerminalSize::new(1, 1),
                display_offset: 0,
                mouse_tracking: TerminalMouseTracking::None,
                cells: vec![cell()],
                cursor: None,
            }),
        }
    }

    fn delta(pane_id: PaneId, base_revision: u64, revision: u64) -> PaneTerminalFrame {
        PaneTerminalFrame {
            pane_id,
            frame: TerminalViewFrame::Delta(TerminalViewDelta {
                selection: None,
                base_revision,
                revision,
                display_offset: 0,
                mouse_tracking: TerminalMouseTracking::None,
                cursor: None,
                runs: vec![TerminalCellRun {
                    start: 0,
                    cells: vec![cell()],
                }],
            }),
        }
    }

    fn cell() -> TerminalCell {
        TerminalCell {
            text: "x".into(),
            foreground: TerminalColor::Named(0),
            background: TerminalColor::Named(0),
            flags: 0,
            hyperlink: None,
        }
    }

    /// ADR 0041: after a change of set, frames for other Panes and a delta for a Pane
    /// whose full frame has not come are old frames in flight, not a gap.
    #[test]
    fn frames_outside_the_set_and_an_old_delta_for_a_newly_watched_pane_are_ignored() {
        let (first, second) = (PaneId::from_u64(1), PaneId::from_u64(2));
        let mut model = SessionModel::default();
        model.watch_terminals([first].into());
        assert_eq!(
            model.apply_terminal_frames(vec![full(first, 1), full(second, 1)]),
            Ok(vec![first])
        );
        assert!(!model.terminals.contains_key(&second));

        model.watch_terminals([first, second].into());
        assert_eq!(
            model.apply_terminal_frames(vec![delta(second, 1, 2)]),
            Ok(Vec::new()),
            "no Bootstrap for an old delta"
        );
        assert_eq!(
            model.apply_terminal_frames(vec![full(second, 3)]),
            Ok(vec![second])
        );
        assert_eq!(
            model.apply_terminal_frames(vec![delta(second, 3, 4)]),
            Ok(vec![second])
        );

        model.watch_terminals([second].into());
        assert!(
            !model.terminals.contains_key(&first),
            "a Pane that left the set loses its view"
        );
        assert_eq!(
            model.apply_terminal_frames(vec![delta(first, 1, 2)]),
            Ok(Vec::new())
        );
    }

    #[test]
    fn a_gap_on_a_watched_pane_with_a_view_still_asks_for_a_bootstrap() {
        let pane_id = PaneId::from_u64(1);
        let mut model = SessionModel::default();
        model.watch_terminals([pane_id].into());
        model.apply_terminal_frames(vec![full(pane_id, 1)]).unwrap();
        assert_eq!(
            model.apply_terminal_frames(vec![delta(pane_id, 5, 6)]),
            Err(())
        );
    }

    /// A record without a view still gives the Pane its title, exit and bell, and a view
    /// for a Pane the set dropped while the Bootstrap was on its way is not kept.
    #[test]
    fn a_bootstrap_keeps_every_record_and_only_the_watched_views() {
        let (first, second) = (PaneId::from_u64(1), PaneId::from_u64(2));
        let TerminalViewFrame::Full(view) = full(first, 1).frame else {
            unreachable!()
        };
        let record = |pane_id, view| condr_core::protocol::PaneTerminalSnapshot {
            pane_id,
            view,
            exited: true,
            title: Some("build".into()),
            attention: true,
        };
        let mut model = SessionModel::default();
        model.watch_terminals(HashSet::new());
        model
            .apply_bootstrap(SessionBootstrap {
                server_id: ServerId(1),
                runtime_epoch: RuntimeEpoch(1),
                session_id: SessionId(1),
                sequence: 0,
                snapshot: Session::new().snapshot(),
                settings: ServerSettings::default(),
                terminals: vec![record(first, Some(view)), record(second, None)],
                agents: Vec::new(),
                workspace_git: Vec::new(),
                zoomed_panes: Vec::new(),
            })
            .unwrap();
        assert!(model.terminals.is_empty());
        for pane_id in [first, second] {
            assert!(model.exited.contains(&pane_id));
            assert!(model.attention.contains(&pane_id));
            assert_eq!(model.pane_title(pane_id).as_deref(), Some("build"));
        }
    }

    /// ADR 0044: an agent that cleared its record is the same process with nothing to
    /// show, not a new one; only a `Cleared` or another kind starts one.
    #[test]
    fn losing_a_record_is_not_a_new_process() {
        let mut model = SessionModel::default();
        let pane_id = PaneId::from_u64(7);
        let snapshot = |state| {
            Some(AgentSnapshot {
                session_id: None,
                kind: condr_core::AgentKind::Codex,
                state,
                detail: None,
            })
        };
        assert!(
            model
                .apply_agent(pane_id, snapshot(AgentState::Unknown))
                .started
        );
        assert!(
            !model
                .apply_agent(pane_id, snapshot(AgentState::Done))
                .started
        );
        // The Server says whether anyone looked at the Done; this Client's own look
        // clears it ahead of the Server's answer.
        assert_eq!(
            model.agent_display_state(pane_id),
            Some(AgentDisplayState::Idle)
        );
        model.agent_unseen.insert(pane_id);
        assert_eq!(
            model.agent_display_state(pane_id),
            Some(AgentDisplayState::Done)
        );
        model.mark_seen(pane_id);
        assert_eq!(
            model.agent_display_state(pane_id),
            Some(AgentDisplayState::Idle)
        );
        assert!(
            !model
                .apply_agent(pane_id, snapshot(AgentState::Unknown))
                .started
        );
        assert_eq!(
            model.agent_display_state(pane_id),
            Some(AgentDisplayState::Unknown)
        );
        assert!(!model.apply_agent(pane_id, None).started);
        assert!(
            model
                .apply_agent(pane_id, snapshot(AgentState::Unknown))
                .started
        );
    }
}
