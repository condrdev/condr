//! What a Workspace's changes and diffs are on the wire (ADR 0017, ADR 0034); `status`
//! computes them.

use relative_path::RelativePathBuf;
use serde::{Deserialize, Serialize};

/// How a path differs from `HEAD`, index and worktree differences folded together.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum GitChangeStatus {
    /// In the index but not in `HEAD`.
    Added,
    Modified,
    Deleted,
    /// Moved from `old_path`; the content may have changed as well.
    Renamed,
    /// In the worktree and known to neither `HEAD` nor the index.
    Untracked,
    Conflicted,
}

/// Added and deleted line counts, as `git diff --numstat` reports them.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitDiffStat {
    pub added: u32,
    pub deleted: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitChangeEntry {
    /// Relative to the work tree root.
    pub path: RelativePathBuf,
    /// The previous path of a renamed file.
    pub old_path: Option<RelativePathBuf>,
    pub status: GitChangeStatus,
    /// `None` for binary files, files over [`MAX_DIFF_BYTES`], conflicts, and whenever the
    /// list hit [`MAX_GIT_CHANGES`].
    pub stat: Option<GitDiffStat>,
}

/// The working tree against `HEAD` or a base (ADR 0034), sorted by path.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitChanges {
    pub entries: Vec<GitChangeEntry>,
    /// The list stopped at [`MAX_GIT_CHANGES`] entries; more exist.
    pub truncated: bool,
}

/// More changed paths than this are cut off and reported as truncated, and line counts are
/// skipped: a repository in that state needs a `.gitignore`, not a longer list.
pub const MAX_GIT_CHANGES: usize = 1000;
/// A side of a diff larger than this is not diffed; the file shows as too large.
pub const MAX_DIFF_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    /// 1-based line number in the old file; `None` for an added line.
    pub old_number: Option<u32>,
    /// 1-based line number in the new file; `None` for a removed line.
    pub new_number: Option<u32>,
    /// Without its line terminator; invalid UTF-8 is replaced.
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiffHunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum FileDiffContent {
    /// Empty when both sides are identical.
    Text {
        hunks: Vec<DiffHunk>,
    },
    Binary,
    /// One side exceeds [`MAX_DIFF_BYTES`].
    TooLarge {
        bytes: u64,
    },
}

/// One file's working tree against `HEAD`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: RelativePathBuf,
    pub content: FileDiffContent,
}
