//! The Server's database (ADR 0068): `server.sqlite` in its data directory, one file for
//! everything the Server keeps in SQLite, each part in tables of its own. Only the Server
//! opens it, and holds it exclusively while it runs. Its schema is the migrations under
//! `migrations/`, which refinery applies in order: a shipped migration is never edited,
//! a change is a new one. A file this Server cannot read is set aside, never overwritten.

use rusqlite::{Connection, ErrorCode};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

refinery::embed_migrations!("migrations");

/// The database's name in the Server's data directory.
pub(crate) const FILE_NAME: &str = "server.sqlite";

/// Opens the database at `path`, creating it owner-only when missing, brings its schema
/// to this build's and reads what the Server keeps there with `read`. It stays held
/// exclusively until the connection closes, so a second Server on the same file cannot
/// write over this one's. One that is not a database, or whose migrations are not this
/// build's (a newer build's, or an edited one), is renamed aside to
/// `<file>.unreadable-<unix seconds>` and a new one starts empty, which the log says. Any
/// other failure, one held by another Server included, leaves the file alone and is the
/// caller's error.
pub(crate) fn open<T>(
    path: &Path,
    read: impl Fn(&Connection) -> rusqlite::Result<T>,
) -> io::Result<(Connection, T)> {
    if let Some(parent) = path.parent() {
        crate::persistence::create_private_directory(parent)?;
    }
    // SQLite gives its log files the database's permissions.
    crate::persistence::private_file_options().open(path)?;
    match load(path, &read) {
        Ok(loaded) => Ok(loaded),
        Err(LoadError::Unreadable(reason)) => {
            let aside = set_aside(path)?;
            tracing::error!(
                "set aside the Server database {} as {}: {reason}; this Server starts without \
                 what it kept there and leaves that file for you",
                path.display(),
                aside.display()
            );
            crate::persistence::private_file_options().open(path)?;
            load(path, &read).map_err(|error| {
                io::Error::other(format!("cannot create {}: {error}", path.display()))
            })
        }
        Err(LoadError::Unavailable(reason)) => Err(io::Error::other(reason)),
    }
}

/// Why the database did not open.
#[derive(Debug)]
enum LoadError {
    /// Not a database, a damaged one, or one whose migrations are not this build's: set
    /// aside (ADR 0068).
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
    classify(error.sqlite_error_code(), error.to_string())
}

fn classify(code: Option<ErrorCode>, message: String) -> LoadError {
    match code {
        Some(ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt) => {
            LoadError::Unreadable(message)
        }
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => {
            LoadError::Unavailable(format!("another Server holds it: {message}"))
        }
        _ => LoadError::Unavailable(message),
    }
}

fn migration(error: refinery::Error) -> LoadError {
    use refinery::error::Kind;
    match error.kind() {
        // A migration this build lacks, a newer build's, or one edited since it ran.
        Kind::MissingVersion(_) | Kind::DivergentVersion(..) => {
            LoadError::Unreadable(error.to_string())
        }
        Kind::Connection(_, source) => classify(
            source
                .downcast_ref::<rusqlite::Error>()
                .and_then(rusqlite::Error::sqlite_error_code),
            error.to_string(),
        ),
        _ => LoadError::Unavailable(error.to_string()),
    }
}

/// Opens the database, takes its lock, migrates it and reads it.
fn load<T>(
    path: &Path,
    read: &impl Fn(&Connection) -> rusqlite::Result<T>,
) -> Result<(Connection, T), LoadError> {
    let mut connection = Connection::open(path).map_err(sqlite)?;
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
    connection
        .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))
        .map_err(sqlite)?;
    // A write takes the exclusive lock now, not at the first change.
    connection
        .execute_batch("BEGIN EXCLUSIVE; COMMIT;")
        .map_err(sqlite)?;
    migrations::runner()
        .run(&mut connection)
        .map_err(migration)?;
    let read = read(&connection).map_err(sqlite)?;
    Ok((connection, read))
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
