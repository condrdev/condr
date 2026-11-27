//! A Workspace's log (ADR 0046, ADR 0079): the commits reachable from `HEAD`, every ref or
//! one, in date order, and one commit's files and diffs against its first parent.

use std::collections::{BTreeMap, HashMap, HashSet};

use relative_path::{RelativePath, RelativePathBuf};

use super::changes::*;
use super::repository::{GitError, GitRepository, git_error};
use super::status::{DIFF_CONTEXT_LINES, Differ, git_path, hunks_content};
use gix::bstr::ByteSlice as _;

impl GitRepository {
    /// Up to `limit` commits after skipping `skip`. From `HEAD`, as Git History walks, newest
    /// commit time first, which stops at the page (ADR 0046); from every ref or one, as Git
    /// Graph walks, in `git rev-list --date-order`'s order, no parent before all its
    /// children, which first visits everything the tips reach (ADR 0079). A first page walks
    /// from `start` and returns where it started; a later one names those commits in `from`,
    /// so it stays put while refs move. With `text`, only the commits whose message holds
    /// it, ignoring case, or whose id starts with one of its words of 7 to 64 hex digits, as
    /// IntelliJ's log filters. Each page walks again from the start.
    pub fn log(
        &self,
        from: &[String],
        start: &LogStart,
        text: Option<&str>,
        skip: u32,
        limit: u32,
    ) -> Result<GitLog, GitError> {
        let repo = self.open()?;
        let head = repo.head_id().ok().map(|id| id.detach());
        let first_page = from.is_empty();
        let refs = if first_page && *start != LogStart::Head {
            refs(&repo)?
        } else {
            Vec::new()
        };
        let mut tips = Vec::new();
        if first_page {
            match start {
                LogStart::Head => tips.extend(head),
                LogStart::All => {
                    for reference in &refs {
                        tips.push(commit_id(&reference.commit)?);
                    }
                }
                LogStart::Ref(name) => tips.push(
                    repo.find_reference(name.as_str())
                        .map_err(git_error)?
                        .peel_to_commit()
                        .map_err(git_error)?
                        .id,
                ),
            }
        } else {
            for id in from {
                tips.push(commit_id(id)?);
            }
        }
        let mut seen = HashSet::new();
        tips.retain(|id| seen.insert(*id));
        let mut log = GitLog {
            from: tips.iter().map(ToString::to_string).collect(),
            user_email: (first_page && *start != LogStart::Head)
                .then(|| repo.config_snapshot().string("user.email"))
                .flatten()
                .map(|email| email.to_string()),
            refs,
            ..GitLog::default()
        };
        // An unborn `HEAD` has no log yet.
        if tips.is_empty() {
            return Ok(log);
        }
        let walk: Box<dyn Iterator<Item = Result<Walked, GitError>>> = if *start == LogStart::Head {
            Box::new(
                repo.rev_walk(tips)
                    .sorting(gix::revision::walk::Sorting::ByCommitTime(
                        gix::traverse::commit::simple::CommitTimeOrder::NewestFirst,
                    ))
                    .all()
                    .map_err(git_error)?
                    .map(|info| {
                        info.map(|info| (info.id, info.parent_ids.to_vec()))
                            .map_err(git_error)
                    }),
            )
        } else {
            Box::new(date_order(&repo, tips)?)
        };
        let filter = text.map(TextFilter::new);
        let mut skipped = 0;
        for walked in walk {
            let (id, parents) = walked?;
            let commit = repo.find_commit(id).map_err(git_error)?;
            if let Some(filter) = &filter
                && !filter.matches(&id, &commit)
            {
                continue;
            }
            if skipped < skip {
                skipped += 1;
                continue;
            }
            if log.commits.len() == limit as usize {
                log.more = true;
                break;
            }
            let author = commit.author().map_err(git_error)?;
            let committer = commit.committer().map_err(git_error)?;
            log.commits.push(CommitSummary {
                id: id.to_string(),
                subject: commit.message().map_err(git_error)?.summary().to_string(),
                author: author.name.to_string(),
                time: author.time().map_err(git_error)?.seconds,
                parents: parents.iter().map(ToString::to_string).collect(),
                author_email: author.email.to_string(),
                committer_differs: author.name != committer.name || author.email != committer.email,
            });
        }
        Ok(log)
    }

    /// The local and remote branches whose tips reach commit `id` (ADR 0079), by full name:
    /// one walk from every tip, each commit carrying the set of tips that reach it down to
    /// its parents, until `id` comes up; children come first, so its set is whole by then.
    pub fn commit_branches(&self, id: &str) -> Result<Vec<String>, GitError> {
        let repo = self.open()?;
        let target = commit_id(id)?;
        let branches: Vec<GitRef> = refs(&repo)?
            .into_iter()
            .filter(|reference| {
                reference.name.starts_with("refs/heads/")
                    || reference.name.starts_with("refs/remotes/")
            })
            .collect();
        let words = branches.len().div_ceil(64);
        let mut reach: HashMap<gix::ObjectId, Vec<u64>> = HashMap::new();
        for (index, branch) in branches.iter().enumerate() {
            reach
                .entry(commit_id(&branch.commit)?)
                .or_insert_with(|| vec![0; words])[index / 64] |= 1 << (index % 64);
        }
        let tips: Vec<gix::ObjectId> = reach.keys().copied().collect();
        if tips.is_empty() {
            return Ok(Vec::new());
        }
        for walked in date_order(&repo, tips)? {
            let (id, parents) = walked?;
            let Some(bits) = reach.remove(&id) else {
                continue;
            };
            if id == target {
                return Ok(branches
                    .into_iter()
                    .enumerate()
                    .filter(|(index, _)| bits[index / 64] & (1 << (index % 64)) != 0)
                    .map(|(_, branch)| branch.name)
                    .collect());
            }
            for parent in parents {
                let set = reach.entry(parent).or_insert_with(|| vec![0; words]);
                for (word, bit) in set.iter_mut().zip(&bits) {
                    *word |= bit;
                }
            }
        }
        Ok(Vec::new())
    }

