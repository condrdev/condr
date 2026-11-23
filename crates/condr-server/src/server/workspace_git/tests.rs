use super::*;

#[test]
fn terminal_wakeups_coalesce_per_workspace_while_a_scan_is_busy() {
    let (messages, inbox) = mpsc::channel();
    let watcher = GitWatcher {
        messages,
        terminal_activity: Arc::new(Mutex::new(HashMap::new())),
        stopping: Arc::new(AtomicBool::new(false)),
        worker: None,
    };
    let first = WorkspaceId::from_u64(1);
    let second = WorkspaceId::from_u64(2);
    let now = Instant::now();
    for offset in 0..1000 {
        watcher.terminal_activity(first, now + Duration::from_millis(offset));
    }
    // Another Pane can arrive with an older timestamp after the latest activity.
    watcher.terminal_activity(first, now);
    watcher.terminal_activity(second, now);
    assert!(matches!(inbox.try_recv(), Ok(WatchMessage::TerminalActivity(id)) if id == first));
    assert!(matches!(inbox.try_recv(), Ok(WatchMessage::TerminalActivity(id)) if id == second));
    assert!(matches!(inbox.try_recv(), Err(mpsc::TryRecvError::Empty)));
    assert_eq!(
        watcher.terminal_activity.lock().unwrap()[&first],
        now + Duration::from_millis(999)
    );
}

#[test]
fn continuous_activity_preserves_each_workspaces_cooldown_and_filesystem_deadline() {
    let now = Instant::now();
    let mut busy = Watched::new(PathBuf::from("busy"));
    let mut other = Watched::new(PathBuf::from("other"));
    busy.record_scan(now, HashMap::new());
    busy.terminal_activity(now);
    assert_eq!(
        busy.next_due(),
        None,
        "a scan covers activity that preceded it"
    );
    busy.terminal_activity(now + Duration::from_millis(1));
    for offset in 2..1000 {
        busy.terminal_activity(now + Duration::from_millis(offset));
    }
    assert_eq!(busy.next_due(), Some(now + GIT_SCAN_INTERVAL));
    other.terminal_activity(now + Duration::from_millis(10));
    assert_eq!(other.next_due(), Some(now + Duration::from_millis(10)));

    // Filesystem debouncing neither moves the terminal deadline nor inherits its throttle.
    busy.filesystem_due = Some(now + GIT_WATCH_DEBOUNCE);
    assert_eq!(busy.next_due(), Some(now + GIT_WATCH_DEBOUNCE));
    busy.filesystem_due = Some(now + GIT_SCAN_INTERVAL + GIT_WATCH_DEBOUNCE);
    assert_eq!(busy.next_due(), Some(now + GIT_SCAN_INTERVAL));
}

#[test]
fn filesystem_refresh_bypasses_unchanged_fingerprint_and_covers_pending_activity() {
    let fixture = Repository::new();
    let mut state = RuntimeState::new(&fixture.root.join("server.sock"));
    let workspace_id = state
        .session
        .create_workspace(fixture.root.clone())
        .unwrap();
    set_workspace_git(
        &mut state,
        workspace_id,
        workspace_repositories(&fixture.root).unwrap(),
    );
    let state = Arc::new(Mutex::new(state));
    let weak = Arc::downgrade(&state);
    let mut watched = Watched::new(fixture.root.clone());
    watched.terminal_activity(Instant::now());
    assert!(refresh_workspace(workspace_id, &mut watched, None, &weak));
    assert!(watched.fingerprints[relative_path::RelativePath::new("")].is_some());

    std::fs::write(fixture.root.join("changed.txt"), "new content\n").unwrap();
    watched.scanned_at = Some(Instant::now() - GIT_SCAN_INTERVAL);
    watched.terminal_activity(Instant::now());
    assert!(refresh_workspace(workspace_id, &mut watched, None, &weak));
    assert!(
        state.lock().unwrap().workspace_git[&workspace_id][0]
            .changes
            .entries
            .is_empty(),
        "output with unchanged refs skips the status walk"
    );

    let before = state.lock().unwrap().events.len();
    watched.terminal_activity(Instant::now());
    assert!(watched.terminal_due.is_some());
    watched.pending.push(PathBuf::from("changed.txt"));
    watched.filesystem_due = Some(Instant::now());
    assert!(refresh_workspace(workspace_id, &mut watched, None, &weak));
    assert_eq!(
        watched.next_due(),
        None,
        "the filesystem scan covers waiting terminal activity"
    );
    let locked = state.lock().unwrap();
    assert_eq!(
        locked.workspace_git[&workspace_id][0].changes.entries[0]
            .path
            .as_str(),
        "changed.txt"
    );
    let events: Vec<_> = locked.events.iter().skip(before).collect();
    assert_eq!(
        events.len(),
        2,
        "merged work publishes once, in commit order"
    );
    assert!(matches!(
        events[0].event,
        SessionEvent::WorkspaceGitChanged { .. }
    ));
    assert!(matches!(
        events[1].event,
        SessionEvent::WorkspaceFilesChanged { .. }
    ));
    assert!(events[0].sequence < events[1].sequence);
}

