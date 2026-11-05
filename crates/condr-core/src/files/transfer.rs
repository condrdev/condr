//! What both ends of a file transfer share (ADR 0061): the temporary file a transfer
//! writes beside its target, placing it under the target's name only when complete, and
//! which names a machine can hold. The Server writes uploads with them and a Client
//! writes downloads.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(windows)]
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

/// The infix of a transfer's temporary name: `.<name>.condr-upload-<random>`.
const TEMP_INFIX: &str = ".condr-";
/// The longest name a common filesystem holds, in bytes.
const MAX_NAME_BYTES: usize = 255;
/// How long a rename or removal on Windows waits out a sharing violation from an
/// antivirus or an indexer that opened the new file.
#[cfg(windows)]
const SHARING_RETRY: Duration = Duration::from_secs(1);

/// Whether `name` is a transfer's temporary file, which the Git watcher ignores.
pub fn is_transfer_temp(name: &str) -> bool {
    name.starts_with('.')
        && [".condr-upload-", ".condr-download-"]
            .iter()
            .any(|infix| name.contains(infix))
}

/// Creates `.<name>.condr-<kind>-<random>` beside `target`, exclusively, on the same
/// filesystem, so the final step can be a link or a rename. `<name>` is shortened to keep
/// the whole within 255 bytes.
pub fn create_transfer_temp(target: &Path, kind: &str) -> io::Result<(PathBuf, File)> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    for _ in 0..16 {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos();
        let random = format!(
            "{:x}{:x}{:x}",
            std::process::id(),
            nanos,
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let suffix = format!("{TEMP_INFIX}{kind}-{random}");
        let room = MAX_NAME_BYTES.saturating_sub(1 + suffix.len());
        let mut cut = name.len().min(room);
        while !name.is_char_boundary(cut) {
            cut -= 1;
        }
        let path = parent.join(format!(".{}{suffix}", &name[..cut]));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free temporary name",
    ))
}

/// Puts the synced temporary file `temp` in `target`'s place. Without `replace` the
/// target must not exist, and a link makes that check and the placing one step, so a
/// file another program created meanwhile is never overwritten; on a filesystem without
/// links a rename follows a check, which only another program can race. With `replace`
/// the old file's permission bits carry over and a rename replaces it.
pub fn place_transfer(temp: &Path, target: &Path, replace: bool) -> io::Result<()> {
    if !replace {
        return match fs::hard_link(temp, target) {
            Ok(()) => {
                // The file is in place; a second name left behind is litter, not failure.
                if let Err(error) = retried(|| fs::remove_file(temp)) {
                    tracing::warn!(path = %temp.display(), "could not remove a transfer's temporary name: {error}");
                }
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(error),
            Err(_) => {
                if fs::symlink_metadata(target).is_ok() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "the target exists",
                    ));
                }
                rename(temp, target)
            }
        };
    }
    if let Ok(meta) = fs::metadata(target) {
        let permissions = meta.permissions();
        // Windows renames over no read-only file: the old one is made writable for the
        // rename, and the file in place, new or old, is read-only again after it.
        #[cfg(windows)]
        if permissions.readonly() {
            let mut writable = permissions.clone();
            #[allow(clippy::permissions_set_readonly_false)] // Windows: only the attribute.
            writable.set_readonly(false);
            fs::set_permissions(target, writable)?;
            let placed = rename(temp, target);
            let _ = fs::set_permissions(target, permissions);
            return placed;
        }
        let _ = fs::set_permissions(temp, permissions);
    }
    rename(temp, target)
}

fn rename(from: &Path, to: &Path) -> io::Result<()> {
    retried(|| fs::rename(from, to))
}

/// Runs `operation`, on Windows again for a second while a scanner holding the file
/// makes it fail with ERROR_SHARING_VIOLATION or ERROR_ACCESS_DENIED.
fn retried(mut operation: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    #[cfg(windows)]
    {
        let started = Instant::now();
        loop {
            match operation() {
                Err(error)
                    if matches!(error.raw_os_error(), Some(32 | 5))
                        && started.elapsed() < SHARING_RETRY =>
                {
                    std::thread::sleep(Duration::from_millis(50));
                }
                result => return result,
            }
        }
    }
    #[cfg(not(windows))]
    operation()
}

/// Whether this machine can hold `name` as an ordinary file: on Windows no alternate
/// data stream, no trailing dot or space and no device name; everywhere no separator and
/// no NUL.
pub fn valid_local_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\0'])
        && (!cfg!(windows) || (!name.contains('\\') && super::valid_windows_name(name)))
}

/// `name` with what this machine cannot hold replaced by `_`, for a save dialog's
/// suggestion.
pub fn local_name(name: &str) -> String {
    if valid_local_name(name) {
        return name.to_owned();
    }
    let replaced: String = name
        .chars()
        .map(|c| {
            if matches!(
                c,
                ':' | '<' | '>' | '"' | '|' | '?' | '*' | '/' | '\\' | '\0'
            ) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = replaced.trim_end_matches(['.', ' ']);
    if valid_local_name(trimmed) {
        trimmed.to_owned()
    } else {
        format!("_{trimmed}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "condr-transfer-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn a_temporary_file_sits_beside_its_target_and_is_recognized() {
        let root = directory("temp");
        let (temp, _) = create_transfer_temp(&root.join("report.pdf"), "upload").unwrap();
        assert_eq!(temp.parent().unwrap(), root);
        let name = temp.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with(".report.pdf.condr-upload-"));
        assert!(is_transfer_temp(name));
        assert!(!is_transfer_temp("report.pdf"));
        let long = "a".repeat(300);
        let (temp, _) = create_transfer_temp(&root.join(&long[..250]), "download").unwrap();
        assert!(temp.file_name().unwrap().len() <= MAX_NAME_BYTES);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn placing_without_replace_never_overwrites_and_replacing_keeps_permissions() {
        let root = directory("place");
        let target = root.join("a.txt");
        let (temp, _) = create_transfer_temp(&target, "upload").unwrap();
        fs::write(&temp, "new").unwrap();
        // Another program created the target after the transfer began.
        fs::write(&target, "theirs").unwrap();
        let error = place_transfer(&temp, &target, false).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&target).unwrap(), "theirs");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o750)).unwrap();
            place_transfer(&temp, &target, true).unwrap();
            assert_eq!(
                fs::metadata(&target).unwrap().permissions().mode() & 0o777,
                0o750
            );
        }
        #[cfg(not(unix))]
        place_transfer(&temp, &target, true).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        assert!(!temp.exists());
        let fresh = root.join("b.txt");
        let (temp, _) = create_transfer_temp(&fresh, "upload").unwrap();
        fs::write(&temp, "b").unwrap();
        place_transfer(&temp, &fresh, false).unwrap();
        assert_eq!(fs::read_to_string(&fresh).unwrap(), "b");
        assert!(!temp.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn local_names_follow_this_machines_rules() {
        assert!(valid_local_name("report.pdf"));
        assert!(!valid_local_name("a/b"));
        assert!(!valid_local_name(""));
        assert_eq!(local_name("report.pdf"), "report.pdf");
        if cfg!(windows) {
            assert!(!valid_local_name("a:b.txt"));
            assert!(!valid_local_name("CON"));
            assert_eq!(local_name("a:b.txt"), "a_b.txt");
        }
    }
}
