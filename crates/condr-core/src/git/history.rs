//! A Workspace's History (ADR 0046): the commits reachable from `HEAD`, newest commit time
//! first as `git log` lists them, and one commit's files and diffs against its first parent.

use std::collections::BTreeMap;

use relative_path::{RelativePath, RelativePathBuf};

use super::changes::*;
use super::repository::{GitError, GitRepository, git_error};
use super::status::{DIFF_CONTEXT_LINES, Differ, git_path, hunks_content};

impl GitRepository {
    /// Up to `limit` commits after skipping `skip`, walking from `from`, or from `HEAD` when
    /// `from` is `None`. Each page walks again from the start; that stays cheap into the tens
    /// of thousands of commits.
    pub fn log(&self, from: Option<&str>, skip: u32, limit: u32) -> Result<GitLog, GitError> {
        let repo = self.open()?;
        let start = match from {
            Some(id) => commit_id(id)?,
            None => match repo.head_id() {
                Ok(id) => id.detach(),
                // An unborn `HEAD` has no History yet.
                Err(_) => return Ok(GitLog::default()),
            },
        };
        let walk = repo
            .rev_walk([start])
            .sorting(gix::revision::walk::Sorting::ByCommitTime(
                gix::traverse::commit::simple::CommitTimeOrder::NewestFirst,
            ))
            .all()
            .map_err(git_error)?;
        let mut log = GitLog {
            from: Some(start.to_string()),
            ..GitLog::default()
        };
        for info in walk.skip(skip as usize) {
            let info = info.map_err(git_error)?;
            if log.commits.len() == limit as usize {
                log.more = true;
                break;
            }
            let commit = info.object().map_err(git_error)?;
            let (author, time) = author(&commit)?;
            log.commits.push(CommitSummary {
                id: info.id.to_string(),
                subject: commit.message().map_err(git_error)?.summary().to_string(),
                author,
                time,
            });
        }
        Ok(log)
    }

    /// `id`'s message and the files it changed against its first parent, renames tracked as
    /// the repository configures them, with line counts unless the list is truncated.
    pub fn commit(&self, id: &str) -> Result<CommitDetails, GitError> {
        let repo = self.open()?;
        let commit = repo.find_commit(commit_id(id)?).map_err(git_error)?;
        let entries = changed_entries(&repo, &commit)?;
        let truncated = entries.len() > MAX_GIT_CHANGES;
        let mut entries: Vec<GitChangeEntry> =
            entries.into_values().take(MAX_GIT_CHANGES).collect();
        if !truncated {
            let mut differ = Differ::commit(&repo, &commit)?;
            for entry in &mut entries {
                entry.stat = differ.stat(entry).ok().flatten();
            }
        }
        let (author, time) = author(&commit)?;
        Ok(CommitDetails {
            id: commit.id.to_string(),
            parents: commit.parent_ids().map(|id| id.to_string()).collect(),
            author,
            time,
            message: commit.message_raw_sloppy().to_string(),
            changes: GitChanges { entries, truncated },
        })
    }

    /// `path` as commit `id` changed it, against its first parent; a renamed file against
    /// where the parent had it.
    pub fn commit_file_diff(
        &self,
        id: &str,
        path: &RelativePath,
        full_context: bool,
    ) -> Result<FileDiff, GitError> {
        if !crate::valid_diff_path(path) {
            return Err(GitError("invalid path for a diff".into()));
        }
        let repo = self.open()?;
        let commit = repo.find_commit(commit_id(id)?).map_err(git_error)?;
        let old_path = changed_entries(&repo, &commit)?
            .remove(path)
            .and_then(|entry| entry.old_path);
        let mut differ = Differ::commit(&repo, &commit)?;
        let content = differ.diff(path, old_path.as_deref(), |diff, input| {
            if full_context {
                let lines = input.before.len().max(input.after.len());
                let content = hunks_content(diff, input, u32::try_from(lines).unwrap_or(u32::MAX))?;
                if !matches!(content, FileDiffContent::TooLarge { .. }) {
                    return Ok(content);
                }
            }
            hunks_content(diff, input, DIFF_CONTEXT_LINES)
        })?;
        Ok(FileDiff {
            path: path.to_relative_path_buf(),
            content,
        })
    }
}

/// The files `commit` changed against its first parent, without line counts.
fn changed_entries(
    repo: &gix::Repository,
    commit: &gix::Commit<'_>,
) -> Result<BTreeMap<RelativePathBuf, GitChangeEntry>, GitError> {
    let parent_tree = match commit.parent_ids().next() {
        Some(parent) => Some(
            parent
                .object()
                .map_err(git_error)?
                .peel_to_tree()
                .map_err(git_error)?,
        ),
        None => None,
    };
    let tree = commit.tree().map_err(git_error)?;
    let mut entries = BTreeMap::new();
    for change in repo
        .diff_tree_to_tree(parent_tree.as_ref(), &tree, None)
        .map_err(git_error)?
    {
        use gix::object::tree::diff::ChangeDetached as Change;
        let (location, old, status, mode) = match change {
            Change::Addition {
                location,
                entry_mode,
                ..
            } => (location, None, GitChangeStatus::Added, entry_mode),
            Change::Deletion {
                location,
                entry_mode,
                ..
            } => (location, None, GitChangeStatus::Deleted, entry_mode),
            Change::Modification {
                location,
                entry_mode,
                ..
            } => (location, None, GitChangeStatus::Modified, entry_mode),
            Change::Rewrite {
                source_location,
                location,
                entry_mode,
                copy,
                ..
            } => {
                // A copy leaves its source in place, so it reads as a new file.
                let (old, status) = if copy {
                    (None, GitChangeStatus::Added)
                } else {
                    (Some(source_location), GitChangeStatus::Renamed)
                };
                (location, old, status, entry_mode)
            }
        };
        if mode.is_tree() {
            continue;
        }
        let path = git_path(location.as_ref());
        entries.insert(
            path.clone(),
            GitChangeEntry {
                path,
                old_path: old.map(|old| git_path(old.as_ref())),
                status,
                stat: None,
            },
        );
    }
    Ok(entries)
}

fn commit_id(id: &str) -> Result<gix::ObjectId, GitError> {
    gix::ObjectId::from_hex(id.as_bytes()).map_err(git_error)
}

fn author(commit: &gix::Commit<'_>) -> Result<(String, i64), GitError> {
    let author = commit.author().map_err(git_error)?;
    let time = author.time().map_err(git_error)?;
    Ok((author.name.to_string(), time.seconds))
}
