//! The Device's Tasks on disk (ADR 0068): one SQLite database in the data directory,
//! which only the Server opens. Tasks live in the Server's state under its lock; this
//! worker writes their changes off that lock, as the Snapshot worker writes the Session
//! (ADR 0005). A file this Server cannot read is set aside, never overwritten.

use crate::poison::{OrAbort, abort_server};
use condr_core::{LastPlace, Task, TaskId, WorkspaceId};
use rusqlite::{Connection, params};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The schema this build writes, kept in `PRAGMA user_version`.
pub(crate) const SCHEMA_VERSION: i64 = 2;
const DEBOUNCE: Duration = Duration::from_millis(100);
const RETRY_DELAY: Duration = Duration::from_secs(1);
const SHUTDOWN_RETRY_DELAY: Duration = Duration::from_millis(50);
const SHUTDOWN_WRITE_ATTEMPTS: usize = 4;

const SCHEMA: &str = "
CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    repository TEXT,
    position INTEGER NOT NULL,
    archived INTEGER NOT NULL,
    workspace INTEGER,
    made_workspace INTEGER NOT NULL,
    last_path TEXT,
    last_branch TEXT,
    last_base_branch TEXT,
    last_managed INTEGER NOT NULL,
    last_repository TEXT
);
";

/// A change to one Task: the whole Task, or `None` once it is deleted.
type Writes = BTreeMap<TaskId, Option<Task>>;

pub(crate) struct TaskStore {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<io::Result<()>>>,
}

struct Shared {
    state: Mutex<Pending>,
    wake: Condvar,
}

#[derive(Default)]
struct Pending {
    writes: Writes,
    deadline: Option<Instant>,
    stopping: bool,
}

/// What a store opened with.
pub(crate) struct OpenedTasks {
    pub(crate) store: TaskStore,
    /// Every Task, archived ones included, with its description.
    pub(crate) tasks: Vec<Task>,
}

impl TaskStore {
    /// Opens the database at `path`, creating it owner-only when missing, and holds it
    /// exclusively until the store stops, so a second Server on the same file cannot write
    /// over this one's Tasks. One whose schema is newer than this build's, or that is not
    /// a database, is renamed aside to `<file>.unreadable-<unix seconds>` and a new one
    /// starts empty, which the log says. Any other failure, one held by another Server
    /// included, leaves the file alone and is the caller's error.
    pub(crate) fn open(path: &Path) -> io::Result<OpenedTasks> {
        if let Some(parent) = path.parent() {
            crate::persistence::create_private_directory(parent)?;
        }
        // SQLite gives its log files the database's permissions.
        crate::persistence::private_file_options().open(path)?;
        let (connection, tasks) = match load(path) {
            Ok(loaded) => loaded,
            Err(LoadError::Unreadable(reason)) => {
                let aside = set_aside(path)?;
                tracing::error!(
                    "set aside the task store {} as {}: {reason}; this Server starts with no \
                     Tasks and leaves that file for you",
                    path.display(),
                    aside.display()
                );
                crate::persistence::private_file_options().open(path)?;
                load(path).map_err(|error| {
                    io::Error::other(format!("cannot create {}: {error}", path.display()))
                })?
            }
            Err(LoadError::Unavailable(reason)) => return Err(io::Error::other(reason)),
        };
        let shared = Arc::new(Shared {
            state: Mutex::new(Pending::default()),
            wake: Condvar::new(),
        });
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("condr-tasks".into())
            .spawn(move || {
                // A worker that died would leave every later change unsaved without a
                // word, so its panic ends the Server instead (ADR 0053).
                panic::catch_unwind(AssertUnwindSafe(|| run_worker(connection, &worker_shared)))
                    .unwrap_or_else(|_| abort_server("the task store worker stopped"))
            })?;
        Ok(OpenedTasks {
            store: Self {
                shared,
                worker: Some(worker),
            },
            tasks,
        })
    }

