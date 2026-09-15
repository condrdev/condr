//! Working-tree changes and per-file diffs (ADR 0017): `git status` folded into one status per
//! path, and one file's working tree against `HEAD` as structured hunks. Everything runs
//! through gix's attribute-aware diff pipeline, so text conversion, binary detection and
//! `diff.algorithm` behave as they do for `git`.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry as MapEntry;
use std::fs;
use std::path::{Path, PathBuf};

use gix::bstr::{BStr, BString};
use gix::diff::blob::unified_diff::{
    ConsumeHunk, ContextSize, DiffLineKind as GixLineKind, HunkHeader,
};
use gix::diff::blob::{Diff, InternedInput, ResourceKind, UnifiedDiff};
use gix::object::tree::EntryKind;
use serde::{Deserialize, Serialize};

use super::{GitError, GitRepository, git_error};

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
    pub path: PathBuf,
    /// The previous path of a renamed file.
    pub old_path: Option<PathBuf>,
    pub status: GitChangeStatus,
    /// `None` for binary files, files over [`MAX_DIFF_BYTES`], conflicts, and whenever the
    /// list hit [`MAX_GIT_CHANGES`].
    pub stat: Option<GitDiffStat>,
}

/// The working tree against `HEAD`, sorted by path.
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
const DIFF_CONTEXT_LINES: u32 = 3;

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
    pub path: PathBuf,
    pub content: FileDiffContent,
}

