//! The working-tree change list and per-file diffs, against `HEAD` (ADR 0017) and against a
//! base (ADR 0034), in repositories the `git` CLI builds, since that is what every user's
//! repository was built with.

use std::fs;
use std::path::{Path, PathBuf};

use relative_path::RelativePath;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use condr_core::{
    DiffLineKind, FileDiffContent, GitChangeStatus, GitDiffStat, discover_repository,
};

static NEXT_REPO: AtomicU64 = AtomicU64::new(0);

fn scratch_repository(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "condr-git-changes-{name}-{}-{}",
        std::process::id(),
        NEXT_REPO.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    git(&root, ["init", "-q", "-b", "main"]);
    git(&root, ["config", "user.email", "condr@example.com"]);
    git(&root, ["config", "user.name", "Condr"]);
    git(&root, ["config", "commit.gpgsign", "false"]);
    root
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
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn commit_file(root: &Path, path: &str, content: &str) {
    let file = root.join(path);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, content).unwrap();
    git(root, ["add", path]);
    git(root, ["commit", "-q", "-m", &format!("add {path}")]);
}

#[test]
fn changes_fold_index_and_worktree_into_one_status_per_path() {
    let root = scratch_repository("status");
    commit_file(&root, "kept.txt", "same\n");
    commit_file(&root, "edited.txt", "one\ntwo\nthree\n");
    commit_file(&root, "gone.txt", "bye\n");
    commit_file(&root, "moved/old.txt", "moving\ncontent\n");
    commit_file(&root, "staged-then-edited.txt", "a\n");
    fs::write(root.join("edited.txt"), "one\n2\nthree\nfour\n").unwrap();
    fs::remove_file(root.join("gone.txt")).unwrap();
    git(&root, ["mv", "moved/old.txt", "moved/new.txt"]);
    fs::write(root.join("brand-new.txt"), "fresh\nfile\n").unwrap();
    fs::write(root.join("staged.txt"), "staged\n").unwrap();
    git(&root, ["add", "staged.txt"]);
    fs::write(root.join("staged-then-edited.txt"), "a\nb\n").unwrap();
    git(&root, ["add", "staged-then-edited.txt"]);
    fs::write(root.join("staged-then-edited.txt"), "a\nb\nc\n").unwrap();
    fs::write(root.join("ignored.log"), "noise\n").unwrap();
    fs::write(root.join(".gitignore"), "*.log\n").unwrap();

    let repository = discover_repository(&root).unwrap().unwrap();
    let changes = repository.changes(None).unwrap().0;

    assert!(!changes.truncated);
    let summary: Vec<(String, Option<String>, GitChangeStatus)> = changes
        .entries
        .iter()
        .map(|entry| {
            (
                entry.path.to_string(),
                entry.old_path.as_ref().map(|old| old.to_string()),
                entry.status,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (".gitignore".to_string(), None, GitChangeStatus::Untracked),
            (
                "brand-new.txt".to_string(),
                None,
                GitChangeStatus::Untracked
            ),
            ("edited.txt".to_string(), None, GitChangeStatus::Modified),
            ("gone.txt".to_string(), None, GitChangeStatus::Deleted),
            (
                "moved/new.txt".to_string(),
                Some("moved/old.txt".to_string()),
                GitChangeStatus::Renamed
            ),
            (
                "staged-then-edited.txt".to_string(),
                None,
                GitChangeStatus::Modified
            ),
            ("staged.txt".to_string(), None, GitChangeStatus::Added),
        ],
        "sorted by path, the ignored file absent, and the kept file absent"
    );
    let stat = |path: &str| {
        changes
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .unwrap()
            .stat
    };
    assert_eq!(
        stat("edited.txt"),
        Some(GitDiffStat {
            added: 2,
            deleted: 1
        })
    );
    assert_eq!(
        stat("gone.txt"),
        Some(GitDiffStat {
            added: 0,
            deleted: 1
        })
    );
    assert_eq!(
        stat("brand-new.txt"),
        Some(GitDiffStat {
            added: 2,
            deleted: 0
        }),
        "an untracked file counts as wholly added"
    );
    assert_eq!(
        stat("staged.txt"),
        Some(GitDiffStat {
            added: 1,
            deleted: 0
        })
    );
    assert_eq!(
        stat("staged-then-edited.txt"),
        Some(GitDiffStat {
            added: 2,
            deleted: 0
        }),
        "the working tree is compared to HEAD, not to the index"
    );
    assert_eq!(
        stat("moved/new.txt"),
        Some(GitDiffStat {
            added: 0,
            deleted: 0
        }),
        "a pure rename changes no lines"
    );

    assert!(
        repository.ignores_all([Path::new("ignored.log"), Path::new("build.log")]),
        "paths the ignore rules cover cannot change the status"
    );
    assert!(!repository.ignores_all([Path::new("ignored.log"), Path::new("edited.txt")]));
    assert!(!repository.ignores_all(Vec::<PathBuf>::new()));
}

#[test]
fn file_diff_numbers_every_line_and_flags_binary_and_oversized_files() {
    let root = scratch_repository("diff");
    commit_file(&root, "notes.txt", "alpha\nbeta\ngamma\ndelta\n");
    commit_file(&root, "image.bin", "\0\u{1}\u{2}binary\0");
    fs::write(
        root.join("notes.txt"),
        "alpha\nBETA\ngamma\ndelta\nepsilon\n",
    )
    .unwrap();
    fs::write(root.join("image.bin"), b"\0\x01\x02changed\0").unwrap();
    fs::write(root.join("huge.txt"), "x\n".repeat(600_000)).unwrap();
    fs::write(root.join("fresh.txt"), "new\n").unwrap();

    let repository = discover_repository(&root).unwrap().unwrap();

    let notes = repository
        .file_diff(RelativePath::new("notes.txt"), None, None)
        .unwrap();
    let FileDiffContent::Text { hunks } = &notes.content else {
        panic!("a text file diffs as text: {notes:?}");
    };
    assert_eq!(hunks.len(), 1);
    let hunk = &hunks[0];
    assert_eq!(
        (
            hunk.old_start,
            hunk.old_lines,
            hunk.new_start,
            hunk.new_lines
        ),
        (1, 4, 1, 5)
    );
    let lines: Vec<(DiffLineKind, Option<u32>, Option<u32>, &str)> = hunk
        .lines
        .iter()
        .map(|line| {
            (
                line.kind,
                line.old_number,
                line.new_number,
                line.text.as_str(),
            )
        })
        .collect();
    assert_eq!(
        lines,
        [
            (DiffLineKind::Context, Some(1), Some(1), "alpha"),
            (DiffLineKind::Removed, Some(2), None, "beta"),
            (DiffLineKind::Added, None, Some(2), "BETA"),
            (DiffLineKind::Context, Some(3), Some(3), "gamma"),
            (DiffLineKind::Context, Some(4), Some(4), "delta"),
            (DiffLineKind::Added, None, Some(5), "epsilon"),
        ]
    );

    let fresh = repository
        .file_diff(RelativePath::new("fresh.txt"), None, None)
        .unwrap();
    let FileDiffContent::Text { hunks } = &fresh.content else {
        panic!("an untracked file diffs against nothing: {fresh:?}");
    };
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].lines[0].kind, DiffLineKind::Added);
    assert_eq!(hunks[0].lines[0].new_number, Some(1));

    assert_eq!(
        repository
            .file_diff(RelativePath::new("image.bin"), None, None)
            .unwrap()
            .content,
        FileDiffContent::Binary
    );
    assert!(matches!(
        repository
            .file_diff(RelativePath::new("huge.txt"), None, None)
            .unwrap()
            .content,
        FileDiffContent::TooLarge { .. }
    ));
    assert!(
        repository
            .file_diff(RelativePath::new("missing.txt"), None, None)
            .is_err(),
        "a path in neither HEAD nor the work tree has no diff"
    );
    assert!(
        repository
            .file_diff(RelativePath::new("../outside.txt"), None, None)
            .is_err()
    );
    assert!(
        repository
            .file_diff(
                RelativePath::new("notes.txt"),
                Some(RelativePath::new("../outside.txt")),
                None
            )
            .is_err()
    );
}

