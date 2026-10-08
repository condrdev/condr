use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use condr_core::{
    GitUpstream, Session, create_worktree, discover_repository, open_worktree, remove_worktree,
    worktree_destination,
};

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
        // `discover_repository` reports one canonical spelling per directory; on macOS the
        // temp dir is reached through the `/var` -> `/private/var` symlink. Resolve it the
        // way discovery does: `fs::canonicalize` would give Windows a `\\?\` verbatim path.
        Self(gix::path::realpath(&path).unwrap())
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

    // Without a configured root the checkout lands beside the repository, on the same volume.
    let (beside, forked_from) = create_worktree(&parent, "feature/beside", None).unwrap();
    // A branch Condr made remembers where it forked (ADR 0034).
    assert_eq!(forked_from.as_deref(), Some("main"));
    assert_eq!(
        beside.root(),
        temp.path()
            .join("repository.worktrees")
            .join("feature-beside")
    );
    remove_worktree(&parent, &beside).unwrap();

    let worktree_root = temp.path().join("worktrees");
    let (existing, forked_from) =
        create_worktree(&parent, "existing", Some(&worktree_root)).unwrap();
    assert_eq!(
        forked_from, None,
        "a branch that existed says nothing about its fork"
    );
    assert_eq!(
        existing.root(),
        worktree_root.join("repository").join("existing")
    );
    assert_eq!(existing.branch(), Some("existing"));
    assert!(existing.is_linked_worktree());
    assert_eq!(open_worktree(&parent, existing.root()).unwrap(), existing);
    // gix wrote the worktree; git must read it back as its own: listed, clean, on the branch.
    let listed = git_stdout(&repository, ["worktree", "list", "--porcelain"]);
    assert!(
        listed.contains("branch refs/heads/existing"),
        "git should list the worktree: {listed}"
    );
    assert!(git_stdout(existing.root(), ["status", "--porcelain"]).is_empty());
    assert!(existing.root().join("README.md").is_file());
    let duplicate = create_worktree(&parent, "existing", None);
    assert!(
        duplicate
            .unwrap_err()
            .to_string()
            .contains("already checked out")
    );
    remove_worktree(&parent, &existing).unwrap();
    assert!(!existing.root().exists());
    assert!(!git_stdout(&repository, ["worktree", "list", "--porcelain"]).contains("existing"));
    assert!(git_status(
        &repository,
        ["show-ref", "--verify", "refs/heads/existing"]
    ));

    let (dirty, _) = create_worktree(&parent, "feature/dirty", Some(&worktree_root)).unwrap();
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
    assert!(session.associate_worktree(
        child,
        parent,
        parent_root.clone(),
        true,
        Some("main".into())
    ));

    let restored = Session::restore(session.snapshot()).unwrap();
    let association = restored.workspace(child).unwrap().worktree().unwrap();
    assert_eq!(association.parent_workspace_id(), parent);
    assert_eq!(association.parent_root_directory(), parent_root);
    assert!(association.is_managed());
    assert_eq!(association.base_branch(), Some("main"));

    session.close_workspace(parent).unwrap();
    assert!(session.workspace(child).unwrap().worktree().is_some());
}

fn empty_repository(path: &Path, tree: Option<gix::ObjectId>) -> condr_core::GitRepository {
    let repo = gix::init(path).unwrap();
    let tree = tree.unwrap_or_else(|| {
        repo.write_object(gix::objs::Tree::empty())
            .unwrap()
            .detach()
    });
    let author = gix::actor::SignatureRef {
        name: "Condr Tests".into(),
        email: "condr@example.invalid".into(),
        time: "0 +0000",
    };
    repo.commit_as(
        author,
        author,
        "HEAD",
        "initial",
        tree,
        std::iter::empty::<gix::ObjectId>(),
    )
    .unwrap();
    discover_repository(path).unwrap().unwrap()
}

#[test]
fn failed_worktree_registration_removes_only_its_own_directories() {
    let temp = TempDirectory::new("worktree-registration-failure");
    let parent = empty_repository(&temp.path().join("repository"), None);
    let (previous, _) = create_worktree(&parent, "existing", None).unwrap();
    let root = temp.path().join("blocked");
    fs::write(&root, "existing file").unwrap();
    let registrations = parent.git_directory().join("worktrees");
    let existing = previous.git_directory().to_path_buf();
    fs::write(existing.join("keep"), "existing registration").unwrap();

    // Repeating the failure must not accumulate partially registered worktrees.
    for _ in 0..2 {
        let error = create_worktree(&parent, "new-branch", Some(&root)).unwrap_err();
        assert!(error.to_string().contains("failed to register worktree"));
        assert_eq!(fs::read_to_string(&root).unwrap(), "existing file");
        let entries = fs::read_dir(&registrations)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(entries, vec![existing.clone()]);
        assert!(previous.root().exists());
        assert_eq!(
            fs::read_to_string(existing.join("keep")).unwrap(),
            "existing registration"
        );
    }
}

