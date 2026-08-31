mod agent;
mod git;
pub mod protocol;
mod session;
mod snapshot;
mod terminal;

pub use agent::{
    AgentDisplayState, AgentKind, AgentSnapshot, AgentState, AgentTracker, classify_agent,
    identify_agent_process,
};
pub use git::{
    GitError, GitRepository, create_worktree, default_worktree_root, discover_repository,
    open_worktree, remove_worktree, validate_worktree_removal,
};
pub use session::{
    CloseOutcome, Pane, PaneDirection, PaneId, PaneLayout, Session, SplitDirection, Tab, TabId,
    Workspace, WorkspaceId, WorktreeAssociation,
};
pub use snapshot::{SessionSnapshot, SnapshotError};
pub use terminal::{
    CommandBuilder, TerminalAgentProbe, TerminalCell, TerminalCellRun, TerminalColor,
    TerminalCommand, TerminalCursor, TerminalCursorShape, TerminalCwdProbe, TerminalFrameError,
    TerminalKey, TerminalModifiers, TerminalPosition, TerminalRuntime, TerminalScroll,
    TerminalSelection, TerminalSide, TerminalSize, TerminalUpdate, TerminalView, TerminalViewDelta,
    TerminalViewFrame, TerminalViewSource,
};

pub const APP_NAME: &str = "Condr";