#[test]
fn a_git_scan_that_panics_loses_only_that_scan() {
    let fixture = Repository::new();
    let mut state = RuntimeState::new(&fixture.root.join("server.sock"));
    let workspace_id = state
        .session
        .create_workspace(fixture.root.clone())
        .unwrap();
    set_workspace_git(
        &mut state,
        workspace_id,
        workspace_repositories(&fixture.root).unwrap(),
    );
    state
        .injected_panics
        .insert(("git-scan", workspace_id.as_u64()));
    let state = Arc::new(Mutex::new(state));
    let weak = Arc::downgrade(&state);
    let mut watched = Watched::new(fixture.root.clone());
    std::fs::write(fixture.root.join("changed.txt"), "new content\n").unwrap();
    let change = |watched: &mut Watched| {
        watched.pending.push(PathBuf::from("changed.txt"));
        watched.filesystem_due = Some(Instant::now());
    };

    change(&mut watched);
    assert!(refresh_isolated(workspace_id, &mut watched, None, &weak));
    assert_eq!(
        watched.next_due(),
        None,
        "the scan that panicked is not retried"
    );
    assert!(
        state.lock().unwrap().workspace_git[&workspace_id][0]
            .changes
            .entries
            .is_empty()
    );

    change(&mut watched);
    assert!(refresh_isolated(workspace_id, &mut watched, None, &weak));
    assert_eq!(
        state.lock().unwrap().workspace_git[&workspace_id][0]
            .changes
            .entries[0]
            .path
            .as_str(),
        "changed.txt"
    );
}

#[test]
fn idle_watcher_shutdown_joins_even_while_callback_sender_is_alive() {
    let watcher = GitWatcher::start(Weak::new());
    let callback_sender = watcher.messages.clone();
    let (done, stopped) = mpsc::channel();
    thread::spawn(move || {
        watcher.shutdown();
        done.send(()).unwrap();
    });
    stopped
        .recv_timeout(Duration::from_secs(3))
        .expect("idle watcher must stop without another file event");
    assert!(
        callback_sender
            .send(WatchMessage::Refresh(WorkspaceId::from_u64(1)))
            .is_err()
    );
}

#[test]
fn dropping_watcher_requests_exit_without_waiting_for_state_lock() {
    // An empty Root of its own, removed after the worker is gone. The system's temp
    // directory would make the scan look for a repository in each of its directories
    // (ADR 0073), and a busy machine's took longer than the worker is given to stop.
    let fixture = Repository {
        root: std::env::temp_dir().join(format!(
            "condr-git-drop-{}-{}",
            std::process::id(),
            crate::test_support::unique_suffix()
        )),
    };
    std::fs::create_dir_all(&fixture.root).unwrap();
    let state = Arc::new(Mutex::new(RuntimeState::new(Path::new(
        "test-watcher.sock",
    ))));
    let mut watcher = GitWatcher::without_filesystem(Arc::downgrade(&state));
    let worker = watcher.worker.take().unwrap();
    let callback_sender = watcher.messages.clone();
    let mut locked = state.lock().unwrap();
    let workspace_id = locked
        .session
        .create_workspace(fixture.root.clone())
        .unwrap();
    watcher.watch(workspace_id, fixture.root.clone(), Vec::new());
    watcher.refresh(workspace_id);
    // RuntimeState can be dropped by the worker's last Weak upgrade, so Drop only signals.
    let (dropped, drop_finished) = mpsc::channel();
    thread::spawn(move || {
        drop(watcher);
        dropped.send(()).unwrap();
    });
    drop_finished
        .recv_timeout(Duration::from_secs(3))
        .expect("Drop must not join while its caller holds the state lock");
    drop(locked);
    let (done, stopped) = mpsc::channel();
    thread::spawn(move || {
        worker.join().unwrap();
        done.send(()).unwrap();
    });
    stopped
        .recv_timeout(Duration::from_secs(3))
        .expect("Drop must wake the worker");
    assert!(
        callback_sender
            .send(WatchMessage::Refresh(workspace_id))
            .is_err()
    );
}