#[test]
fn file_diff_of_a_rename_compares_against_the_old_path() {
    let root = scratch_repository("rename");
    commit_file(&root, "old.txt", "one\ntwo\n");
    git(&root, ["mv", "old.txt", "new.txt"]);
    fs::write(root.join("new.txt"), "one\n2\n").unwrap();

    let repository = discover_repository(&root).unwrap().unwrap();
    let entry = repository.changes(None).unwrap().0.entries.remove(0);
    assert_eq!(entry.status, GitChangeStatus::Renamed);
    assert_eq!(
        entry.old_path.as_deref(),
        Some(RelativePath::new("old.txt"))
    );

    let diff = repository
        .file_diff(&entry.path, entry.old_path.as_deref(), None)
        .unwrap();
    let FileDiffContent::Text { hunks } = &diff.content else {
        panic!("a renamed text file diffs as text: {diff:?}");
    };
    let kinds: Vec<DiffLineKind> = hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .map(|line| line.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            DiffLineKind::Context,
            DiffLineKind::Removed,
            DiffLineKind::Added
        ],
        "only the edited line differs, not the whole file"
    );

    let FileDiffContent::Text { hunks } = repository
        .file_diff(&entry.path, None, None)
        .unwrap()
        .content
    else {
        panic!("without the old path the file is new");
    };
    assert!(
        hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .all(|line| line.kind == DiffLineKind::Added)
    );
}

