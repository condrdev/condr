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

/// Why a transfer cannot go on, as the Stream connection says it (ADR 0061).
pub type TransferRefusal = (crate::protocol::StreamError, String);

fn transfer_io(error: io::Error) -> TransferRefusal {
    use crate::protocol::StreamError;
    let kind = match error.kind() {
        io::ErrorKind::NotFound => StreamError::NotFound,
        io::ErrorKind::AlreadyExists => StreamError::AlreadyExists,
        _ => StreamError::Io,
    };
    (kind, error.to_string())
}

/// A download's source: `read_file`'s rules, a regular file reached through symlinks.
pub fn open_download(
    root: &Path,
    relative: &RelativePath,
) -> Result<(fs::File, u64), TransferRefusal> {
    use crate::protocol::StreamError;
    if !valid_diff_path(relative) {
        return Err((StreamError::InvalidPath, "invalid path".into()));
    }
    let path = relative.to_path(root);
    let meta = fs::metadata(&path).map_err(transfer_io)?;
    if !meta.is_file() {
        return Err((StreamError::NotAFile, "only files can be downloaded".into()));
    }
    let file = fs::File::open(&path).map_err(transfer_io)?;
    Ok((file, meta.len()))
}

/// An upload in progress: bytes go to a temporary file beside the target with no lock
/// held, and only [`Upload::commit`] takes the mutation lock, for the final check and the
/// placing. Dropped uncommitted, it removes the temporary file and leaves the target.
pub struct Upload {
    temp: std::path::PathBuf,
    target: std::path::PathBuf,
    file: Option<fs::File>,
    replace: bool,
}

/// Starts an upload of `relative` under `root`, under ADR 0045's write rules. A target
/// that exists is `ALREADY_EXISTS` without `replace`; a symlink is never replaced, since
/// that would turn the link into a file.
pub fn begin_upload(
    root: &Path,
    relative: &RelativePath,
    replace: bool,
) -> Result<Upload, TransferRefusal> {
    use crate::protocol::StreamError;
    check_path(relative).map_err(|refusal| match refusal {
        FileMutationResult::Invalid(reason) => (StreamError::InvalidPath, reason),
        other => (StreamError::Other, format!("{other:?}")),
    })?;
    let target = relative.to_path(root);
    check_upload_target(&target, replace)?;
    let (temp, file) = super::create_transfer_temp(&target, "upload").map_err(transfer_io)?;
    Ok(Upload {
        temp,
        target,
        file: Some(file),
        replace,
    })
}

fn check_upload_target(target: &Path, replace: bool) -> Result<(), TransferRefusal> {
    use crate::protocol::StreamError;
    match fs::symlink_metadata(target) {
        Ok(meta) if meta.file_type().is_symlink() => Err((
            StreamError::NotAFile,
            "a symlink is not replaced by an upload".into(),
        )),
        Ok(meta) if !meta.is_file() => Err((StreamError::NotAFile, "not a regular file".into())),
        Ok(_) if !replace => Err((StreamError::AlreadyExists, "already exists".into())),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(transfer_io(error)),
    }
}

impl Upload {
    pub fn write(&mut self, data: &[u8]) -> io::Result<()> {
        use std::io::Write as _;
        self.file
            .as_mut()
            .expect("an uncommitted upload has its file")
            .write_all(data)
    }

    /// Syncs the temporary file and places it under the target's name.
    pub fn commit(mut self) -> Result<(), TransferRefusal> {
        let file = self.file.take().expect("an upload commits once");
        file.sync_all().map_err(transfer_io)?;
        drop(file);
        let _serialized = MUTATIONS.lock().unwrap_or_else(PoisonError::into_inner);
        check_upload_target(&self.target, self.replace)?;
        super::place_transfer(&self.temp, &self.target, self.replace).map_err(transfer_io)?;
        self.temp = std::path::PathBuf::new();
        Ok(())
    }
}

impl Drop for Upload {
    fn drop(&mut self) {
        if !self.temp.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.temp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::valid_windows_name;
    use relative_path::RelativePath;

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

    fn workspace(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "condr-upload-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// An upload holds no lock while it writes, so Condr's own saves elsewhere go on.
    #[test]
    fn an_unfinished_upload_does_not_hold_up_a_save() {
        let root = workspace("lock");
        std::fs::write(root.join("notes.md"), "old").unwrap();
        let mut upload = super::begin_upload(&root, RelativePath::new("big.bin"), false).unwrap();
        upload.write(b"half").unwrap();
        let saved = super::mutate_file(
            &root,
            &super::FileMutation::Write {
                path: "notes.md".into(),
                text: "new".into(),
                base: None,
            },
        );
        assert!(
            matches!(saved, super::FileMutationResult::Done { .. }),
            "{saved:?}"
        );
        assert!(
            !root.join("big.bin").exists(),
            "nothing under the real name yet"
        );
        upload.commit().unwrap();
        assert_eq!(std::fs::read(root.join("big.bin")).unwrap(), b"half");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_never_replaced_and_a_dropped_upload_leaves_nothing() {
        use crate::protocol::StreamError;
        let root = workspace("link");
        std::fs::write(root.join("real.txt"), "real").unwrap();
        std::os::unix::fs::symlink("real.txt", root.join("link.txt")).unwrap();
        let refused = super::begin_upload(&root, RelativePath::new("link.txt"), true)
            .err()
            .unwrap();
        assert_eq!(refused.0, StreamError::NotAFile);
        let refused = super::begin_upload(&root, RelativePath::new("real.txt"), false)
            .err()
            .unwrap();
        assert_eq!(refused.0, StreamError::AlreadyExists);
        let mut upload = super::begin_upload(&root, RelativePath::new("real.txt"), true).unwrap();
        upload.write(b"partial").unwrap();
        drop(upload);
        assert_eq!(
            std::fs::read_to_string(root.join("real.txt")).unwrap(),
            "real"
        );
        let left: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .filter(|entry| super::super::is_transfer_temp(&entry.file_name().to_string_lossy()))
            .collect();
        assert!(left.is_empty(), "{left:?}");
        std::fs::remove_dir_all(root).unwrap();
    }
}
