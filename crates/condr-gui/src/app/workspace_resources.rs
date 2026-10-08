//! Server answers and their current requests share one lifecycle per connection.
use super::*;

type ResourcePath = (WorkspaceId, RelativePathBuf);

#[derive(Default)]
pub(super) struct WorkspaceResources {
    pub(super) diffs: HashMap<ResourcePath, Result<FileDiff, String>>,
    pub(super) diffs_generation: u64,
    pub(super) directories: HashMap<ResourcePath, Result<DirectoryListing, String>>,
    /// An unchanged answer keeps its generation and therefore its Editor and viewport.
    pub(super) files: HashMap<ResourcePath, (u64, Result<FileContent, String>)>,
    pub(super) files_generation: u64,
    pub(super) pending_diffs: HashMap<ResourcePath, u64>,
    pub(super) pending_directories: HashMap<ResourcePath, u64>,
    pub(super) pending_files: HashMap<ResourcePath, u64>,
}

impl WorkspaceResources {
    pub(super) fn clear_pending(&mut self) {
        self.pending_diffs.clear();
        self.pending_directories.clear();
        self.pending_files.clear();
    }

    pub(super) fn reset(&mut self) {
        self.clear_pending();
        self.diffs.clear();
        self.diffs_generation += 1;
        self.directories.clear();
        self.files.clear();
        self.files_generation += 1;
    }

    pub(super) fn retain_workspaces(&mut self, live: &HashSet<WorkspaceId>) {
        self.diffs.retain(|(id, _), _| live.contains(id));
        self.directories.retain(|(id, _), _| live.contains(id));
        self.files.retain(|(id, _), _| live.contains(id));
        self.pending_diffs.retain(|(id, _), _| live.contains(id));
        self.pending_directories
            .retain(|(id, _), _| live.contains(id));
        self.pending_files.retain(|(id, _), _| live.contains(id));
    }

    pub(super) fn invalidate_diffs(&mut self, workspace_id: WorkspaceId) {
        self.diffs.retain(|(id, _), _| *id != workspace_id);
        self.pending_diffs.retain(|(id, _), _| *id != workspace_id);
        self.diffs_generation += 1;
    }

    pub(super) fn fold_directory(&mut self, workspace_id: WorkspaceId, path: &RelativePath) {
        let keep = |(id, listed): &ResourcePath| *id != workspace_id || !listed.starts_with(path);
        self.directories.retain(|slot, _| keep(slot));
        self.pending_directories.retain(|slot, _| keep(slot));
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
            .retain(|(id, path), _| *id != workspace_id || shown == Some(path.as_relative_path()));
        self.pending_directories
            .retain(|(id, _), _| *id != workspace_id);
        self.pending_files.retain(|(id, _), _| *id != workspace_id);
        directories
    }

    pub(super) fn accept_diff(
        &mut self,
        slot: ResourcePath,
        request_id: u64,
        result: Result<FileDiff, String>,
    ) -> bool {
        if self.pending_diffs.get(&slot) != Some(&request_id) {
            return false;
        }
        self.pending_diffs.remove(&slot);
        self.diffs.insert(slot, result);
        self.diffs_generation += 1;
        true
    }

    pub(super) fn accept_directory(
        &mut self,
        slot: ResourcePath,
        request_id: u64,
        result: Result<DirectoryListing, String>,
    ) -> bool {
        if self.pending_directories.get(&slot) != Some(&request_id) {
            return false;
        }
        self.pending_directories.remove(&slot);
        self.directories.insert(slot, result);
        true
    }