/// ADR 0073: a change walks the status of the repository it lies in and no other, a
/// repository that appears under the Root joins, and a path in none changes Files only.
#[test]
fn a_scan_walks_only_the_repositories_a_change_touched() {
    // A directory that is no repository, removed like one.
    let fixture = Repository {
        root: std::env::temp_dir().join(format!(
            "condr-git-several-{}-{}",
            std::process::id(),
            crate::test_support::unique_suffix()
        )),
    };
    let root = fixture.root.join("work");
    unborn_repository(&root.join("api"));
    unborn_repository(&root.join("web"));
    let mut state = RuntimeState::new(&fixture.root.join("server.sock"));
    let workspace_id = state.session.create_workspace(root.clone()).unwrap();
    set_workspace_git(
        &mut state,
        workspace_id,
        workspace_repositories(&root).unwrap(),
    );
    let state = Arc::new(Mutex::new(state));
    let weak = Arc::downgrade(&state);
    let mut watched = Watched::new(root.clone());
    let scan = |watched: &mut Watched, pending: &[&str]| {
        watched.pending.extend(pending.iter().map(PathBuf::from));
        watched.filesystem_due = Some(Instant::now());
        assert!(refresh_workspace(workspace_id, watched, None, &weak));
    };
    let changes = || -> Vec<(String, Vec<String>)> {
        state.lock().unwrap().workspace_git[&workspace_id]
            .iter()
            .map(|git| {
                (
                    git.repository.directory().to_string(),
                    git.changes
                        .entries
                        .iter()
                        .map(|entry| entry.path.to_string())
                        .collect(),
                )
            })
            .collect()
    };
    let files_changed = || {
        state
            .lock()
            .unwrap()
            .events
            .iter()
            .filter(|event| matches!(event.event, SessionEvent::WorkspaceFilesChanged { .. }))
            .count()
    };

    watched.forced = true;
    scan(&mut watched, &[]);
    std::fs::write(root.join("api/a.txt"), "a\n").unwrap();
    std::fs::write(root.join("web/b.txt"), "b\n").unwrap();
    scan(&mut watched, &["api/a.txt"]);
    assert_eq!(
        changes(),
        [
            ("api".to_owned(), vec!["api/a.txt".to_owned()]),
            ("web".to_owned(), Vec::new()),
        ],
        "web's file was not among the changed paths"
    );

    let before = files_changed();
    std::fs::write(root.join("notes.txt"), "notes\n").unwrap();
    scan(&mut watched, &["notes.txt"]);
    assert_eq!(files_changed(), before + 1);
    assert_eq!(changes()[1].1, Vec::<String>::new());

    unborn_repository(&root.join("cloned"));
    std::fs::write(root.join("cloned/c.txt"), "c\n").unwrap();
    scan(&mut watched, &["cloned/.git"]);
    assert_eq!(
        changes(),
        [
            ("api".to_owned(), vec!["api/a.txt".to_owned()]),
            ("cloned".to_owned(), vec!["cloned/c.txt".to_owned()]),
            ("web".to_owned(), Vec::new()),
        ]
    );

    std::fs::remove_dir_all(root.join("api")).unwrap();
    std::fs::remove_dir_all(root.join("cloned")).unwrap();
    std::fs::remove_dir_all(root.join("web")).unwrap();
    scan(&mut watched, &["api", "cloned", "web"]);
    assert!(
        !state
            .lock()
            .unwrap()
            .workspace_git
            .contains_key(&workspace_id),
        "no repository left is no Git"
    );
}