    /// Writes `task` soon, or deletes `id` when it is `None`; a later change to the same
    /// Task replaces this one.
    pub(crate) fn schedule(&self, id: TaskId, task: Option<Task>) {
        let mut state = self.shared.state.lock().or_abort();
        if state.stopping {
            return;
        }
        state.writes.insert(id, task);
        state
            .deadline
            .get_or_insert_with(|| Instant::now() + DEBOUNCE);
        self.shared.wake.notify_one();
    }

    /// Writes what is pending and stops the worker.
    pub(crate) fn shutdown(&mut self) -> io::Result<()> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        {
            let mut state = self.shared.state.lock().or_abort();
            state.stopping = true;
            self.shared.wake.notify_one();
        }
        worker
            .join()
            .map_err(|_| io::Error::other("the task store worker panicked"))?
    }
}

impl Drop for TaskStore {
    fn drop(&mut self) {
        if let Err(error) = self.shutdown() {
            tracing::error!("failed to write the last Task changes: {error}");
        }
    }
}

/// Why a store did not open.
#[derive(Debug)]
enum LoadError {
    /// Not a database, a damaged one, or one from a newer Server: set aside (ADR 0068).
    Unreadable(String),
    /// Anything else, in use by another Server, a full disk or a failed read, which may
    /// pass: the file is left as it is.
    Unavailable(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(reason) | Self::Unavailable(reason) => formatter.write_str(reason),
        }
    }
}

fn sqlite(error: rusqlite::Error) -> LoadError {
    use rusqlite::ErrorCode;
    match error.sqlite_error_code() {
        Some(ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt) => {
            LoadError::Unreadable(error.to_string())
        }
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => {
            LoadError::Unavailable(format!("another Server holds it: {error}"))
        }
        _ => LoadError::Unavailable(error.to_string()),
    }
}

/// Opens and reads the database, creating its schema in a new one, and takes its lock.
fn load(path: &Path) -> Result<(Connection, Vec<Task>), LoadError> {
    let connection = Connection::open(path).map_err(sqlite)?;
    // Another Server holds its lock for as long as it runs: waiting would not help.
    connection.busy_timeout(Duration::ZERO).map_err(sqlite)?;
    // Held from the first write until the connection closes; with it set before WAL is
    // entered, SQLite keeps no shared-memory index either.
    connection
        .pragma_update_and_check(None, "locking_mode", "EXCLUSIVE", |row| {
            row.get::<_, String>(0)
        })
        .map_err(sqlite)?;
    // The first read of the file: one that is not a database fails here.
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(sqlite)?;
    // A newer build's, or an early one's that numbered its Tasks: there is no migration
    // before the first release.
    if version != 0 && version != SCHEMA_VERSION {
        return Err(LoadError::Unreadable(format!(
            "its schema version {version} is not this Server's {SCHEMA_VERSION}"
        )));
    }
    connection
        .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))
        .map_err(sqlite)?;
    // A write takes the exclusive lock now, not at the first change.
    connection
        .execute_batch("BEGIN EXCLUSIVE; COMMIT;")
        .map_err(sqlite)?;
    if version == 0 {
        connection.execute_batch(SCHEMA).map_err(sqlite)?;
        connection
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(sqlite)?;
    }
    let tasks = {
        let mut statement = connection
            .prepare(
                "SELECT id, title, description, repository, position, archived, workspace, \
                 last_path, last_branch, last_base_branch, last_managed, last_repository, \
                 made_workspace FROM tasks",
            )
            .map_err(sqlite)?;
        let rows = statement
            .query_map([], |row| {
                let last_path: Option<String> = row.get(7)?;
                Ok(Task {
                    id: row.get::<_, String>(0)?.parse().map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?,
                    title: row.get(1)?,
                    description: Some(row.get(2)?),
                    repository: row.get::<_, Option<String>>(3)?.map(PathBuf::from),
                    order: row.get(4)?,
                    archived: row.get(5)?,
                    workspace: row
                        .get::<_, Option<i64>>(6)?
                        .map(|id| WorkspaceId::from_u64(id as u64)),
                    made_workspace: row.get(12)?,
                    starting: false,
                    last: match last_path {
                        Some(path) => Some(LastPlace {
                            path: PathBuf::from(path),
                            branch: row.get(8)?,
                            base_branch: row.get(9)?,
                            managed: row.get(10)?,
                            repository: row.get::<_, Option<String>>(11)?.map(PathBuf::from),
                        }),
                        None => None,
                    },
                })
            })
            .map_err(sqlite)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(sqlite)?
    };
    Ok((connection, tasks))
}