#[test]
fn mark_ignored_flags_the_listing_entries_the_ignore_rules_exclude() {
    let root = scratch_repository("ignored");
    commit_file(&root, ".gitignore", "target/\n*.log\n");
    fs::create_dir_all(root.join("target/debug")).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(root.join("src/build.log"), "noise\n").unwrap();
    let repository = discover_repository(&root).unwrap().unwrap();

    let mut listing = condr_core::list_directory(&root, RelativePath::new("")).unwrap();
    repository.mark_ignored(RelativePath::new(""), &mut listing);
    let flags: Vec<(&str, bool)> = listing
        .entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.ignored))
        .collect();
    assert_eq!(
        flags,
        [("src", false), ("target", true), (".gitignore", false)]
    );

    let mut src = condr_core::list_directory(&root, RelativePath::new("src")).unwrap();
    repository.mark_ignored(RelativePath::new("src"), &mut src);
    assert_eq!(
        src.entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.ignored))
            .collect::<Vec<_>>(),
        [("build.log", true), ("main.rs", false)]
    );

    let _ = fs::remove_dir_all(&root);
}

/// Path and status of every entry, the way the assertions below read a list.
fn summary(changes: &condr_core::GitChanges) -> Vec<(&str, GitChangeStatus)> {
    changes
        .entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry.status))
        .collect()
}