#[test]
fn failed_worktree_checkout_removes_its_registration_and_destination() {
    let temp = TempDirectory::new("worktree-checkout-failure");
    // A readable commit whose tree object is missing fails after registration succeeded.
    let missing_tree =
        gix::ObjectId::from_hex(b"1111111111111111111111111111111111111111").unwrap();
    let parent = empty_repository(&temp.path().join("repository"), Some(missing_tree));
    let root = temp.path().join("worktrees");
    assert!(create_worktree(&parent, "new-branch", Some(&root)).is_err());
    assert!(!worktree_destination(parent.root(), "new-branch", Some(&root)).exists());
    assert_eq!(
        fs::read_dir(parent.git_directory().join("worktrees"))
            .unwrap()
            .count(),
        0
    );
    assert!(
        gix::open(parent.root())
            .unwrap()
            .try_find_reference("refs/heads/new-branch")
            .unwrap()
            .is_some()
    );
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
    let initial = on_main.fingerprint(None).unwrap();
    assert_eq!(on_main.fingerprint(None).unwrap(), initial);
    std::thread::sleep(std::time::Duration::from_millis(20));
    // Committing on the same branch does not rewrite HEAD, but it moves the head.
    fs::write(repository.join("README.md"), "again\n").unwrap();
    git(&repository, ["commit", "-am", "second"]);
    let before = on_main.fingerprint(None).unwrap();
    assert_ne!(before, initial);

    std::thread::sleep(std::time::Duration::from_millis(20));
    git(&repository, ["checkout", "-b", "feature/nested"]);
    assert_eq!(
        discover_repository(&repository).unwrap().unwrap().branch(),
        Some("feature/nested")
    );
    git(&repository, ["checkout", "--detach"]);
    let detached = discover_repository(&repository).unwrap().unwrap();
    assert_eq!(detached.branch(), None);
    assert_ne!(detached.fingerprint(None).unwrap(), before);
    assert!(
        discover_repository(repository.join(".git"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn discovery_counts_divergence_from_the_local_upstream_ref() {
    let temp = TempDirectory::new("git-upstream");
    let remote = temp.path().join("remote.git");
    let repository = temp.path().join("repository");
    fs::create_dir_all(&remote).unwrap();
    fs::create_dir_all(&repository).unwrap();
    git(&remote, ["init", "--bare", "-b", "main"]);
    git(&repository, ["init", "-b", "main"]);
    git(&repository, ["config", "user.name", "Condr Tests"]);
    git(
        &repository,
        ["config", "user.email", "condr@example.invalid"],
    );
    fs::write(repository.join("README.md"), "condr\n").unwrap();
    git(&repository, ["add", "README.md"]);
    git(&repository, ["commit", "-m", "initial"]);
    // No upstream yet: the branch exists but has nothing to diverge from.
    assert_eq!(
        discover_repository(&repository)
            .unwrap()
            .unwrap()
            .upstream(),
        None
    );

    let remote_url = remote.to_string_lossy().replace('\\', "/");
    git(&repository, ["remote", "add", "origin", &remote_url]);
    git(&repository, ["push", "-u", "origin", "main"]);
    let in_sync = discover_repository(&repository).unwrap().unwrap();
    assert_eq!(
        in_sync.upstream(),
        Some(GitUpstream {
            ahead: 0,
            behind: 0
        })
    );
    let synced = in_sync.fingerprint(None).unwrap();

    // Two local commits, one of them also pushed elsewhere and fetched back as remote work.
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::write(repository.join("README.md"), "ahead\n").unwrap();
    git(&repository, ["commit", "-am", "ahead"]);
    assert_eq!(
        discover_repository(&repository)
            .unwrap()
            .unwrap()
            .upstream(),
        Some(GitUpstream {
            ahead: 1,
            behind: 0
        })
    );

    let other = temp.path().join("other");
    git(temp.path(), ["clone", "-q", &remote_url, "other"]);
    git(&other, ["config", "user.name", "Condr Tests"]);
    git(&other, ["config", "user.email", "condr@example.invalid"]);
    fs::write(other.join("OTHER.md"), "behind\n").unwrap();
    git(&other, ["add", "OTHER.md"]);
    git(&other, ["commit", "-m", "behind"]);
    git(&other, ["push", "-q", "origin", "main"]);
    // Fetching rewrites the remote-tracking ref, so the fingerprint moves without a checkout.
    std::thread::sleep(std::time::Duration::from_millis(20));
    git(&repository, ["fetch", "-q", "origin"]);
    assert_ne!(in_sync.fingerprint(None).unwrap(), synced);
    assert_eq!(
        discover_repository(&repository)
            .unwrap()
            .unwrap()
            .upstream(),
        Some(GitUpstream {
            ahead: 1,
            behind: 1
        })
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

fn git_stdout<const N: usize>(cwd: &Path, args: [&str; N]) -> String {
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
    String::from_utf8_lossy(&output.stdout).into_owned()
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

#[test]
fn worktree_destination_follows_the_configured_root_kind() {
    let repo = Path::new("/srv/code/condr");
    assert_eq!(
        worktree_destination(repo, "feature/x", None),
        PathBuf::from("/srv/code/condr.worktrees/feature-x")
    );
    let shared = if cfg!(windows) {
        "C:/data/worktrees"
    } else {
        "/data/worktrees"
    };
    assert_eq!(
        worktree_destination(repo, "feature/x", Some(Path::new(shared))),
        Path::new(shared).join("condr").join("feature-x")
    );
    assert_eq!(
        worktree_destination(repo, "feature/x", Some(Path::new(".."))),
        PathBuf::from("/srv/code/condr/../feature-x")
    );
    assert_eq!(
        worktree_destination(repo, "feature/x", Some(Path::new(".worktrees"))),
        PathBuf::from("/srv/code/condr/.worktrees/feature-x")
    );
    let home = dirs::home_dir().unwrap();
    assert_eq!(
        worktree_destination(repo, "feature/x", Some(Path::new("~/worktrees"))),
        home.join("worktrees").join("condr").join("feature-x")
    );
    // `~user` is a literal directory name, not a home shorthand.
    assert_eq!(
        worktree_destination(repo, "feature/x", Some(Path::new("~other"))),
        PathBuf::from("/srv/code/condr/~other/feature-x")
    );
}