    /// `id`'s message and the files under the Root it changed against its first parent,
    /// renames tracked as the repository configures them, with line counts unless the list
    /// is truncated.
    pub fn commit(&self, id: &str) -> Result<CommitDetails, GitError> {
        let repo = self.open()?;
        let commit = repo.find_commit(commit_id(id)?).map_err(git_error)?;
        let mut entries = changed_entries(&repo, &commit)?;
        entries.retain(|path, _| self.to_workspace(path).is_some());
        let truncated = entries.len() > MAX_GIT_CHANGES;
        let mut entries: Vec<GitChangeEntry> =
            entries.into_values().take(MAX_GIT_CHANGES).collect();
        if !truncated {
            let mut differ = Differ::commit(&repo, &commit)?;
            for entry in &mut entries {
                entry.stat = differ.stat(entry).ok().flatten();
            }
        }
        let author = commit.author().map_err(git_error)?;
        let committer = commit.committer().map_err(git_error)?;
        Ok(CommitDetails {
            id: commit.id.to_string(),
            parents: commit.parent_ids().map(|id| id.to_string()).collect(),
            author: author.name.to_string(),
            time: author.time().map_err(git_error)?.seconds,
            message: commit.message_raw_sloppy().to_string(),
            changes: self.workspace_changes(GitChanges { entries, truncated }),
            author_email: author.email.to_string(),
            committer: (author.name != committer.name || author.email != committer.email)
                .then(|| {
                    Ok::<_, GitError>(CommitPerson {
                        name: committer.name.to_string(),
                        email: committer.email.to_string(),
                        time: committer.time().map_err(git_error)?.seconds,
                    })
                })
                .transpose()?,
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
        let (repository_path, _) = self.diff_paths(path, None)?;
        let repo = self.open()?;
        let commit = repo.find_commit(commit_id(id)?).map_err(git_error)?;
        let old_path = changed_entries(&repo, &commit)?
            .remove(&repository_path)
            .and_then(|entry| entry.old_path);
        let mut differ = Differ::commit(&repo, &commit)?;
        let content = differ.diff(&repository_path, old_path.as_deref(), |diff, input| {
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

/// A commit the walk reached, and its parents.
type Walked = (gix::ObjectId, Vec<gix::ObjectId>);

/// `git rev-list --date-order` from `tips`: no parent before all its children, otherwise
/// newest commit time first. It first visits everything the tips reach, faster with a
/// commit-graph file.
fn date_order(
    repo: &gix::Repository,
    tips: Vec<gix::ObjectId>,
) -> Result<impl Iterator<Item = Result<Walked, GitError>> + '_, GitError> {
    Ok(gix::traverse::commit::topo::Builder::from_iters(
        &repo.objects,
        tips,
        None::<Vec<gix::ObjectId>>,
    )
    .sorting(gix::traverse::commit::topo::Sorting::DateOrder)
    .with_commit_graph(repo.commit_graph_if_enabled().ok().flatten())
    .build()
    .map_err(git_error)?
    .map(|info| {
        info.map(|info| (info.id, info.parent_ids.to_vec()))
            .map_err(git_error)
    }))
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

/// `HEAD` and every branch and tag, each peeled to its commit; a remote's symbolic `HEAD`
/// and refs that name no commit are left out.
fn refs(repo: &gix::Repository) -> Result<Vec<GitRef>, GitError> {
    let mut refs = Vec::new();
    if let Ok(id) = repo.head_id() {
        refs.push(GitRef {
            name: "HEAD".into(),
            commit: id.to_string(),
        });
    }
    let platform = repo.references().map_err(git_error)?;
    for reference in platform.all().map_err(git_error)? {
        let mut reference = reference.map_err(git_error)?;
        let name = reference.name().as_bstr().to_string();
        let kept = ["refs/heads/", "refs/remotes/", "refs/tags/"]
            .iter()
            .any(|prefix| name.starts_with(prefix));
        if !kept || (name.starts_with("refs/remotes/") && name.ends_with("/HEAD")) {
            continue;
        }
        if let Ok(commit) = reference.peel_to_commit() {
            refs.push(GitRef {
                name,
                commit: commit.id.to_string(),
            });
        }
    }
    Ok(refs)
}

/// The text filter IntelliJ's log has (ADR 0079): the full message holding the text, ignoring
/// case, or the id starting with one of its words of 7 to 64 hex digits.
struct TextFilter {
    text: String,
    ids: Vec<String>,
}

impl TextFilter {
    fn new(text: &str) -> Self {
        Self {
            text: text.to_lowercase(),
            ids: text
                .split_whitespace()
                .filter(|word| {
                    (7..=64).contains(&word.len()) && word.chars().all(|c| c.is_ascii_hexdigit())
                })
                .map(str::to_ascii_lowercase)
                .collect(),
        }
    }

    fn matches(&self, id: &gix::oid, commit: &gix::Commit<'_>) -> bool {
        let hex = id.to_string();
        self.ids
            .iter()
            .any(|prefix| hex.starts_with(prefix.as_str()))
            || commit
                .message_raw_sloppy()
                .to_str_lossy()
                .to_lowercase()
                .contains(&self.text)
    }
}
