use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use atomicwrites::{AllowOverwrite, AtomicFile};
use condr_core::SessionSnapshot;

const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(250);
const RETRY_DELAY: Duration = Duration::from_secs(1);
const SHUTDOWN_RETRY_DELAY: Duration = Duration::from_millis(50);
const SHUTDOWN_WRITE_ATTEMPTS: usize = 4;
pub(crate) const MAX_SNAPSHOT_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, PartialEq)]
pub(crate) enum SnapshotLoad {
    Missing,
    Loaded(SessionSnapshot),
    Rejected(String),
}

pub(crate) struct SnapshotPersistence {
    path: PathBuf,
    debounce: Duration,
    shared: Arc<Shared>,
    worker: Option<JoinHandle<io::Result<()>>>,
    _lock_file: File,
}

struct Shared {
    state: Mutex<WorkerState>,
    wake: Condvar,
}

struct WorkerState {
    pending: Option<SessionSnapshot>,
    deadline: Option<Instant>,
    stopping: bool,
}

/// Where an atomic replace of `path` must land: a symlinked file (dotfiles, a snapshot
/// kept elsewhere) is updated through its target instead of being replaced by a regular
/// file. A dangling link is replaced as is.
pub(crate) fn resolve_write_target(path: &std::path::Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Writes the hand-editable `config.toml`, owner-only on Unix. It is replaced atomically
/// when possible; when another program holds the file open, which on Windows makes the
/// replace fail with access denied (an editor showing the file is enough), the text is
/// written in place instead so the change is not lost.
pub fn write_config_text(path: &std::path::Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let target = resolve_write_target(path);
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let atomic = atomicwrites::AtomicFile::new(&target, atomicwrites::AllowOverwrite)
        .write_with_options(|file| file.write_all(text.as_bytes()), options.clone())
        .map_err(|error| match error {
            atomicwrites::Error::Internal(error) | atomicwrites::Error::User(error) => error,
        });
    match atomic {
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            let mut file = options.open(&target)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()
        }
        result => result,
    }
}

impl SnapshotPersistence {
    pub(crate) fn open(path: PathBuf) -> io::Result<Self> {
        Self::open_inner(path, DEFAULT_DEBOUNCE)
    }

    #[cfg(test)]
    pub(crate) fn open_with_debounce(path: PathBuf, debounce: Duration) -> io::Result<Self> {
        Self::open_inner(path, debounce)
    }

    fn open_inner(path: PathBuf, debounce: Duration) -> io::Result<Self> {
        let path = std::path::absolute(path)?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Snapshot path has no parent directory",
                )
            })?;
        let file_name = path
            .file_name()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Snapshot path must name a file",
                )
            })?
            .to_os_string();
        fs::create_dir_all(parent)?;
        let path = resolve_write_target(&fs::canonicalize(parent)?.join(file_name));

        let lock_path = adjacent_lock_path(&path);
        let mut lock_options = OpenOptions::new();
        lock_options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            lock_options.mode(0o600);
        }
        let lock_file = lock_options.open(&lock_path)?;
        lock_file.try_lock().map_err(|error| {
            let error = io::Error::from(error);
            io::Error::new(
                error.kind(),
                format!(
                    "cannot lock Session Snapshot store {}: {error}",
                    path.display()
                ),
            )
        })?;

        let shared = Arc::new(Shared {
            state: Mutex::new(WorkerState {
                pending: None,
                deadline: None,
                stopping: false,
            }),
            wake: Condvar::new(),
        });
        let worker_shared = Arc::clone(&shared);
        let worker_path = path.clone();
        let worker = thread::Builder::new()
            .name("condr-snapshot".into())
            .spawn(move || run_worker_with(worker_path, worker_shared, write_snapshot))?;

        Ok(Self {
            path,
            debounce,
            shared,
            worker: Some(worker),
            _lock_file: lock_file,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn load(&self) -> SnapshotLoad {
        self.load_inner().unwrap_or_else(|error| {
            SnapshotLoad::Rejected(format!("cannot read Session Snapshot: {error}"))
        })
    }

    fn load_inner(&self) -> io::Result<SnapshotLoad> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(SnapshotLoad::Missing);
            }
            Err(error) => return Err(error),
        };
        let length = file.metadata()?.len();
        if length == 0 {
            return Ok(SnapshotLoad::Rejected(
                "Session Snapshot file is empty".into(),
            ));
        }
        if length > MAX_SNAPSHOT_BYTES {
            return Ok(SnapshotLoad::Rejected(format!(
                "Session Snapshot is {length} bytes; limit is {MAX_SNAPSHOT_BYTES} bytes"
            )));
        }

        let mut bytes = Vec::with_capacity(length as usize);
        Read::by_ref(&mut file)
            .take(MAX_SNAPSHOT_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_SNAPSHOT_BYTES {
            return Ok(SnapshotLoad::Rejected(format!(
                "Session Snapshot exceeds the {MAX_SNAPSHOT_BYTES}-byte limit"
            )));
        }
        match SessionSnapshot::from_bytes(&bytes) {
            Ok(snapshot) => Ok(SnapshotLoad::Loaded(snapshot)),
            Err(error) => Ok(SnapshotLoad::Rejected(format!(
                "cannot decode Session Snapshot: {error}"
            ))),
        }
    }

    pub(crate) fn schedule(&self, snapshot: SessionSnapshot) {
        let mut state = self
            .shared
            .state
            .lock()
            .expect("Snapshot worker lock poisoned");
        if state.stopping {
            return;
        }
        state.pending = Some(snapshot);
        state.deadline = Some(Instant::now() + self.debounce);
        self.shared.wake.notify_one();
    }

    pub(crate) fn shutdown(&mut self) -> io::Result<()> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        {
            let mut state = self
                .shared
                .state
                .lock()
                .expect("Snapshot worker lock poisoned");
            state.stopping = true;
            self.shared.wake.notify_one();
        }
        worker
            .join()
            .map_err(|_| io::Error::other("Session Snapshot worker panicked"))?
    }
}

