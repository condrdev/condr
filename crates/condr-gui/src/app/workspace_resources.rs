//! Server answers and their current requests share one lifecycle per connection.
use super::*;

type ResourcePath = (WorkspaceId, RelativePathBuf);

/// Answers from the Server by key, and the request each key still waits on: only the
/// latest request's answer lands, so a late answer to an older one is dropped.
pub(super) struct Fetched<K, V> {
    answers: HashMap<K, V>,
    pending: HashMap<K, u64>,
}

impl<K, V> Default for Fetched<K, V> {
    fn default() -> Self {
        Self {
            answers: HashMap::new(),
            pending: HashMap::new(),
        }
    }
}

impl<K: Eq + std::hash::Hash, V> Fetched<K, V> {
    pub(super) fn get(&self, key: &K) -> Option<&V> {
        self.answers.get(key)
    }

    pub(super) fn is_pending(&self, key: &K) -> bool {
        self.pending.contains_key(key)
    }

    /// Neither answered nor asked for, so it is still to be asked.
    pub(super) fn wants(&self, key: &K) -> bool {
        !self.answers.contains_key(key) && !self.pending.contains_key(key)
    }

    pub(super) fn asked(&mut self, key: K, request_id: u64) {
        self.pending.insert(key, request_id);
    }

    /// Whether `request_id` is what `key` waits on; it then waits no more.
    fn settle(&mut self, key: &K, request_id: u64) -> bool {
        if self.pending.get(key) != Some(&request_id) {
            return false;
        }
        self.pending.remove(key);
        true
    }

    /// Keeps `answer` when it answers the request `key` waits on.
    pub(super) fn land(&mut self, key: K, request_id: u64, answer: V) -> bool {
        if !self.settle(&key, request_id) {
            return false;
        }
        self.answers.insert(key, answer);
        true
    }

    pub(super) fn keys(&self) -> impl Iterator<Item = &K> {
        self.answers.keys()
    }

    #[cfg(test)]
    pub(super) fn pending_keys(&self) -> impl Iterator<Item = &K> {
        self.pending.keys()
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.answers.is_empty()
    }

    pub(super) fn retain(&mut self, keep: impl Fn(&K) -> bool) {
        self.answers.retain(|key, _| keep(key));
        self.pending.retain(|key, _| keep(key));
    }

    fn clear_pending(&mut self) {
        self.pending.clear();
    }

    fn clear(&mut self) {
        self.answers.clear();
        self.pending.clear();
    }
}

#[derive(Default)]
pub(super) struct WorkspaceResources {
    pub(super) diffs: Fetched<ResourcePath, Result<FileDiff, String>>,
    pub(super) diffs_generation: u64,
    pub(super) directories: Fetched<ResourcePath, Result<DirectoryListing, String>>,
    /// An unchanged answer keeps its generation and therefore its Editor and viewport.
    pub(super) files: Fetched<ResourcePath, (u64, Result<FileContent, String>)>,
    pub(super) files_generation: u64,
    /// Each Workspace's History as far as it was loaded (ADR 0046).
    pub(super) logs: HashMap<WorkspaceId, GitLog>,
    /// The page each Workspace waits for: its request and how many commits it skips.
    pub(super) pending_logs: HashMap<WorkspaceId, (u64, u32)>,
    /// Unfolded commits; a commit never changes, so only a reset drops them.
    pub(super) commits: Fetched<(WorkspaceId, String), Result<CommitDetails, String>>,
    /// One file of one commit, as its Diff Tab shows it.
    pub(super) commit_diffs: HashMap<CommitPath, Result<FileDiff, String>>,
    /// The `GitDiff` requests for them; the reply names only the path.
    pub(super) pending_commit_diffs: HashMap<u64, CommitPath>,
}

/// A Workspace, a commit id and a path in that commit.
pub(super) type CommitPath = (WorkspaceId, String, RelativePathBuf);

impl WorkspaceResources {
    pub(super) fn clear_pending(&mut self) {
        self.diffs.clear_pending();
        self.directories.clear_pending();
        self.files.clear_pending();
        self.pending_logs.clear();
        self.commits.clear_pending();
        self.pending_commit_diffs.clear();
    }

    pub(super) fn reset(&mut self) {
        self.clear_pending();
        self.diffs.clear();
        self.diffs_generation += 1;
        self.directories.clear();
        self.files.clear();
        self.files_generation += 1;
        self.logs.clear();
        self.commits.clear();
        self.commit_diffs.clear();
    }

    pub(super) fn retain_workspaces(&mut self, live: &HashSet<WorkspaceId>) {
        self.diffs.retain(|(id, _)| live.contains(id));
        self.directories.retain(|(id, _)| live.contains(id));
        self.files.retain(|(id, _)| live.contains(id));
        self.logs.retain(|id, _| live.contains(id));
        self.pending_logs.retain(|id, _| live.contains(id));
        self.commits.retain(|(id, _)| live.contains(id));
        self.commit_diffs.retain(|(id, _, _), _| live.contains(id));
        self.pending_commit_diffs
            .retain(|_, (id, _, _)| live.contains(id));
    }

