use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use condr_core::{Session, create_worktree, discover_repository, open_worktree, remove_worktree};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "condr-{label}-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn git_worktree_lifecycle_preserves_branches_and_refuses_dirty_removal() {
    let temp = TempDirectory::new("git-worktrees");
    let repository = temp.path().join("repository");
    fs::create_dir_all(&repository).unwrap();
    git(&repository, ["init"]);
    git(&repository, ["config", "user.name", "Condr Tests"]);
    git(
        &repository,
        ["config", "user.email", "condr@example.invalid"],
    );
    fs::write(repository.join("README.md"), "condr\n").unwrap();
    git(&repository, ["add", "README.md"]);
    git(&repository, ["commit", "-m", "initial"]);
    git(&repository, ["branch", "-M", "main"]);
    git(&repository, ["branch", "existing"]);

    assert!(discover_repository(temp.path()).unwrap().is_none());
    let parent = discover_repository(&repository).unwrap().unwrap();
    assert_eq!(parent.branch(), Some("main"));
    assert!(!parent.is_linked_worktree());

    let worktree_root = temp.path().join("worktrees");
    let existing = create_worktree(&parent, "existing", &worktree_root).unwrap();
    assert_eq!(existing.branch(), Some("existing"));
    assert!(existing.is_linked_worktree());
    assert_eq!(open_worktree(&parent, existing.root()).unwrap(), existing);
    remove_worktree(&parent, &existing).unwrap();
    assert!(!existing.root().exists());
    assert!(git_status(
        &repository,
        ["show-ref", "--verify", "refs/heads/existing"]
    ));

    let dirty = create_worktree(&parent, "feature/dirty", &worktree_root).unwrap();
    fs::write(dirty.root().join("untracked.txt"), "keep me\n").unwrap();
    let error = remove_worktree(&parent, &dirty).unwrap_err();
    assert!(error.to_string().contains("modified or untracked"));
    assert!(dirty.root().exists());
    fs::remove_file(dirty.root().join("untracked.txt")).unwrap();
    remove_worktree(&parent, &dirty).unwrap();
    assert!(git_status(
        &repository,
        ["show-ref", "--verify", "refs/heads/feature/dirty"]
    ));
}

#[test]
fn worktree_association_survives_parent_workspace_close() {
    let mut session = Session::new();
    let parent_root = PathBuf::from("/tmp/condr-parent");
    let parent = session
        .create_workspace(parent_root.clone())
        .expect("Workspace capacity");
    let child = session
        .create_workspace(PathBuf::from("/tmp/condr-child"))
        .expect("Workspace capacity");
    assert!(session.associate_worktree(child, parent, parent_root.clone(), true));

    let restored = Session::restore(session.snapshot()).unwrap();
    let association = restored.workspace(child).unwrap().worktree().unwrap();
    assert_eq!(association.parent_workspace_id(), parent);
    assert_eq!(association.parent_root_directory(), parent_root);
    assert!(association.is_managed());

    session.close_workspace(parent).unwrap();
    assert!(session.workspace(child).unwrap().worktree().is_some());
}

#[test]
fn discovery_reports_detached_heads_and_fingerprints_branch_switches() {
    let temp = TempDirectory::new("git-head");
    let repository = temp.path().join("repository");
    fs::create_dir_all(&repository).unwrap();
    git(&repository, ["init"]);
    git(&repository, ["config", "user.name", "Condr Tests"]);
    git(
        &repository,
        ["config", "user.email", "condr@example.invalid"],
    );
    git(&repository, ["branch", "-M", "main"]);
    // Unborn branch: no commits yet, but HEAD already names it.
    assert_eq!(
        discover_repository(&repository).unwrap().unwrap().branch(),
        Some("main")
    );
    fs::write(repository.join("README.md"), "condr\n").unwrap();
    git(&repository, ["add", "README.md"]);
    git(&repository, ["commit", "-m", "initial"]);

    let on_main = discover_repository(&repository).unwrap().unwrap();
    let before = on_main.head_fingerprint().unwrap();
    // Committing on the same branch does not rewrite HEAD.
    fs::write(repository.join("README.md"), "again\n").unwrap();
    git(&repository, ["commit", "-am", "second"]);
    assert_eq!(on_main.head_fingerprint().unwrap(), before);

    std::thread::sleep(std::time::Duration::from_millis(20));
    git(&repository, ["checkout", "--detach"]);
    let detached = discover_repository(&repository).unwrap().unwrap();
    assert_eq!(detached.branch(), None);
    assert_ne!(detached.head_fingerprint().unwrap(), before);
    assert!(
        discover_repository(repository.join(".git"))
            .unwrap()
            .is_none()
    );
}

fn git<const N: usize>(cwd: &Path, args: [&str; N]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_status<const N: usize>(cwd: &Path, args: [&str; N]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .status()
        .unwrap()
        .success()
}
