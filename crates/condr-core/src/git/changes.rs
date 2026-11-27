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
    /// Relative to the Workspace's Root Directory (ADR 0073).
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

/// The side a Workspace file is compared with, for the Preview Pane's change marks
/// (ADR 0050): what `GitDiff` would diff the working tree against.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OldFile {
    /// That side's text as a checkout would write it, decoded lossily.
    Text(String),
    /// That side lacks the path, and the ignore rules do not exclude it: every line is new.
    Added,
    /// Nothing to compare: ignored, binary, over [`MAX_DIFF_BYTES`], or a symlink on
    /// either side.
    None,
}

/// One commit as Git History and Git Graph list it (ADR 0046, ADR 0079).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommitSummary {
    /// The full id, as hex.
    pub id: String,
    /// The first line of the message.
    pub subject: String,
    pub author: String,
    /// The author's time, in seconds since the Unix epoch.
    pub time: i64,
    /// The parents' full ids, first parent first.
    pub parents: Vec<String>,
    pub author_email: String,
    /// Someone other than the author committed it.
    pub committer_differs: bool,
}

/// Where a log walk starts (ADR 0079).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum LogStart {
    /// `HEAD`, as Git History walks.
    #[default]
    Head,
    /// Every branch and tag, and `HEAD`, as Git Graph opens.
    All,
    /// One ref by its full name, such as `refs/heads/main`, as Git Graph's branch filter asks.
    Ref(String),
}

/// A branch, tag or `HEAD`, and the commit it names (ADR 0079).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitRef {
    /// `HEAD`, or the full name: `refs/heads/…`, `refs/remotes/…` or `refs/tags/…`.
    pub name: String,
    /// The commit, an annotated tag peeled to it.
    pub commit: String,
}

/// One page of a log: the commits reachable from `from`, in `git rev-list --date-order`'s
/// order: no parent before all its children, otherwise newest commit time first.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitLog {
    /// The commits the walk started from, which later pages name to stay put while refs
    /// move; empty for an unborn `HEAD`.
    pub from: Vec<String>,
    pub commits: Vec<CommitSummary>,
    /// Further commits follow this page.
    pub more: bool,
    /// The refs, on a first page that walked from every ref or one (ADR 0079).
    pub refs: Vec<GitRef>,
    /// The repository's `user.email`, on such a page.
    pub user_email: Option<String>,
}

/// Who committed a commit and when, where that is not its author (ADR 0079).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommitPerson {
    pub name: String,
    pub email: String,
    /// Seconds since the Unix epoch.
    pub time: i64,
}

/// One commit unfolded in Git History: its whole message and the files it changed against its
/// first parent, or against nothing for a root commit.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommitDetails {
    pub id: String,
    pub parents: Vec<String>,
    pub author: String,
    pub time: i64,
    pub message: String,
    pub changes: GitChanges,
    pub author_email: String,
    /// The committer, when someone other than the author.
    pub committer: Option<CommitPerson>,
}

/// How many commits a Git History page holds.
pub const GIT_LOG_PAGE: u32 = 100;
