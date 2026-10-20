//! The Server's side of editing a Workspace (ADR 0045). The checks guard against mistakes,
//! not against the Client: whoever may send these may also type into a Pane.

use std::fs;
use std::io;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use relative_path::RelativePath;

use super::*;

/// Held by every mutation from its checks to its last write, so Condr's own changes never
/// interleave, whichever Client or path names the file. Other programs are not held off.
static MUTATIONS: Mutex<()> = Mutex::new(());

/// Applies `mutation` under `root`.
pub fn mutate_file(root: &Path, mutation: &FileMutation) -> FileMutationResult {
    let _serialized = MUTATIONS.lock().unwrap_or_else(PoisonError::into_inner);
    let result = match mutation {
        FileMutation::Write { path, text, base } => write(root, path, text, *base),
        FileMutation::Create { path, kind } => create(root, path, *kind),
        FileMutation::Rename { from, to } => rename(root, from, to),
        FileMutation::Delete { path } => delete(root, path),
    };
    result.unwrap_or_else(|outcome| outcome)
}

type Outcome = Result<FileMutationResult, FileMutationResult>;

fn failed(error: io::Error) -> FileMutationResult {
    FileMutationResult::Failed(error.to_string())
}

fn done() -> Outcome {
    Ok(FileMutationResult::Done { version: None })
}

fn write(root: &Path, path: &RelativePath, text: &str, base: Option<u64>) -> Outcome {
    check_path(path)?;
    let bytes = text.as_bytes();
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(FileMutationResult::Invalid(format!(
            "larger than {} KiB",
            MAX_FILE_BYTES / 1024
        )));
    }
    let native = path.to_path(root);
    // Through a symlink, wherever it points, as `read_file` reads through it.
    match fs::metadata(&native) {
        Ok(meta) if !meta.is_file() => {
            return Err(FileMutationResult::Invalid("not a regular file".into()));
        }
        Ok(meta) => {
            if let Some(base) = base {
                // A version only ever describes a file within the limit.
                if meta.len() > MAX_FILE_BYTES
                    || file_version(&fs::read(&native).map_err(failed)?) != base
                {
                    return Err(FileMutationResult::Conflict);
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound && base.is_some() => {
            return Err(FileMutationResult::Conflict);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(failed(error)),
    }
    // In place, so permissions, ownership, hard links and a symlink survive; a failure
    // partway leaves the file partial, and the editor still holds the whole text.
    fs::write(&native, bytes).map_err(failed)?;
    Ok(FileMutationResult::Done {
        version: Some(file_version(bytes)),
    })
}

fn create(root: &Path, path: &RelativePath, kind: FileKind) -> Outcome {
    check_path(path)?;
    let native = path.to_path(root);
    let created = match kind {
        FileKind::File => fs::File::create_new(&native).map(drop),
        FileKind::Directory => fs::create_dir(&native),
    };
    created.map_err(exists_or_failed)?;
    done()
}

fn rename(root: &Path, from: &RelativePath, to: &RelativePath) -> Outcome {
    check_path(from)?;
    check_path(to)?;
    if from.parent() != to.parent() {
        return Err(FileMutationResult::Invalid(
            "a rename stays in its directory".into(),
        ));
    }
    if from == to {
        return done();
    }
    let (from, to) = (from.to_path(root), to.to_path(root));
    fs::symlink_metadata(&from).map_err(failed)?;
    // A dangling symlink is an entry too. On a case- or normalization-insensitive volume
    // the target may be the source itself under another spelling, which may be renamed.
    match fs::symlink_metadata(&to) {
        Ok(_) if !same_entry(&from, &to) => {
            return Err(FileMutationResult::Invalid("already exists".into()));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(failed(error)),
    }
    fs::rename(&from, &to).map_err(failed)?;
    done()
}

fn delete(root: &Path, path: &RelativePath) -> Outcome {
    check_path(path)?;
    let native = path.to_path(root);
    // The entry itself: a symlink is removed, never what it points at, and
    // `remove_dir_all` does not follow the links inside the tree it removes.
    let meta = fs::symlink_metadata(&native).map_err(failed)?;
    #[cfg(windows)]
    let link_to_directory = std::os::windows::fs::FileTypeExt::is_symlink_dir(&meta.file_type());
    #[cfg(not(windows))]
    let link_to_directory = false;
    let removed = if meta.is_dir() {
        fs::remove_dir_all(&native)
    } else if link_to_directory {
        fs::remove_dir(&native)
    } else {
        fs::remove_file(&native)
    };
    removed.map_err(failed)?;
    done()
}

fn exists_or_failed(error: io::Error) -> FileMutationResult {
    if error.kind() == io::ErrorKind::AlreadyExists {
        FileMutationResult::Invalid("already exists".into())
    } else {
        failed(error)
    }
}

/// A path these requests may name: plain names, none of them `.git` in any case, and a
/// last name the Server's platform can hold as an ordinary entry.
fn check_path(path: &RelativePath) -> Result<(), FileMutationResult> {
    let invalid = |reason: &str| Err(FileMutationResult::Invalid(reason.into()));
    if !valid_diff_path(path) {
        return invalid("invalid path");
    }
    if path
        .components()
        .any(|component| component.as_str().eq_ignore_ascii_case(".git"))
    {
        return invalid("Condr does not change .git");
    }
    let name = path.file_name().unwrap_or_default();
    if name.contains('\0') {
        return invalid("a name cannot hold NUL");
    }
    if cfg!(windows) && !valid_windows_name(name) {
        return invalid("Windows does not allow this name");
    }
    Ok(())
}

/// No `:`, which names an alternate data stream, no trailing dot or space, which Windows
/// strips, and no reserved device name, with or without an extension.
fn valid_windows_name(name: &str) -> bool {
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem = name.split('.').next().unwrap_or_default().trim_end();
    !name.contains([':', '<', '>', '"', '|', '?', '*'])
        && !name.ends_with(['.', ' '])
        && !RESERVED
            .iter()
            .any(|reserved| stem.eq_ignore_ascii_case(reserved))
}

/// Whether `a` and `b` name one directory entry.
#[cfg(unix)]
fn same_entry(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::symlink_metadata(a), fs::symlink_metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// Whether `a` and `b` name one directory entry: Windows names each as stored on disk.
// ponytail: follows symlinks, so two links to one target count as one; std has no stable
// file index to compare instead.
#[cfg(windows)]
fn same_entry(a: &Path, b: &Path) -> bool {
    matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

#[cfg(test)]
mod tests {
    use super::valid_windows_name;

    #[test]
    fn windows_names_refuse_streams_trailing_dots_and_devices() {
        for name in [
            "a:b", "notes.", "notes ", "CON", "con.txt", "Com1.log", "a?b",
        ] {
            assert!(!valid_windows_name(name), "{name}");
        }
        for name in ["notes.md", "console", "COM10", ".env", "CONFIG"] {
            assert!(valid_windows_name(name), "{name}");
        }
    }
}