    pub(super) fn invalidate_diffs(&mut self, workspace_id: WorkspaceId) {
        self.diffs.retain(|(id, _)| *id != workspace_id);
        self.diffs_generation += 1;
    }

    pub(super) fn fold_directory(&mut self, workspace_id: WorkspaceId, path: &RelativePath) {
        self.directories
            .retain(|(id, listed)| *id != workspace_id || !listed.starts_with(path));
    }

    /// Retain visible contents during refresh; invalidate in-flight answers before
    /// issuing replacements, including requests whose first answer has not arrived.
    pub(super) fn refresh_files(
        &mut self,
        workspace_id: WorkspaceId,
        shown: Option<&RelativePath>,
    ) -> Vec<RelativePathBuf> {
        let directories = self
            .directories
            .keys()
            .filter(|(id, _)| *id == workspace_id)
            .map(|(_, path)| path.clone())
            .collect();
        self.files
            .answers
            .retain(|(id, path), _| *id != workspace_id || shown == Some(path.as_relative_path()));
        self.directories
            .pending
            .retain(|(id, _), _| *id != workspace_id);
        self.files.pending.retain(|(id, _), _| *id != workspace_id);
        directories
    }

    /// A `GitDiff` answer: one file of one commit when its request asked for that
    /// (ADR 0046), otherwise a file of the Workspace against its comparison.
    pub(super) fn accept_git_diff(
        &mut self,
        request_id: u64,
        workspace_id: WorkspaceId,
        path: RelativePathBuf,
        result: Result<FileDiff, String>,
    ) -> bool {
        if let Some(slot) = self.pending_commit_diffs.remove(&request_id) {
            if slot.0 != workspace_id || slot.2 != path {
                return false;
            }
            self.commit_diffs.insert(slot, result);
            return true;
        }
        if !self.diffs.land((workspace_id, path), request_id, result) {
            return false;
        }
        self.diffs_generation += 1;
        true
    }

    pub(super) fn accept_commit(
        &mut self,
        slot: (WorkspaceId, String),
        request_id: u64,
        result: Result<CommitDetails, String>,
    ) -> bool {
        self.commits.land(slot, request_id, result)
    }

    /// A History page: the first replaces what is shown, a later one extends it when it
    /// walked from the same commit.
    pub(super) fn accept_log(
        &mut self,
        workspace_id: WorkspaceId,
        request_id: u64,
        result: Result<GitLog, String>,
    ) -> bool {
        let Some((pending, skip)) = self.pending_logs.get(&workspace_id).copied() else {
            return false;
        };
        if pending != request_id {
            return false;
        }
        self.pending_logs.remove(&workspace_id);
        // ponytail: a failed page shows as the end of History; the next refresh asks again.
        let page = result.unwrap_or_default();
        match self.logs.get_mut(&workspace_id) {
            Some(log) if skip > 0 && log.from == page.from => {
                log.commits.extend(page.commits);
                log.more = page.more;
            }
            _ if skip > 0 => {}
            _ => {
                self.logs.insert(workspace_id, page);
            }
        }
        true
    }

    pub(super) fn accept_directory(
        &mut self,
        slot: ResourcePath,
        request_id: u64,
        result: Result<DirectoryListing, String>,
    ) -> bool {
        self.directories.land(slot, request_id, result)
    }

