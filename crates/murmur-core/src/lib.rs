pub mod protocol;
mod session;
mod snapshot;
mod terminal;

pub use session::{
    CloseOutcome, Pane, PaneId, PaneLayout, Session, SplitDirection, Tab, TabId, Workspace,
    WorkspaceId,
};
pub use snapshot::{SessionSnapshot, SnapshotError};
pub use terminal::{CommandBuilder, TerminalRuntime, TerminalSize};

pub const APP_NAME: &str = "Murmur";
