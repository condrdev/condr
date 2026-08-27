pub mod protocol;
mod session;
mod snapshot;

pub use session::{
    CloseOutcome, Pane, PaneId, PaneLayout, Session, SplitDirection, Tab, TabId, Workspace,
    WorkspaceId,
};
pub use snapshot::{SessionSnapshot, SnapshotError};

pub const APP_NAME: &str = "Murmur";
