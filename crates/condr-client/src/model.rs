//! What a Client keeps of one connection (ADR 0039): the last validated Session, the
//! terminals, titles, bells and Agents in it, kept current by the Bootstrap, layout events,
//! Agent events and terminal frames. Which Workspace a Client shows, and how, is its own.

use crate::frames::{ClientTerminal, apply_terminal_frame_batch};
use condr_core::protocol::{
    PaneTerminalFrame, RuntimeEpoch, ServerId, ServerSettings, SessionBootstrap, SessionId,
    WorkspaceGitSnapshot,
};
use condr_core::{
    AgentDisplayState, AgentSnapshot, AgentState, AgentTracker, PaneId, Session, SessionSnapshot,
    TerminalHyperlinkBudget, Workspace, WorkspaceId,
};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct SessionModel {
    pub server_id: Option<ServerId>,
    pub runtime_epoch: Option<RuntimeEpoch>,
    pub session_id: Option<SessionId>,
    pub sequence: u64,
    /// The last validated structure; absent until the first Bootstrap.
    session: Option<Session>,
    pub terminals: HashMap<PaneId, ClientTerminal>,
    pub terminal_titles: HashMap<PaneId, String>,
    pub terminal_hyperlinks: HashMap<PaneId, TerminalHyperlinkBudget>,
    pub agents: HashMap<PaneId, AgentSnapshot>,
    /// Done is each Client's own (ADR 0014, ADR 0041): an `Idle` after work this Client
    /// did not watch.
    pub agent_trackers: HashMap<PaneId, AgentTracker>,
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
        if authority_changed {
            self.agent_trackers.clear();
        }
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
        self.terminal_titles.clear();
        self.terminal_hyperlinks.clear();
        for mut terminal in bootstrap.terminals {
            let pane_id = terminal.pane_id;
            if let Some(title) = terminal.title.take() {
                self.terminal_titles.insert(pane_id, title);
            }
            self.terminal_hyperlinks
                .insert(pane_id, TerminalHyperlinkBudget::new(&mut terminal.view));
            self.terminals.insert(pane_id, terminal.into());
        }
        self.agents = bootstrap
            .agents
            .into_iter()
            .map(|agent| (agent.pane_id, agent.agent))
            .collect();
        self.agent_trackers
            .retain(|pane_id, _| self.agents.contains_key(pane_id));
        for (&pane_id, agent) in &self.agents {
            self.agent_trackers
                .entry(pane_id)
                .and_modify(|tracker| tracker.update(agent.state, false))
                .or_insert_with(|| AgentTracker::new(agent.state));
        }
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
        self.terminal_hyperlinks
            .retain(|pane_id, _| live.contains(pane_id));
        self.terminal_titles
            .retain(|pane_id, _| live.contains(pane_id));
        self.agents.retain(|pane_id, _| live.contains(pane_id));
        self.agent_trackers
            .retain(|pane_id, _| live.contains(pane_id));
        self.attention.retain(|pane_id| live.contains(pane_id));
        self.workspace_git
            .retain(|workspace_id, _| live_workspaces.contains(workspace_id));
        Ok(previous)
    }

    /// Applies one batch of terminal frames, every Pane or none; the Panes it changed.
    /// `Err(())` asks for a new Bootstrap, as [`crate::frames`] explains.
    #[allow(clippy::result_unit_err)]
    pub fn apply_terminal_frames(
        &mut self,
        panes: Vec<PaneTerminalFrame>,
    ) -> Result<Vec<PaneId>, ()> {
        apply_terminal_frame_batch(&mut self.terminals, &mut self.terminal_hyperlinks, panes)
    }

    /// Applies an `AgentChanged`. `visible` is whether this Client shows the Pane, so an
    /// `Idle` it watched happen is not Done.
    pub fn apply_agent(
        &mut self,
        pane_id: PaneId,
        agent: Option<AgentSnapshot>,
        visible: bool,
    ) -> AgentChange {
        let Some(agent) = agent else {
            self.agent_trackers.remove(&pane_id);
            return AgentChange {
                previous: self.agents.remove(&pane_id),
                started: false,
            };
        };
        let previous = self.agents.insert(pane_id, agent.clone());
        // The Server only publishes changed snapshots, so Unknown here is a new process,
        // not a repeat.
        let started = previous
            .as_ref()
            .is_none_or(|previous| previous.kind != agent.kind)
            || agent.state == AgentState::Unknown;
        self.agent_trackers
            .entry(pane_id)
            .and_modify(|tracker| {
                // Unknown is only published for a new process; it must not carry the
                // previous generation's done.
                if agent.state == AgentState::Unknown {
                    *tracker = AgentTracker::new(agent.state);
                } else {
                    tracker.update(agent.state, visible);
                }
            })
            .or_insert_with(|| AgentTracker::new(agent.state));
        AgentChange { previous, started }
    }

    /// The person looked at the Pane: its Done goes.
    pub fn mark_seen(&mut self, pane_id: PaneId) {
        if let Some(tracker) = self.agent_trackers.get_mut(&pane_id) {
            tracker.mark_seen();
        }
    }

    /// How the Pane's Agent shows to this Client: its state, or Done.
    pub fn agent_display_state(&self, pane_id: PaneId) -> Option<AgentDisplayState> {
        let agent = self.agents.get(&pane_id)?;
        Some(
            self.agent_trackers
                .get(&pane_id)
                .map(|tracker| tracker.display_state())
                .unwrap_or_else(|| AgentTracker::new(agent.state).display_state()),
        )
    }

    /// What a Pane is called: the title its program set, else its Agent's name.
    pub fn pane_title(&self, pane_id: PaneId) -> Option<String> {
        self.terminal_titles.get(&pane_id).cloned().or_else(|| {
            self.agents
                .get(&pane_id)
                .map(|agent| agent.kind.label().to_owned())
        })
    }

    /// The "Needs you" entries of this connection (ADR 0024): every `Blocked` Agent, with
    /// the Workspace it runs in, in Session order.
    pub fn blocked_agents(&self) -> Vec<(&Workspace, PaneId, &AgentSnapshot)> {
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
                let agent = self
                    .agents
                    .get(&pane_id)
                    .filter(|agent| agent.state == AgentState::Blocked)?;
                Some((workspace, pane_id, agent))
            })
            .collect()
    }
}
