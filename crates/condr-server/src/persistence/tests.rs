use super::*;
use condr_core::Session;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[cfg(unix)]
#[test]
fn write_target_follows_a_symlinked_file() {
    let root = std::env::temp_dir().join(format!(
        "condr-write-target-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let target = root.join("real.toml");
    std::fs::write(&target, "").unwrap();
    let link = root.join("link.toml");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    assert_eq!(
        super::resolve_write_target(&link),
        std::fs::canonicalize(&target).unwrap()
    );
    let dangling = root.join("dangling.toml");
    std::os::unix::fs::symlink(root.join("missing"), &dangling).unwrap();
    assert_eq!(super::resolve_write_target(&dangling), dangling);
    let _ = std::fs::remove_dir_all(&root);
}

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

#[test]
fn missing_empty_and_corrupt_snapshots_are_classified() {
    let directory = TestDirectory::new();
    let path = directory.path().join("session.bin");
    let mut persistence =
        SnapshotPersistence::open_with_debounce(path.clone(), Duration::from_millis(10)).unwrap();

    assert_eq!(persistence.load(), SnapshotLoad::Missing);

    fs::write(&path, []).unwrap();
    assert!(matches!(
        persistence.load(),
        SnapshotLoad::Loaded(snapshot) if snapshot == Session::new().snapshot()
    ));

    fs::write(&path, [0xff, 0x00, 0x7f]).unwrap();
    assert!(matches!(
        persistence.load(),
        SnapshotLoad::Rejected(reason) if reason.contains("decode")
    ));

    fs::write(&path, vec![0; (MAX_SNAPSHOT_BYTES + 1) as usize]).unwrap();
    assert!(matches!(
        persistence.load(),
        SnapshotLoad::Rejected(reason) if reason.contains("limit")
    ));

    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(matches!(
        persistence.load(),
        SnapshotLoad::Rejected(reason) if reason.contains("read")
    ));
    persistence.shutdown().unwrap();
}

#[test]
fn atomic_write_replaces_the_previous_snapshot() {
    let directory = TestDirectory::new();
    let path = directory.path().join("session.bin");
    let mut persistence =
        SnapshotPersistence::open_with_debounce(path.clone(), Duration::from_millis(10)).unwrap();
    let first = snapshot("first");
    let second = snapshot("second");

    persistence.schedule(first.clone());
    wait_for_snapshot(&persistence, &first);
    persistence.schedule(second.clone());
    wait_for_snapshot(&persistence, &second);
    persistence.shutdown().unwrap();

    assert_eq!(
        SessionSnapshot::from_bytes(&fs::read(path).unwrap()).unwrap(),
        second
    );
}

#[test]
fn failed_atomic_write_preserves_the_previous_file() {
    let directory = TestDirectory::new();
    let path = directory.path().join("session.bin");
    fs::write(&path, b"previous complete snapshot").unwrap();
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);

    let error = atomic_replace(&path, options, |file| {
        file.write_all(b"partial replacement")?;
        Err(io::Error::other("injected write failure"))
    })
    .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert_eq!(fs::read(path).unwrap(), b"previous complete snapshot");
}

#[test]
fn debounce_keeps_the_latest_snapshot() {
    let directory = TestDirectory::new();
    let path = directory.path().join("session.bin");
    let debounce = Duration::from_millis(600);
    let mut persistence = SnapshotPersistence::open_with_debounce(path.clone(), debounce).unwrap();
    let latest = snapshot("latest");

    persistence.schedule(snapshot("first"));
    thread::sleep(Duration::from_millis(100));
    persistence.schedule(snapshot("second"));
    thread::sleep(Duration::from_millis(100));
    persistence.schedule(latest.clone());
    thread::sleep(Duration::from_millis(450));
    assert!(
        !path.exists(),
        "the first deadline must not flush an obsolete Snapshot"
    );
    wait_for_snapshot(&persistence, &latest);
    persistence.shutdown().unwrap();
}

#[test]
fn shutdown_retries_a_transient_write_failure() {
    let expected = snapshot("tail");
    let shared = Arc::new(Shared {
        state: Mutex::new(WorkerState {
            pending: Some(expected.clone()),
            deadline: None,
            stopping: true,
        }),
        wake: Condvar::new(),
    });
    let attempts = Arc::new(AtomicUsize::new(0));
    let writer_attempts = Arc::clone(&attempts);

    run_worker_with(PathBuf::from("unused"), shared, move |_, snapshot| {
        assert_eq!(snapshot, &expected);
        let attempt = writer_attempts.fetch_add(1, Ordering::Relaxed);
        if attempt < 2 {
            Err(io::Error::other("transient sharing violation"))
        } else {
            Ok(())
        }
    })
    .unwrap();

    assert_eq!(attempts.load(Ordering::Relaxed), 3);
}

#[test]
fn shutdown_stops_retrying_after_the_bounded_attempts() {
    let shared = Arc::new(Shared {
        state: Mutex::new(WorkerState {
            pending: Some(snapshot("tail")),
            deadline: None,
            stopping: true,
        }),
        wake: Condvar::new(),
    });
    let attempts = AtomicUsize::new(0);

    let error = run_worker_with(PathBuf::from("unused"), shared, |_, _| {
        attempts.fetch_add(1, Ordering::Relaxed);
        Err(io::Error::other("persistent failure"))
    })
    .unwrap_err();

    assert_eq!(error.to_string(), "persistent failure");
    assert_eq!(attempts.load(Ordering::Relaxed), SHUTDOWN_WRITE_ATTEMPTS);
}

#[test]
fn shutdown_flushes_a_snapshot_before_the_debounce_deadline() {
    let directory = TestDirectory::new();
    let path = directory.path().join("session.bin");
    let mut persistence =
        SnapshotPersistence::open_with_debounce(path.clone(), Duration::from_secs(60)).unwrap();
    let expected = snapshot("tail");

    persistence.schedule(expected.clone());
    persistence.shutdown().unwrap();

    assert_eq!(
        SessionSnapshot::from_bytes(&fs::read(path).unwrap()).unwrap(),
        expected
    );
}

#[test]
fn snapshot_store_lock_is_exclusive_until_the_owner_is_dropped() {
    let directory = TestDirectory::new();
    let path = directory.path().join("session.bin");
    let mut owner = SnapshotPersistence::open(path.clone()).unwrap();

    let error = match SnapshotPersistence::open(path.clone()) {
        Ok(_) => panic!("a second Snapshot writer must not acquire the same path"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("cannot lock"));

    owner.shutdown().unwrap();
    drop(owner);
    let mut replacement = SnapshotPersistence::open(path).unwrap();
    replacement.shutdown().unwrap();
}

fn snapshot(root: &str) -> SessionSnapshot {
    let mut session = Session::new();
    session
        .create_workspace(PathBuf::from(root))
        .expect("Workspace capacity");
    session.snapshot()
}

fn wait_for_snapshot(persistence: &SnapshotPersistence, expected: &SessionSnapshot) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if matches!(
            persistence.load(),
            SnapshotLoad::Loaded(snapshot) if &snapshot == expected
        ) {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("latest Session Snapshot was not persisted");
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "condr-persistence-{}-{}",
            std::process::id(),
            NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
