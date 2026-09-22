mod agent;
pub mod agent_discovery;
mod config;
mod files;
mod git;
mod paths;
pub mod protocol;
mod session;
mod snapshot;
mod terminal;
pub mod uri;

pub use agent::{
    AgentDetector, AgentDisplayState, AgentEvent, AgentEventKind, AgentKind, AgentPublish,
    AgentResume, AgentSnapshot, AgentState, AgentTracker, ProcessInfo, ProcessProbeResult,
    hook as agent_hook, hooks as agent_hooks, identify_agent_among, identify_agent_process,
};
pub use config::{config_path, read_config_text, read_config_value, update_config_values};
pub use files::{
    DirectoryEntry, DirectoryListing, FileContent, FileKind, MAX_DIRECTORY_ENTRIES, MAX_FILE_BYTES,
    list_directory, read_file, valid_directory_path,
};
pub use git::{
    DiffHunk, DiffLine, DiffLineKind, FileDiff, FileDiffContent, GitChangeEntry, GitChangeStatus,
    GitChanges, GitDiffStat, GitError, GitFingerprint, GitRepository, GitUpstream, MAX_DIFF_BYTES,
    MAX_GIT_CHANGES, create_worktree, discover_repository, open_worktree, remove_worktree,
    validate_worktree_removal, worktree_destination,
};
pub use paths::{
    config_directory, data_directory, log_directory, runtime_directory, state_directory,
};
pub use session::{
    CloseOutcome, DIFF_TAB_NAME, DiffView, FILE_TAB_NAME, FileView, Pane, PaneDirection, PaneId,
    PaneLayout, PaneRect, Session, SplitDirection, Tab, TabContent, TabId, TerminalLayout,
    Workspace, WorkspaceId, WorktreeAssociation, valid_diff_path,
};
pub use snapshot::{SessionSnapshot, SnapshotError};
pub use terminal::{
    CURSOR_POSITION_SETTLE, CommandBuilder, DEFAULT_ANSI_COLORS, DEFAULT_BACKGROUND_COLOR,
    DEFAULT_CURSOR_COLOR, DEFAULT_FOREGROUND_COLOR, DETECTED_LINK_FLAG, PaneEnvironment,
    TerminalAgentProbe, TerminalCell, TerminalCellRun, TerminalColor, TerminalCommand,
    TerminalCursor, TerminalCursorShape, TerminalCwdProbe, TerminalFrameError,
    TerminalHyperlinkBudget, TerminalKey, TerminalLaunchProbe, TerminalModifiers,
    TerminalMouseButton, TerminalMouseEvent, TerminalMousePosition, TerminalMouseTracking,
    TerminalMouseWheel, TerminalNoticeBatch, TerminalNoticeProbe, TerminalPosition,
    TerminalRuntime, TerminalScroll, TerminalSelection, TerminalSelectionUnit, TerminalSide,
    TerminalSize, TerminalUpdate, TerminalView, TerminalViewDelta, TerminalViewFrame,
    TerminalViewSource, default_indexed_color, default_shell_program,
};

pub const APP_NAME: &str = "Condr";

/// What this binary is: the crate version, and the commit CI built it from when it did
/// (`CONDR_BUILD_COMMIT` at compile time; a local `cargo build` has none). Every Condr
/// crate shares the workspace version, so one value names the whole build.
pub fn build_identity() -> String {
    match option_env!("CONDR_BUILD_COMMIT") {
        Some(commit) if !commit.is_empty() => {
            format!("{}+{commit}", env!("CARGO_PKG_VERSION"))
        }
        _ => env!("CARGO_PKG_VERSION").to_string(),
    }
}
