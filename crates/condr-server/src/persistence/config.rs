//! Serialized TOML edits shared by the CLI, Server and GUI.

use super::*;

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
        let path =
            resolve_write_target(&fs::canonicalize(parent)?.join(path.file_name().ok_or_else(
                || io::Error::new(io::ErrorKind::InvalidInput, "config path must name a file"),
            )?));
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(adjacent_lock_path(&path))?;
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
    let result = atomic_replace(path, options, |file| file.write_all(text.as_bytes()));
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
