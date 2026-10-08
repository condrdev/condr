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
    busy.record_scan(now, None);
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
        discover_repository(&fixture.root).unwrap(),
    );
    let state = Arc::new(Mutex::new(state));
    let weak = Arc::downgrade(&state);
    let mut watched = Watched::new(fixture.root.clone());
    watched.terminal_activity(Instant::now());
    assert!(refresh_workspace(workspace_id, &mut watched, None, &weak));
    assert!(watched.fingerprint.is_some());

    std::fs::write(fixture.root.join("changed.txt"), "new content\n").unwrap();
    watched.scanned_at = Some(Instant::now() - GIT_SCAN_INTERVAL);
    watched.terminal_activity(Instant::now());
    assert!(refresh_workspace(workspace_id, &mut watched, None, &weak));
    assert!(
        state.lock().unwrap().workspace_git[&workspace_id]
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
        locked.workspace_git[&workspace_id].changes.entries[0]
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
    let state = Arc::new(Mutex::new(RuntimeState::new(Path::new(
        "test-watcher.sock",
    ))));
    let mut watcher = GitWatcher::without_filesystem(Arc::downgrade(&state));
    let worker = watcher.worker.take().unwrap();
    let callback_sender = watcher.messages.clone();
    let mut locked = state.lock().unwrap();
    let workspace_id = locked
        .session
        .create_workspace(std::env::temp_dir())
        .unwrap();
    watcher.watch(workspace_id, std::env::temp_dir(), None);
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
        std::fs::create_dir_all(root.join(".git/objects")).unwrap();
        std::fs::create_dir_all(root.join(".git/refs/heads")).unwrap();
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(
            root.join(".git/config"),
            "[core]\nrepositoryformatversion = 0\nbare = false\n",
        )
        .unwrap();
        Self { root }
    }
}

impl Drop for Repository {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