    pub(super) fn accept_file(
        &mut self,
        slot: ResourcePath,
        request_id: u64,
        result: Result<FileContent, String>,
    ) -> bool {
        if !self.files.settle(&slot, request_id) {
            return false;
        }
        if self
            .files
            .get(&slot)
            .is_none_or(|(_, cached)| *cached != result)
        {
            self.files_generation += 1;
            self.files
                .answers
                .insert(slot, (self.files_generation, result));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FileContent, HashSet, RelativePath, RelativePathBuf, WorkspaceId, WorkspaceResources,
    };

    fn text(value: &str) -> Result<FileContent, String> {
        Ok(FileContent::Text {
            text: value.into(),
            version: None,
            lossy: false,
        })
    }

    #[test]
    fn only_the_latest_request_can_replace_an_answer_or_finish_loading() {
        let slot = (WorkspaceId::from_u64(1), RelativePathBuf::from("README.md"));
        let mut resources = WorkspaceResources::default();
        resources.files.asked(slot.clone(), 10);
        assert!(resources.accept_file(slot.clone(), 10, text("original")));
        let generation = resources.files_generation;
        resources.files.asked(slot.clone(), 11);
        resources.refresh_files(slot.0, Some(&slot.1));
        resources.files.asked(slot.clone(), 12);
        assert!(!resources.accept_file(slot.clone(), 11, text("obsolete")));
        assert!(resources.files.is_pending(&slot));
        assert_eq!(resources.files.get(&slot).unwrap().1, text("original"));
        assert!(resources.accept_file(slot.clone(), 12, text("original")));
        assert_eq!(resources.files_generation, generation);
        assert!(!resources.accept_file(slot.clone(), 12, text("duplicate")));
        resources.files.asked(slot.clone(), 13);
        assert!(resources.accept_file(slot.clone(), 13, text("new")));
        assert_eq!(resources.files.get(&slot).unwrap().1, text("new"));
        assert_eq!(resources.files_generation, generation + 1);
    }

    #[test]
    fn changing_diff_base_rejects_the_previous_comparison() {
        let slot = (WorkspaceId::from_u64(1), RelativePathBuf::from("main.rs"));
        let mut resources = WorkspaceResources::default();
        resources.diffs.asked(slot.clone(), 20);
        resources.invalidate_diffs(slot.0);
        resources.diffs.asked(slot.clone(), 21);
        assert!(!resources.accept_git_diff(
            20,
            slot.0,
            slot.1.clone(),
            Err("old HEAD result".into())
        ));
        assert!(resources.diffs.is_empty());
        assert!(resources.diffs.is_pending(&slot));
        assert!(resources.accept_git_diff(
            21,
            slot.0,
            slot.1.clone(),
            Err("new base result".into())
        ));
        assert_eq!(
            resources.diffs.get(&slot),
            Some(&Err("new base result".into()))
        );
    }

    #[test]
    fn folded_directories_ignore_late_answers_and_refresh_only_live_listings() {
        let mut resources = WorkspaceResources::default();
        let workspace = WorkspaceId::from_u64(1);
        let root = (workspace, RelativePathBuf::new());
        let child = (workspace, RelativePathBuf::from("src/app"));
        resources.directories.asked(root.clone(), 30);
        assert!(resources.accept_directory(root, 30, Err("root".into())));
        resources.directories.asked(child.clone(), 31);
        resources.fold_directory(workspace, RelativePath::new("src"));
        assert!(!resources.accept_directory(child.clone(), 31, Err("late".into())));
        assert!(resources.directories.get(&child).is_none());
        assert_eq!(
            resources.refresh_files(workspace, None),
            vec![RelativePathBuf::new()]
        );
        resources.directories.asked(child.clone(), 32);
        assert!(resources.accept_directory(child, 32, Err("reopened".into())));
    }

    #[test]
    fn closing_workspace_or_bootstrapping_invalidates_all_resource_kinds() {
        let mut resources = WorkspaceResources::default();
        let closed = (WorkspaceId::from_u64(1), RelativePathBuf::from("file"));
        let live = (WorkspaceId::from_u64(2), RelativePathBuf::from("file"));
        for slot in [&closed, &live] {
            resources.files.asked(slot.clone(), 40);
            resources.diffs.asked(slot.clone(), 41);
            resources.directories.asked(slot.clone(), 42);
        }
        resources.retain_workspaces(&HashSet::from([live.0]));
        assert!(!resources.accept_file(closed.clone(), 40, text("late")));
        assert!(!resources.accept_git_diff(41, closed.0, closed.1.clone(), Err("late".into())));
        assert!(!resources.accept_directory(closed, 42, Err("late".into())));
        assert!(resources.accept_file(live.clone(), 40, text("live")));
        resources.reset();
        assert!(!resources.accept_git_diff(
            41,
            live.0,
            live.1.clone(),
            Err("pre-bootstrap".into())
        ));
        assert!(!resources.accept_directory(live, 42, Err("pre-bootstrap".into())));
        assert!(resources.files.is_empty());
    }

    #[test]
    fn commits_and_their_diffs_take_only_the_latest_answer() {
        let mut resources = WorkspaceResources::default();
        let workspace = WorkspaceId::from_u64(1);
        let commit = (workspace, "abc".to_owned());
        resources.commits.asked(commit.clone(), 50);
        resources.commits.asked(commit.clone(), 51);
        assert!(!resources.accept_commit(commit.clone(), 50, Err("old".into())));
        assert!(resources.accept_commit(commit.clone(), 51, Err("new".into())));
        let path = RelativePathBuf::from("main.rs");
        let commit_path = (workspace, "abc".to_owned(), path.clone());
        resources
            .pending_commit_diffs
            .insert(52, commit_path.clone());
        let other = RelativePathBuf::from("other.rs");
        assert!(!resources.accept_git_diff(52, workspace, other, Err("wrong path".into())));
        resources.pending_commit_diffs.insert(53, commit_path);
        assert!(resources.accept_git_diff(53, workspace, path, Err("diff".into())));
        assert!(
            resources.diffs.is_empty(),
            "a commit's diff is not the Workspace's"
        );
        resources.retain_workspaces(&HashSet::new());
        assert!(resources.commits.get(&commit).is_none());
        assert!(resources.commit_diffs.is_empty());
    }
}
