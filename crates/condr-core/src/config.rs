//! `config.toml`: the one hand-editable file the CLI, Server and GUI share. Every read
//! and write goes through one cross-process lock, and writes edit the parsed document in
//! place so other keys, comments and formatting survive.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use atomicwrites::{AllowOverwrite, AtomicFile};

/// The shared `config.toml` under [`config_directory`](crate::config_directory).
pub fn config_path() -> Option<PathBuf> {
    crate::config_directory().map(|root| root.join("config.toml"))
}

struct ConfigTransaction {
    path: PathBuf,
    _lock: File,
}

impl ConfigTransaction {
    fn open(path: &Path) -> io::Result<Self> {
        let path = std::path::absolute(path)?;
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "config path has no parent")
        })?;
        fs::create_dir_all(parent)?;
        let path = fs::canonicalize(parent)?.join(path.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "config path must name a file")
        })?);
        // A symlinked config (dotfiles) is updated through its target instead of being
        // replaced by a regular file; a dangling link is replaced as is.
        let path = fs::canonicalize(&path).unwrap_or(path);
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lock.lock()?;
        Ok(Self { path, _lock: lock })
    }

    fn read(&self) -> io::Result<Option<String>> {
        match fs::read_to_string(&self.path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

/// Reads under the same cross-process lock as updates, including Windows in-place saves.
/// Call on a background thread when used by an interactive client.
pub fn read_config_text(path: &Path) -> io::Result<Option<String>> {
    ConfigTransaction::open(path)?.read()
}

/// One key of the table nested under `tables`; `None` when the file, a table or the key
/// is absent. Malformed TOML is `InvalidData`, so callers can report it or fall back.
pub fn read_config_value(
    path: &Path,
    tables: &[&str],
    key: &str,
) -> io::Result<Option<toml::Value>> {
    let Some(text) = read_config_text(path)? else {
        return Ok(None);
    };
    let root: toml::Table = toml::from_str(&text)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let mut table = Some(&root);
    for name in tables {
        table = table
            .and_then(|table| table.get(*name))
            .and_then(toml::Value::as_table);
    }
    Ok(table.and_then(|table| table.get(key)).cloned())
}

/// Updates keys in one TOML table as one serialized transaction; `None` removes a key.
/// Unchanged values, comments and formatting remain in the document.
pub fn update_config_values(
    path: &Path,
    tables: &[&str],
    entries: impl IntoIterator<Item = (impl AsRef<str>, Option<toml_edit::Item>)>,
) -> io::Result<()> {
    let transaction = ConfigTransaction::open(path)?;
    let mut document = transaction
        .read()?
        .unwrap_or_default()
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut table: &mut dyn toml_edit::TableLike = document.as_table_mut();
    for name in tables {
        table = table
            .entry(name)
            .or_insert(toml_edit::table())
            .as_table_like_mut()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{name} must be a table"),
                )
            })?;
    }
    for (key, value) in entries {
        let key = key.as_ref();
        let Some(value) = value else {
            table.remove(key);
            continue;
        };
        match (
            table.get_mut(key).and_then(toml_edit::Item::as_value_mut),
            value.as_value(),
        ) {
            (Some(existing), Some(replacement)) => {
                let decor = existing.decor().clone();
                *existing = replacement.clone();
                *existing.decor_mut() = decor;
            }
            _ => {
                table.insert(key, value);
            }
        }
    }
    write_config_text(&transaction.path, &document.to_string())
}

/// The caller holds the config transaction lock through persistence.
fn write_config_text(path: &Path, text: &str) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let result = AtomicFile::new(path, AllowOverwrite)
        .write_with_options(|file| file.write_all(text.as_bytes()), options)
        .map_err(io::Error::from);
    #[cfg(windows)]
    if let Err(error) = &result
        && matches!(error.raw_os_error(), Some(5 | 32 | 33))
    {
        // Editors may omit FILE_SHARE_DELETE. All Condr readers hold the transaction
        // lock, so this fallback cannot expose partial TOML to another Condr process.
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.write_all(text.as_bytes())?;
        return file.sync_all();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_walk_nested_tables_and_none_removes_a_key() {
        let directory = std::env::temp_dir().join(format!(
            "condr-core-config-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = directory.join("config.toml");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            &path,
            "[server]\nlisten = '127.0.0.1:4242'\n\n[client]\nappearance = 'light'\n",
        )
        .unwrap();

        update_config_values(
            &path,
            &["server", "terminal"],
            [("shell", Some(toml_edit::value("nu")))],
        )
        .unwrap();
        update_config_values(&path, &["client"], [("appearance", None)]).unwrap();

        assert_eq!(
            read_config_value(&path, &["server", "terminal"], "shell")
                .unwrap()
                .as_ref()
                .and_then(toml::Value::as_str),
            Some("nu")
        );
        assert_eq!(
            read_config_value(&path, &["client"], "appearance").unwrap(),
            None
        );
        assert_eq!(read_config_value(&path, &["missing"], "key").unwrap(), None);
        assert_eq!(
            read_config_value(&directory.join("absent.toml"), &[], "key").unwrap(),
            None
        );

        fs::write(&path, "[server\n").unwrap();
        assert_eq!(
            read_config_value(&path, &["server"], "listen")
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