#[test]
fn the_base_list_keeps_what_the_branch_committed_and_nothing_the_base_gained() {
    let root = scratch_repository("base-list");
    commit_file(&root, "edited.txt", "one\n");
    commit_file(&root, "old.txt", "x\ny\n");
    git(&root, ["switch", "-q", "-c", "feature"]);
    commit_file(&root, "committed.txt", "agent work\n");
    git(&root, ["mv", "old.txt", "new.txt"]);
    fs::write(root.join("new.txt"), "x\nY\n").unwrap();
    git(&root, ["commit", "-q", "-am", "rename"]);
    // The base branch moves on; its commit is not the branch's change.
    git(&root, ["switch", "-q", "main"]);
    commit_file(&root, "main-only.txt", "elsewhere\n");
    git(&root, ["switch", "-q", "feature"]);
    fs::write(root.join("edited.txt"), "one\ntwo\n").unwrap();
    fs::write(root.join("fresh.txt"), "new\n").unwrap();

    let repository = discover_repository(&root).unwrap().unwrap();
    let base = repository.base(Some("main")).unwrap().unwrap();
    assert_eq!(base.name(), "main");
    let (head, against_base) = repository.changes(Some(&base)).unwrap();
    assert_eq!(
        summary(&head),
        [
            ("edited.txt", GitChangeStatus::Modified),
            ("fresh.txt", GitChangeStatus::Untracked),
        ],
        "a commit drops its files off the HEAD list"
    );
    let against_base = against_base.unwrap();
    assert_eq!(
        summary(&against_base),
        [
            ("committed.txt", GitChangeStatus::Added),
            ("edited.txt", GitChangeStatus::Modified),
            ("fresh.txt", GitChangeStatus::Untracked),
            ("new.txt", GitChangeStatus::Renamed),
        ]
    );
    let renamed = &against_base.entries[3];
    assert_eq!(
        renamed.old_path.as_deref(),
        Some(RelativePath::new("old.txt"))
    );
    assert_eq!(
        renamed.stat,
        Some(GitDiffStat {
            added: 1,
            deleted: 1
        })
    );

    // A rename committed on the branch diffs against where the base has the file.
    let FileDiffContent::Text { hunks } = repository
        .file_diff(&renamed.path, renamed.old_path.as_deref(), Some(&base))
        .unwrap()
        .content
    else {
        panic!("a renamed text file diffs as text");
    };
    let lines: Vec<(DiffLineKind, &str)> = hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .map(|line| (line.kind, line.text.as_str()))
        .collect();
    assert_eq!(
        lines,
        [
            (DiffLineKind::Context, "x"),
            (DiffLineKind::Removed, "y"),
            (DiffLineKind::Added, "Y"),
        ]
    );
    // A committed file is new against the base and unchanged against HEAD.
    let path = RelativePath::new("committed.txt");
    assert!(matches!(
        repository.file_diff(path, None, Some(&base)).unwrap().content,
        FileDiffContent::Text { hunks } if hunks.len() == 1
    ));
    assert_eq!(
        repository.file_diff(path, None, None).unwrap().content,
        FileDiffContent::Text { hunks: Vec::new() }
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_base_follows_a_rebase_and_empties_once_the_branch_is_merged() {
    let root = scratch_repository("base-rebase");
    commit_file(&root, "README.md", "condr\n");
    git(&root, ["switch", "-q", "-c", "feature"]);
    commit_file(&root, "committed.txt", "agent work\n");
    git(&root, ["switch", "-q", "main"]);
    commit_file(&root, "main-only.txt", "elsewhere\n");
    git(&root, ["switch", "-q", "feature"]);
    git(&root, ["rebase", "-q", "main"]);

    let repository = discover_repository(&root).unwrap().unwrap();
    let base = repository.base(Some("main")).unwrap().unwrap();
    let (_, against_base) = repository.changes(Some(&base)).unwrap();
    assert_eq!(
        summary(&against_base.unwrap()),
        [("committed.txt", GitChangeStatus::Added)],
        "the recorded name is followed to the new fork point"
    );

    // Merged: main now holds the branch, so only what is uncommitted is left.
    git(&root, ["branch", "-f", "main", "feature"]);
    fs::write(root.join("fresh.txt"), "new\n").unwrap();
    let base = repository.base(Some("main")).unwrap().unwrap();
    let (_, against_base) = repository.changes(Some(&base)).unwrap();
    assert_eq!(
        summary(&against_base.unwrap()),
        [("fresh.txt", GitChangeStatus::Untracked)]
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_base_is_the_recorded_branch_else_the_default_branch() {
    let origin = scratch_repository("base-origin");
    commit_file(&origin, "README.md", "condr\n");
    let clone = origin.with_extension("clone");
    let _ = fs::remove_dir_all(&clone);
    let url = origin.to_string_lossy().replace('\\', "/");
    git(
        origin.parent().unwrap(),
        ["clone", "-q", &url, clone.to_str().unwrap()],
    );
    git(&clone, ["config", "user.email", "condr@example.com"]);
    git(&clone, ["config", "user.name", "Condr"]);
    git(&clone, ["config", "commit.gpgsign", "false"]);

    // On the default branch itself there is nothing to compare against: the remote's
    // default is its upstream, and a recorded `main` is the branch.
    let on_main = discover_repository(&clone).unwrap().unwrap();
    assert_eq!(on_main.base(None).unwrap(), None);
    assert_eq!(on_main.base(Some("main")).unwrap(), None);

    git(&clone, ["branch", "trunk"]);
    git(&clone, ["switch", "-q", "-c", "feature"]);
    commit_file(&clone, "feature.txt", "work\n");
    let feature = discover_repository(&clone).unwrap().unwrap();
    let name = |recorded: Option<&str>| {
        feature
            .base(recorded)
            .unwrap()
            .map(|base| base.name().to_owned())
    };
    assert_eq!(name(Some("trunk")).as_deref(), Some("trunk"));
    assert_eq!(name(None).as_deref(), Some("origin/main"));
    assert_eq!(
        name(Some("deleted")).as_deref(),
        Some("origin/main"),
        "a recorded branch that is gone falls back"
    );
    let _ = fs::remove_dir_all(&clone);
    let _ = fs::remove_dir_all(&origin);

    // Without a remote: a local `main`, else `master`, else nothing.
    for (default, expected) in [
        ("main", Some("main")),
        ("master", Some("master")),
        ("trunk", None),
    ] {
        let root = scratch_repository("base-local");
        commit_file(&root, "README.md", "condr\n");
        git(&root, ["branch", "-M", default]);
        git(&root, ["switch", "-q", "-c", "feature"]);
        let repository = discover_repository(&root).unwrap().unwrap();
        assert_eq!(
            repository
                .base(None)
                .unwrap()
                .as_ref()
                .map(|base| base.name()),
            expected,
            "default branch {default}"
        );
        // Histories that share no commit have no fork point.
        git(&root, ["switch", "-q", "--orphan", "unrelated"]);
        commit_file(&root, "other.txt", "other\n");
        let unrelated = discover_repository(&root).unwrap().unwrap();
        assert_eq!(unrelated.base(Some("feature")).unwrap(), None);
        let _ = fs::remove_dir_all(&root);
    }

    let unborn = scratch_repository("base-unborn");
    let repository = discover_repository(&unborn).unwrap().unwrap();
    assert_eq!(repository.base(Some("main")).unwrap(), None);
    let _ = fs::remove_dir_all(&unborn);
}

#[test]
fn the_fingerprint_moves_with_the_base_ref() {
    let root = scratch_repository("base-fingerprint");
    commit_file(&root, "README.md", "condr\n");
    git(&root, ["switch", "-q", "-c", "feature"]);
    commit_file(&root, "feature.txt", "work\n");
    let repository = discover_repository(&root).unwrap().unwrap();
    let base = repository.base(None).unwrap().unwrap();
    let without = repository.fingerprint(None).unwrap();
    let with = repository.fingerprint(Some(&base)).unwrap();

    // Another checkout moving `main` touches nothing of this branch but the base ref.
    std::thread::sleep(std::time::Duration::from_millis(20));
    git(&root, ["update-ref", "refs/heads/main", "feature"]);
    assert_eq!(repository.fingerprint(None).unwrap(), without);
    assert_ne!(repository.fingerprint(Some(&base)).unwrap(), with);

    let _ = fs::remove_dir_all(&root);
}
