//! Git through gitoxide: repository facts for the sidebar and the linked-worktree lifecycle.
//! Nothing here spawns `git` and nothing contacts a remote; the upstream numbers come from
//! the remote-tracking ref as the user's last fetch left it.

mod changes;
#[cfg(feature = "runtime")]
mod repository;
#[cfg(feature = "runtime")]
mod status;

use serde::{Deserialize, Serialize};

pub use changes::{
    DiffHunk, DiffLine, DiffLineKind, FileDiff, FileDiffContent, GitChangeEntry, GitChangeStatus,
    GitChanges, GitDiffStat, MAX_DIFF_BYTES, MAX_GIT_CHANGES,
};
#[cfg(feature = "runtime")]
pub use repository::{
    GitBase, GitError, GitFingerprint, GitRepository, create_worktree, discover_repository,
    open_worktree, remove_worktree, validate_worktree_removal, worktree_destination,
};

/// How far the branch has diverged from its upstream, as of the last local fetch.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitUpstream {
    pub ahead: u32,
    pub behind: u32,
}
