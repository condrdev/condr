//! Staging for clipboard images pasted into remote Panes (ADR 0012): a private per-user
//! directory of uniquely named files the Server writes and removes itself. An image lives
//! as long as the Client that uploaded it stays connected, and never past a Server stop;
//! a stale sweep covers crashes.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use condr_core::protocol::ClipboardImageFormat;

const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
static SWEEP: Once = Once::new();

/// The staging directory: under the per-user runtime directory on Unix (owner-only), and
/// process-scoped under it on Windows, where permissions are not enforced the same way.
pub(super) fn staging_directory() -> Option<PathBuf> {
    let root = condr_core::runtime_directory()?;
    Some(if cfg!(windows) {
        root.join(format!("clipboard-images-{}", std::process::id()))
    } else {
        root.join("clipboard-images")
    })
}

/// Writes `bytes` to a fresh file named by the Server and returns its path. Exclusive
/// creation means a guessed name can never replace an existing file.
pub(super) fn stage(
    client_id: u64,
    format: ClipboardImageFormat,
    bytes: &[u8],
) -> io::Result<PathBuf> {
    let directory = staging_directory().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no runtime directory for clipboard images",
        )
    })?;
    SWEEP.call_once(|| {
        sweep_stale(&directory);
        #[cfg(windows)]
        if let Some(root) = directory.parent()
            && let Ok(entries) = fs::read_dir(root)
        {
            for entry in entries.flatten() {
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("clipboard-images-")
                    && entry.file_type().is_ok_and(|kind| kind.is_dir())
                {
                    sweep_stale(&entry.path());
                    let _ = fs::remove_dir(entry.path());
                }
            }
        }
    });
    stage_in(&directory, client_id, format, bytes)
}

fn stage_in(
    directory: &Path,
    client_id: u64,
    format: ClipboardImageFormat,
    bytes: &[u8],
) -> io::Result<PathBuf> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(directory)?;
    if !fs::symlink_metadata(directory)?.is_dir() {
        return Err(io::Error::other(format!(
            "{} is not a directory",
            directory.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or_default();
    for attempt in 0..100 {
        let path = directory.join(format!(
            "client-{}-{client_id}-{unique}-{attempt}.{}",
            std::process::id(),
            format.extension()
        ));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = match options.open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        return Ok(path);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique clipboard image path",
    ))
}

pub(super) fn remove(paths: impl IntoIterator<Item = PathBuf>) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

/// Files older than a day were left by a Server that did not get to clean up.
fn sweep_stale(directory: &Path) {
    if !fs::symlink_metadata(directory).is_ok_and(|metadata| metadata.is_dir()) {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .map(|modified| modified.elapsed().unwrap_or_default() > STALE_AFTER)
            .unwrap_or(false);
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_files_are_unique_private_and_swept_when_stale() {
        let directory = std::env::temp_dir().join(format!("condr-images-{}", uuid::Uuid::new_v4()));
        let first = stage_in(&directory, 7, ClipboardImageFormat::Png, b"one").unwrap();
        let second = stage_in(&directory, 7, ClipboardImageFormat::Jpeg, b"two").unwrap();
        assert_ne!(first, second);
        assert!(first.starts_with(&directory));
        assert_eq!(first.extension().unwrap(), "png");
        assert_eq!(second.extension().unwrap(), "jpg");
        assert_eq!(fs::read(&first).unwrap(), b"one");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&first).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        // Startup sweeps old files; subsequent uploads must not expire live images.
        let stale = directory.join("client-0-stale-0.png");
        fs::write(&stale, b"old").unwrap();
        let old = SystemTime::now() - STALE_AFTER - Duration::from_secs(60);
        fs::File::options()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let third = stage_in(&directory, 8, ClipboardImageFormat::Gif, b"three").unwrap();
        assert!(stale.exists());
        sweep_stale(&directory);
        assert!(!stale.exists());
        assert!(first.exists());

        remove([first.clone(), second.clone(), third.clone()]);
        assert!(!first.exists() && !second.exists() && !third.exists());
        remove([first]);
        fs::remove_dir(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn staging_rejects_a_symlink_directory() {
        let root = std::env::temp_dir().join(format!("condr-images-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&root, &link).unwrap();
        assert!(stage_in(&link, 1, ClipboardImageFormat::Png, b"image").is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_file(link).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