/// Renames the database and its WAL files to `<file>.unreadable-<unix seconds>`, so a new
/// one never applies an old log.
fn set_aside(path: &Path) -> io::Result<PathBuf> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let suffixed = |suffix: &str, extra: &str| {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        name.push(extra);
        PathBuf::from(name)
    };
    let aside = suffixed("", &format!(".unreadable-{seconds}"));
    fs::rename(path, &aside)?;
    for log in ["-wal", "-shm"] {
        let from = suffixed(log, "");
        if from.exists() {
            fs::rename(&from, suffixed(log, &format!(".unreadable-{seconds}")))?;
        }
    }
    Ok(aside)
}

fn run_worker(mut connection: Connection, shared: &Shared) -> io::Result<()> {
    let mut shutdown_attempts = 0;
    loop {
        let Some(writes) = wait_for_writes(shared) else {
            return Ok(());
        };
        let Err(error) = write(&mut connection, &writes) else {
            continue;
        };
        let mut state = shared.state.lock().or_abort();
        // Newer changes to the same Tasks win over the ones that failed.
        for (id, task) in writes {
            state.writes.entry(id).or_insert(task);
        }
        if state.stopping {
            shutdown_attempts += 1;
            if shutdown_attempts >= SHUTDOWN_WRITE_ATTEMPTS {
                return Err(io::Error::other(format!("cannot write Tasks: {error}")));
            }
            drop(state);
            thread::sleep(SHUTDOWN_RETRY_DELAY);
            continue;
        }
        tracing::error!("cannot write Tasks; retrying: {error}");
        state.deadline = Some(Instant::now() + RETRY_DELAY);
    }
}

fn wait_for_writes(shared: &Shared) -> Option<Writes> {
    let mut state = shared.state.lock().or_abort();
    loop {
        if state.stopping {
            return (!state.writes.is_empty()).then(|| std::mem::take(&mut state.writes));
        }
        let Some(deadline) = state.deadline else {
            state = shared.wake.wait(state).or_abort();
            continue;
        };
        let now = Instant::now();
        if now >= deadline {
            state.deadline = None;
            return Some(std::mem::take(&mut state.writes));
        }
        state = shared.wake.wait_timeout(state, deadline - now).or_abort().0;
    }
}

fn write(connection: &mut Connection, writes: &Writes) -> rusqlite::Result<()> {
    let text = |path: &Option<PathBuf>| {
        path.as_ref()
            .map(|path| path.to_string_lossy().into_owned())
    };
    let transaction = connection.transaction()?;
    for (id, task) in writes {
        let Some(task) = task else {
            transaction.execute("DELETE FROM tasks WHERE id = ?1", [id.to_string()])?;
            continue;
        };
        let last = task.last.as_ref();
        transaction.execute(
            "INSERT OR REPLACE INTO tasks (id, title, description, repository, position, \
             archived, workspace, last_path, last_branch, last_base_branch, last_managed, \
             last_repository, made_workspace) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                id.to_string(),
                task.title,
                task.description.as_deref().unwrap_or_default(),
                text(&task.repository),
                task.order,
                task.archived,
                task.workspace.map(|id| id.as_u64() as i64),
                last.map(|last| last.path.to_string_lossy().into_owned()),
                last.and_then(|last| last.branch.clone()),
                last.and_then(|last| last.base_branch.clone()),
                last.is_some_and(|last| last.managed),
                last.and_then(|last| text(&last.repository)),
                task.made_workspace,
            ],
        )?;
    }
    transaction.commit()
}

#[cfg(test)]
mod tests;