impl Drop for SnapshotPersistence {
    fn drop(&mut self) {
        if let Err(error) = self.shutdown() {
            eprintln!(
                "condr-server: failed to flush Session Snapshot {}: {error}",
                self.path.display()
            );
        }
    }
}

fn run_worker_with(
    path: PathBuf,
    shared: Arc<Shared>,
    mut write: impl FnMut(&Path, &SessionSnapshot) -> io::Result<()>,
) -> io::Result<()> {
    let mut shutdown_write_attempts = 0;
    loop {
        let Some(snapshot) = wait_for_snapshot(&shared) else {
            return Ok(());
        };

        match write(&path, &snapshot) {
            Ok(()) => shutdown_write_attempts = 0,
            Err(error) => {
                let mut state = shared.state.lock().expect("Snapshot worker lock poisoned");
                if state.stopping {
                    if state.pending.is_some() {
                        shutdown_write_attempts = 0;
                        continue;
                    }
                    shutdown_write_attempts += 1;
                    if shutdown_write_attempts >= SHUTDOWN_WRITE_ATTEMPTS {
                        return Err(error);
                    }
                    state.pending = Some(snapshot);
                    drop(state);
                    thread::sleep(SHUTDOWN_RETRY_DELAY);
                    continue;
                }
                eprintln!(
                    "condr-server: failed to persist Session Snapshot {}: {error}",
                    path.display()
                );
                if state.pending.is_none() {
                    state.pending = Some(snapshot);
                    state.deadline = Some(Instant::now() + RETRY_DELAY);
                }
                continue;
            }
        }

        let state = shared.state.lock().expect("Snapshot worker lock poisoned");
        if state.stopping && state.pending.is_none() {
            return Ok(());
        }
    }
}

fn wait_for_snapshot(shared: &Shared) -> Option<SessionSnapshot> {
    let mut state = shared.state.lock().expect("Snapshot worker lock poisoned");
    loop {
        if state.stopping {
            return state.pending.take();
        }
        let Some(deadline) = state.deadline else {
            state = shared
                .wake
                .wait(state)
                .expect("Snapshot worker lock poisoned");
            continue;
        };
        let now = Instant::now();
        if now >= deadline {
            state.deadline = None;
            return state.pending.take();
        }
        let (next, _) = shared
            .wake
            .wait_timeout(state, deadline - now)
            .expect("Snapshot worker lock poisoned");
        state = next;
    }
}

fn write_snapshot(path: &Path, snapshot: &SessionSnapshot) -> io::Result<()> {
    let bytes = snapshot.to_bytes().map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("cannot encode Session Snapshot: {error}"),
        )
    })?;
    if bytes.len() as u64 > MAX_SNAPSHOT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Session Snapshot is {} bytes; limit is {MAX_SNAPSHOT_BYTES} bytes",
                bytes.len()
            ),
        ));
    }

    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    atomic_replace(path, options, |file| file.write_all(&bytes))
}

fn atomic_replace(
    path: &Path,
    options: OpenOptions,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    AtomicFile::new(path, AllowOverwrite)
        .write_with_options(write, options)
        .map_err(io::Error::from)
}

fn adjacent_lock_path(path: &Path) -> PathBuf {
    let mut path = path.as_os_str().to_os_string();
    path.push(".lock");
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
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

    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    use condr_core::Session;

    use super::*;

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn missing_empty_and_corrupt_snapshots_are_classified() {
        let directory = TestDirectory::new();
        let path = directory.path().join("session.bin");
        let mut persistence =
            SnapshotPersistence::open_with_debounce(path.clone(), Duration::from_millis(10))
                .unwrap();

        assert_eq!(persistence.load(), SnapshotLoad::Missing);

        fs::write(&path, []).unwrap();
        assert!(matches!(
            persistence.load(),
            SnapshotLoad::Rejected(reason) if reason.contains("empty")
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
            SnapshotPersistence::open_with_debounce(path.clone(), Duration::from_millis(10))
                .unwrap();
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
        let mut persistence =
            SnapshotPersistence::open_with_debounce(path.clone(), debounce).unwrap();
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

    /// An editor showing config.toml keeps it open without delete sharing, which makes
    /// the atomic replace fail; the save must still land.
    #[cfg(windows)]
    #[test]
    fn config_write_falls_back_to_in_place_while_another_program_holds_the_file() {
        use std::os::windows::fs::OpenOptionsExt as _;

        let directory = std::env::temp_dir().join(format!(
            "condr-config-held-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.toml");
        fs::write(&path, "old").unwrap();
        // FILE_SHARE_READ | FILE_SHARE_WRITE, no FILE_SHARE_DELETE: what a typical editor holds.
        let holder = OpenOptions::new()
            .read(true)
            .share_mode(0x1 | 0x2)
            .open(&path)
            .unwrap();

        super::write_config_text(&path, "new").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");

        drop(holder);
        let _ = fs::remove_dir_all(directory);
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
}