    pub(super) fn accept_file(
        &mut self,
        slot: ResourcePath,
        request_id: u64,
        result: Result<FileContent, String>,
    ) -> bool {
        if self.pending_files.get(&slot) != Some(&request_id) {
            return false;
        }
        self.pending_files.remove(&slot);
        if self
            .files
            .get(&slot)
            .is_none_or(|(_, cached)| *cached != result)
        {
            self.files_generation += 1;
            self.files.insert(slot, (self.files_generation, result));
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
        Ok(FileContent::Text { text: value.into() })
    }

    #[test]
    fn only_the_latest_request_can_replace_an_answer_or_finish_loading() {
        let slot = (WorkspaceId::from_u64(1), RelativePathBuf::from("README.md"));
        let mut resources = WorkspaceResources::default();
        resources.pending_files.insert(slot.clone(), 10);
        assert!(resources.accept_file(slot.clone(), 10, text("original")));
        let generation = resources.files_generation;
        resources.pending_files.insert(slot.clone(), 11);
        resources.refresh_files(slot.0, Some(&slot.1));
        resources.pending_files.insert(slot.clone(), 12);
        assert!(!resources.accept_file(slot.clone(), 11, text("obsolete")));
        assert_eq!(resources.pending_files[&slot], 12);
        assert_eq!(resources.files[&slot].1, text("original"));
        assert!(resources.accept_file(slot.clone(), 12, text("original")));
        assert_eq!(resources.files_generation, generation);
        assert!(!resources.accept_file(slot.clone(), 12, text("duplicate")));
        resources.pending_files.insert(slot.clone(), 13);
        assert!(resources.accept_file(slot.clone(), 13, text("new")));
        assert_eq!(resources.files[&slot].1, text("new"));
        assert_eq!(resources.files_generation, generation + 1);
    }

    #[test]
    fn changing_diff_base_rejects_the_previous_comparison() {
        let slot = (WorkspaceId::from_u64(1), RelativePathBuf::from("main.rs"));
        let mut resources = WorkspaceResources::default();
        resources.pending_diffs.insert(slot.clone(), 20);
        resources.invalidate_diffs(slot.0);
        resources.pending_diffs.insert(slot.clone(), 21);
        assert!(!resources.accept_diff(slot.clone(), 20, Err("old HEAD result".into())));
        assert!(resources.diffs.is_empty());
        assert_eq!(resources.pending_diffs[&slot], 21);
        assert!(resources.accept_diff(slot.clone(), 21, Err("new base result".into())));
        assert_eq!(resources.diffs[&slot], Err("new base result".into()));
    }

    #[test]
    fn folded_directories_ignore_late_answers_and_refresh_only_live_listings() {
        let mut resources = WorkspaceResources::default();
        let workspace = WorkspaceId::from_u64(1);
        let root = (workspace, RelativePathBuf::new());
        let child = (workspace, RelativePathBuf::from("src/app"));
        resources.pending_directories.insert(root.clone(), 30);
        assert!(resources.accept_directory(root, 30, Err("root".into())));
        resources.pending_directories.insert(child.clone(), 31);
        resources.fold_directory(workspace, RelativePath::new("src"));
        assert!(!resources.accept_directory(child.clone(), 31, Err("late".into())));
        assert!(!resources.directories.contains_key(&child));
        assert_eq!(
            resources.refresh_files(workspace, None),
            vec![RelativePathBuf::new()]
        );
        resources.pending_directories.insert(child.clone(), 32);
        assert!(resources.accept_directory(child, 32, Err("reopened".into())));
    }

    #[test]
    fn closing_workspace_or_bootstrapping_invalidates_all_resource_kinds() {
        let mut resources = WorkspaceResources::default();
        let closed = (WorkspaceId::from_u64(1), RelativePathBuf::from("file"));
        let live = (WorkspaceId::from_u64(2), RelativePathBuf::from("file"));
        for slot in [&closed, &live] {
            resources.pending_files.insert(slot.clone(), 40);
            resources.pending_diffs.insert(slot.clone(), 41);
            resources.pending_directories.insert(slot.clone(), 42);
        }
        resources.retain_workspaces(&HashSet::from([live.0]));
        assert!(!resources.accept_file(closed.clone(), 40, text("late")));
        assert!(!resources.accept_diff(closed.clone(), 41, Err("late".into())));
        assert!(!resources.accept_directory(closed, 42, Err("late".into())));
        assert!(resources.accept_file(live.clone(), 40, text("live")));
        resources.reset();
        assert!(!resources.accept_diff(live.clone(), 41, Err("pre-bootstrap".into())));
        assert!(!resources.accept_directory(live, 42, Err("pre-bootstrap".into())));
        assert!(resources.files.is_empty());
    }
}
