pub mod protocol;
mod session;
mod snapshot;
mod terminal;

pub use session::{
    CloseOutcome, Pane, PaneDirection, PaneId, PaneLayout, Session, SplitDirection, Tab, TabId,
    Workspace, WorkspaceId,
};
pub use snapshot::{SessionSnapshot, SnapshotError};
pub use terminal::{
    CommandBuilder, TerminalCell, TerminalColor, TerminalCommand, TerminalCursor,
    TerminalCursorShape, TerminalKey, TerminalModifiers, TerminalPosition, TerminalRuntime,
    TerminalScroll, TerminalSide, TerminalSize, TerminalUpdate, TerminalView,
};

pub const APP_NAME: &str = "Murmur";