impl GitRepository {
    /// The working tree against `HEAD`: `git status` with index and worktree folded into one
    /// status per path, untracked files listed one by one, renames tracked, and per-file line
    /// counts unless the list is truncated.
    pub fn changes(&self) -> Result<GitChanges, GitError> {
        let repo = self.open()?;
        let mut folded = BTreeMap::<PathBuf, GitChangeEntry>::new();
        let iter = repo
            .status(gix::progress::Discard)
            .map_err(git_error)?
            .untracked_files(gix::status::UntrackedFiles::Files)
            .into_iter(Vec::<BString>::new())
            .map_err(git_error)?;
        for item in iter {
            let item = item.map_err(git_error)?;
            let Some((path, old_path, status)) = fold_status_item(item) else {
                continue;
            };
            let path = git_path(path.as_ref());
            let old_path = old_path.map(|old| git_path(old.as_ref()));
            match folded.entry(path.clone()) {
                MapEntry::Vacant(slot) => {
                    slot.insert(GitChangeEntry {
                        path,
                        old_path,
                        status,
                        stat: None,
                    });
                }
                MapEntry::Occupied(mut slot) => {
                    let entry = slot.get_mut();
                    match (entry.status, status) {
                        // Staged as new, then deleted again: nothing left against HEAD.
                        (GitChangeStatus::Added, GitChangeStatus::Deleted)
                        | (GitChangeStatus::Deleted, GitChangeStatus::Added) => {
                            slot.remove();
                        }
                        (current, next) if status_rank(next) > status_rank(current) => {
                            entry.status = next;
                            if old_path.is_some() {
                                entry.old_path = old_path;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        let truncated = folded.len() > MAX_GIT_CHANGES;
        let mut entries: Vec<GitChangeEntry> = folded.into_values().take(MAX_GIT_CHANGES).collect();
        if !truncated {
            let mut differ = Differ::new(&repo)?;
            for entry in &mut entries {
                entry.stat = differ.stat(entry).ok().flatten();
            }
        }
        Ok(GitChanges { entries, truncated })
    }

    /// `path`'s working-tree content against `HEAD`, as structured hunks. `old_path` is where
    /// `HEAD` has a renamed file, as [`GitChangeEntry::old_path`] reports it; without it a
    /// rename diffs as wholly added. A path in neither side is an error; a path in both with
    /// identical content yields no hunks.
    pub fn file_diff(&self, path: &Path, old_path: Option<&Path>) -> Result<FileDiff, GitError> {
        if !crate::valid_diff_path(path) || old_path.is_some_and(|old| !crate::valid_diff_path(old))
        {
            return Err(GitError("invalid path for a diff".into()));
        }
        let repo = self.open()?;
        let mut differ = Differ::new(&repo)?;
        let content = differ.diff(path, old_path, |diff, input| {
            let hunks = UnifiedDiff::new(
                diff,
                input,
                HunkCollector::default(),
                ContextSize::symmetrical(DIFF_CONTEXT_LINES),
            )
            .consume()
            .map_err(git_error)?;
            // Two small sides can still produce more hunk text than a protocol frame holds
            // (a file of one-byte lines); the same cap applies to the output.
            let bytes = hunks
                .iter()
                .flat_map(|hunk| &hunk.lines)
                .map(|line| line.text.len() as u64 + DIFF_LINE_OVERHEAD)
                .sum::<u64>();
            if bytes > MAX_DIFF_BYTES {
                return Ok(FileDiffContent::TooLarge { bytes });
            }
            Ok(FileDiffContent::Text { hunks })
        })?;
        Ok(FileDiff {
            path: path.to_path_buf(),
            content,
        })
    }

    /// Whether every one of `paths`, relative to the work tree, is excluded by the ignore
    /// rules, so a change to them cannot change the status. `false` on any doubt.
    pub fn ignores_all<P: AsRef<Path>>(&self, paths: impl IntoIterator<Item = P>) -> bool {
        let Ok(repo) = self.open() else {
            return false;
        };
        let Ok(index) = repo.index_or_load_from_head_or_empty() else {
            return false;
        };
        let Ok(mut excludes) = repo.excludes(
            &index,
            None,
            gix::worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
        ) else {
            return false;
        };
        let mut any = false;
        for path in paths {
            let path = path.as_ref();
            any = true;
            // A deleted path has no metadata; treating it as a file only matters for a
            // directory-only ignore pattern, where a wrong answer costs one recomputation.
            let mode = if fs::metadata(self.root.join(path)).is_ok_and(|meta| meta.is_dir()) {
                gix::index::entry::Mode::DIR
            } else {
                gix::index::entry::Mode::FILE
            };
            match excludes.at_path(path, Some(mode)) {
                Ok(platform) if platform.is_excluded() => {}
                _ => return false,
            }
        }
        any
    }
}

/// The wire cost of a [`DiffLine`] beyond its text.
const DIFF_LINE_OVERHEAD: u64 = 24;

/// Later statuses win when index and worktree disagree about one path.
fn status_rank(status: GitChangeStatus) -> u8 {
    match status {
        GitChangeStatus::Modified => 0,
        GitChangeStatus::Untracked => 1,
        GitChangeStatus::Deleted => 2,
        GitChangeStatus::Added => 3,
        GitChangeStatus::Renamed => 4,
        GitChangeStatus::Conflicted => 5,
    }
}

/// `git` spells repository paths with `/`; a `PathBuf` built from them is the platform's
/// spelling and compares equal to one built from the Workspace root.
fn git_path(path: &BStr) -> PathBuf {
    gix::path::from_bstr(path).into_owned()
}

type FoldedStatus = (BString, Option<BString>, GitChangeStatus);

fn fold_status_item(item: gix::status::Item) -> Option<FoldedStatus> {
    use gix::diff::index::Change as IndexChange;
    use gix::dir::entry::{Kind, Status};
    use gix::status::index_worktree::{Item as WorktreeItem, RewriteSource};
    use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};

    Some(match item {
        gix::status::Item::TreeIndex(change) => match change {
            IndexChange::Addition { location, .. } => {
                (location.into_owned(), None, GitChangeStatus::Added)
            }
            IndexChange::Deletion { location, .. } => {
                (location.into_owned(), None, GitChangeStatus::Deleted)
            }
            IndexChange::Modification { location, .. } => {
                (location.into_owned(), None, GitChangeStatus::Modified)
            }
            IndexChange::Rewrite {
                source_location,
                location,
                ..
            } => (
                location.into_owned(),
                Some(source_location.into_owned()),
                GitChangeStatus::Renamed,
            ),
        },
        gix::status::Item::IndexWorktree(item) => match item {
            WorktreeItem::Modification {
                rela_path, status, ..
            } => {
                let status = match status {
                    EntryStatus::Conflict { .. } => GitChangeStatus::Conflicted,
                    EntryStatus::Change(Change::Removed) => GitChangeStatus::Deleted,
                    EntryStatus::Change(_) => GitChangeStatus::Modified,
                    EntryStatus::IntentToAdd => GitChangeStatus::Added,
                    EntryStatus::NeedsUpdate(_) => return None,
                };
                (rela_path, None, status)
            }
            WorktreeItem::DirectoryContents { entry, .. } => {
                if entry.status != Status::Untracked
                    || !matches!(entry.disk_kind, Some(Kind::File | Kind::Symlink))
                {
                    return None;
                }
                (entry.rela_path, None, GitChangeStatus::Untracked)
            }
            WorktreeItem::Rewrite {
                source,
                dirwalk_entry,
                ..
            } => {
                let old = match source {
                    RewriteSource::RewriteFromIndex {
                        source_rela_path, ..
                    } => source_rela_path,
                    RewriteSource::CopyFromDirectoryEntry {
                        source_dirwalk_entry,
                        ..
                    } => source_dirwalk_entry.rela_path,
                };
                (dirwalk_entry.rela_path, Some(old), GitChangeStatus::Renamed)
            }
        },
    })
}

/// Diffs `HEAD` blobs against worktree files.
struct Differ<'repo> {
    repo: &'repo gix::Repository,
    /// `None` on an unborn branch: everything is new.
    head_tree: Option<gix::Tree<'repo>>,
    cache: gix::diff::blob::Platform,
    workdir: PathBuf,
}

impl<'repo> Differ<'repo> {
    fn new(repo: &'repo gix::Repository) -> Result<Self, GitError> {
        let workdir = repo
            .workdir()
            .ok_or_else(|| GitError("repository has no work tree".into()))?
            .to_path_buf();
        let head_tree = repo.head_tree().ok();
        let cache = repo
            .diff_resource_cache(
                gix::diff::blob::pipeline::Mode::ToGit,
                gix::diff::blob::pipeline::WorktreeRoots {
                    old_root: None,
                    new_root: Some(workdir.clone()),
                },
            )
            .map_err(git_error)?;
        Ok(Self {
            repo,
            head_tree,
            cache,
            workdir,
        })
    }

    /// The `HEAD` side of `path`: its blob and mode, or `None` when HEAD lacks the path.
    fn head_entry(&self, path: &Path) -> Result<Option<(gix::ObjectId, EntryKind)>, GitError> {
        let Some(tree) = &self.head_tree else {
            return Ok(None);
        };
        let Some(entry) = tree.lookup_entry_by_path(path).map_err(git_error)? else {
            return Ok(None);
        };
        let kind = entry.mode().kind();
        if !matches!(
            kind,
            EntryKind::Blob | EntryKind::BlobExecutable | EntryKind::Link
        ) {
            return Ok(None);
        }
        Ok(Some((entry.object_id(), kind)))
    }

    /// The worktree side of `path`: its mode, or `None` when the file is gone.
    fn worktree_kind(&self, path: &Path) -> Option<EntryKind> {
        let metadata = fs::symlink_metadata(self.workdir.join(path)).ok()?;
        if metadata.file_type().is_symlink() {
            Some(EntryKind::Link)
        } else if metadata.is_file() {
            Some(EntryKind::Blob)
        } else {
            None
        }
    }

    /// Runs `render` over the internal diff of `path`, or returns the placeholder that says
    /// why there is none. `old_path` is where `HEAD` has the file when it was renamed.
    fn diff<T>(
        &mut self,
        path: &Path,
        old_path: Option<&Path>,
        render: impl FnOnce(&Diff, &InternedInput<&[u8]>) -> Result<T, GitError>,
    ) -> Result<T, GitError>
    where
        T: From<FileDiffContent>,
    {
        use gix::diff::blob::platform::prepare_diff::Operation;
        use gix::diff::blob::platform::resource::Data;

        let head = self.head_entry(old_path.unwrap_or(path))?;
        let worktree = self.worktree_kind(path);
        if head.is_none() && worktree.is_none() {
            return Err(GitError(format!(
                "{} is neither in HEAD nor in the work tree",
                path.display()
            )));
        }
        let rela_path = repository_path(path);
        let old_rela_path = old_path.map_or_else(|| rela_path.clone(), repository_path);
        let null = gix::ObjectId::null(self.repo.object_hash());
        let (old_id, old_kind) = head.unwrap_or((null, EntryKind::Blob));
        self.cache
            .set_resource(
                old_id,
                old_kind,
                old_rela_path.as_ref(),
                ResourceKind::OldOrSource,
                &self.repo.objects,
            )
            .map_err(git_error)?;
        // A null id with a worktree root reads the file from disk.
        self.cache
            .set_resource(
                null,
                worktree.unwrap_or(EntryKind::Blob),
                rela_path.as_ref(),
                ResourceKind::NewOrDestination,
                &self.repo.objects,
            )
            .map_err(git_error)?;
        let outcome = self.cache.prepare_diff().map_err(git_error)?;
        for side in [&outcome.old.data, &outcome.new.data] {
            if let Data::Binary { size } = side
                && *size > MAX_DIFF_BYTES
            {
                return Ok(FileDiffContent::TooLarge { bytes: *size }.into());
            }
        }
        match outcome.operation {
            Operation::InternalDiff { algorithm } => {
                let input = outcome.interned_input();
                let diff = Diff::compute(algorithm, &input);
                render(&diff, &input)
            }
            Operation::ExternalCommand { .. } | Operation::SourceOrDestinationIsBinary => {
                Ok(FileDiffContent::Binary.into())
            }
        }
    }

    fn stat(&mut self, entry: &GitChangeEntry) -> Result<Option<GitDiffStat>, GitError> {
        if entry.status == GitChangeStatus::Conflicted {
            return Ok(None);
        }
        let counted: StatOutcome =
            self.diff(&entry.path, entry.old_path.as_deref(), |diff, _| {
                Ok(StatOutcome(Some(GitDiffStat {
                    added: diff.count_additions(),
                    deleted: diff.count_removals(),
                })))
            })?;
        Ok(counted.0)
    }
}

/// gix wants repository paths `/`-separated whatever the platform.
fn repository_path(path: &Path) -> BString {
    gix::path::to_unix_separators_on_windows(gix::path::into_bstr(path)).into_owned()
}

/// `stat` wants counts where `file_diff` wants hunks; both get the same placeholders back.
struct StatOutcome(Option<GitDiffStat>);

impl From<FileDiffContent> for StatOutcome {
    fn from(_: FileDiffContent) -> Self {
        Self(None)
    }
}

/// Collects unified-diff hunks, numbering each line by walking the hunk: gix gives the hunk
/// header and every line's kind, not per-line numbers.
#[derive(Default)]
struct HunkCollector {
    hunks: Vec<DiffHunk>,
}

impl ConsumeHunk for HunkCollector {
    type Out = Vec<DiffHunk>;

    fn consume_hunk(
        &mut self,
        header: HunkHeader,
        lines: &[(GixLineKind, &[u8])],
    ) -> std::io::Result<()> {
        let mut old_number = header.before_hunk_start;
        let mut new_number = header.after_hunk_start;
        let lines = lines
            .iter()
            .map(|(kind, text)| {
                let text = String::from_utf8_lossy(text)
                    .trim_end_matches(['\n', '\r'])
                    .to_owned();
                let (kind, old, new) = match kind {
                    GixLineKind::Context => {
                        let numbers = (Some(old_number), Some(new_number));
                        old_number += 1;
                        new_number += 1;
                        (DiffLineKind::Context, numbers.0, numbers.1)
                    }
                    GixLineKind::Add => {
                        let number = new_number;
                        new_number += 1;
                        (DiffLineKind::Added, None, Some(number))
                    }
                    GixLineKind::Remove => {
                        let number = old_number;
                        old_number += 1;
                        (DiffLineKind::Removed, Some(number), None)
                    }
                };
                DiffLine {
                    kind,
                    old_number: old,
                    new_number: new,
                    text,
                }
            })
            .collect();
        self.hunks.push(DiffHunk {
            old_start: header.before_hunk_start,
            old_lines: header.before_hunk_len,
            new_start: header.after_hunk_start,
            new_lines: header.after_hunk_len,
            lines,
        });
        Ok(())
    }

    fn finish(self) -> Self::Out {
        self.hunks
    }
}