#[test]
fn a_change_in_a_repository_under_the_roots_own_walks_that_one_alone() {
    let fixture = Repository::new();
    let root = fixture.root.join("work");
    unborn_repository(&root);
    unborn_repository(&root.join("api"));
    let mut state = RuntimeState::new(&fixture.root.join("server.sock"));
    let workspace_id = state.session.create_workspace(root.clone()).unwrap();
    set_workspace_git(
        &mut state,
        workspace_id,
        workspace_repositories(&root).unwrap(),
    );
    let state = Arc::new(Mutex::new(state));
    let weak = Arc::downgrade(&state);
    let mut watched = Watched::new(root.clone());
    let scan = |watched: &mut Watched, pending: &[&str]| {
        watched.pending.extend(pending.iter().map(PathBuf::from));
        watched.filesystem_due = Some(Instant::now());
        assert!(refresh_workspace(workspace_id, watched, None, &weak));
    };
    let changes = || -> Vec<(String, Vec<String>)> {
        state.lock().unwrap().workspace_git[&workspace_id]
            .iter()
            .map(|git| {
                let paths = git.changes.entries.iter().map(|e| e.path.to_string());
                (git.repository.directory().to_string(), paths.collect())
            })
            .collect()
    };

    watched.forced = true;
    scan(&mut watched, &[]);
    std::fs::write(root.join("own.txt"), "own\n").unwrap();
    std::fs::write(root.join("api/a.txt"), "a\n").unwrap();
    scan(&mut watched, &["api/a.txt"]);
    assert_eq!(
        changes(),
        [
            (String::new(), Vec::new()),
            ("api".to_owned(), vec!["api/a.txt".to_owned()]),
        ],
        "the Root's own file was not among the changed paths"
    );
    scan(&mut watched, &["own.txt"]);
    assert_eq!(changes()[0].1, ["own.txt"]);
}

#[test]
fn a_change_inside_any_git_directory_counts_as_its_repository_and_never_its_objects() {
    let mut watched = Watched::new(PathBuf::from("/work"));
    watched.git_dirs = vec![
        (
            RelativePathBuf::from("linked"),
            PathBuf::from("/elsewhere/api/.git/worktrees/linked"),
        ),
        (
            RelativePathBuf::from("api"),
            PathBuf::from("/elsewhere/api/.git"),
        ),
    ];
    let change = |path: &str| relevant_change(&watched, Path::new(path));
    assert_eq!(
        change("/work/api/src/lib.rs"),
        Some(PathBuf::from("api/src/lib.rs"))
    );
    assert_eq!(change("/work/.git/index"), Some(PathBuf::from(".git")));
    assert_eq!(
        change("/work/web/.git/HEAD"),
        Some(PathBuf::from("web/.git"))
    );
    assert_eq!(change("/work/web/.git/objects/ab/cd"), None);
    assert_eq!(
        change("/elsewhere/api/.git/worktrees/linked/HEAD"),
        Some(PathBuf::from("linked/.git"))
    );
    assert_eq!(
        change("/elsewhere/api/.git/HEAD"),
        Some(PathBuf::from("api/.git"))
    );
    assert_eq!(change("/elsewhere/api/.git/lfs/x"), None);
    assert_eq!(change("/elsewhere/other"), None);
}

/// An unborn repository needs no Git subprocess or object fixtures.
struct Repository {
    root: PathBuf,
}

impl Repository {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "condr-git-refresh-{}-{}",
            std::process::id(),
            crate::test_support::unique_suffix()
        ));
        unborn_repository(&root);
        Self { root }
    }
}

fn unborn_repository(root: &Path) {
    std::fs::create_dir_all(root.join(".git/objects")).unwrap();
    std::fs::create_dir_all(root.join(".git/refs/heads")).unwrap();
    std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(
        root.join(".git/config"),
        "[core]\nrepositoryformatversion = 0\nbare = false\n",
    )
    .unwrap();
}

impl Drop for Repository {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
