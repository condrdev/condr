mod agent;
pub mod agent_discovery;
mod git;
mod paths;
pub mod protocol;
mod session;
mod snapshot;
mod terminal;

pub use agent::{
    AgentDetector, AgentDisplayState, AgentEvent, AgentEventKind, AgentKind, AgentPublish,
    AgentSnapshot, AgentState, AgentTracker, ProcessInfo, ProcessProbeResult, hook as agent_hook,
    identify_agent_among, identify_agent_process,
};
pub use git::{
    GitError, GitHeadFingerprint, GitRepository, create_worktree, default_worktree_root,
    discover_repository, open_worktree, remove_worktree, validate_worktree_removal,
};
pub use paths::{
    config_directory, data_directory, log_directory, runtime_directory, state_directory,
};
pub use session::{
    CloseOutcome, Pane, PaneDirection, PaneId, PaneLayout, PaneRect, Session, SplitDirection, Tab,
    TabId, Workspace, WorkspaceId, WorktreeAssociation,
};
pub use snapshot::{SessionSnapshot, SnapshotError};
pub use terminal::{
    CURSOR_POSITION_SETTLE, CommandBuilder, PaneEnvironment, TerminalAgentProbe, TerminalCell,
    TerminalCellRun, TerminalColor, TerminalCommand, TerminalCursor, TerminalCursorShape,
    TerminalCwdProbe, TerminalFrameError, TerminalHyperlinkBudget, TerminalKey, TerminalModifiers,
    TerminalMouseButton, TerminalMouseEvent, TerminalMousePosition, TerminalMouseTracking,
    TerminalMouseWheel, TerminalNoticeBatch, TerminalNoticeProbe, TerminalPosition,
    TerminalRuntime, TerminalScroll, TerminalSelection, TerminalSide, TerminalSize, TerminalUpdate,
    TerminalView, TerminalViewDelta, TerminalViewFrame, TerminalViewSource, default_shell_program,
};

pub const APP_NAME: &str = "Condr";
