mod config;

use atomicwrites::{AllowOverwrite, AtomicFile};
use condr_core::SessionSnapshot;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub use config::{read_config_text, update_config_values};

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
            tracing::error!(
                "failed to flush Session Snapshot {}: {error}",
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
                tracing::error!(
                    "failed to persist Session Snapshot {}: {error}",
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
mod tests;
