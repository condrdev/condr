//! The Files sidebar's data (ADR 0018): one directory level of a Workspace root and one
//! file's content, read by the Server and answered over the protocol. Plain filesystem
//! reads: browsing a Workspace does not need it to be a repository. Also the subdirectories
//! of any absolute path, for choosing a Root Directory on another Device.

use std::fs;
use std::path::{Path, PathBuf};

use relative_path::RelativePath;
use serde::{Deserialize, Serialize};

use crate::valid_diff_path;

/// The largest file the Preview Tab shows; larger ones yield a placeholder like a diff.
pub const MAX_FILE_BYTES: u64 = crate::MAX_DIFF_BYTES;
/// The most entries one directory listing carries; the rest are dropped and said so.
pub const MAX_DIRECTORY_ENTRIES: usize = 2000;
/// How much of a file decides whether it is text, as git's own heuristic does.
const BINARY_PROBE_BYTES: usize = 8000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum FileKind {
    Directory,
    File,
}

/// One row of a directory listing. Symlinks report what they point at.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub name: String,
    pub kind: FileKind,
    /// Excluded by the repository's ignore rules, so the GUI dims it; always `false`
    /// outside a repository. Set by [`crate::GitRepository::mark_ignored`], not here.
    pub ignored: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DirectoryListing {
    /// Directories first, then files, each group by name ignoring case.
    pub entries: Vec<DirectoryEntry>,
    /// The listing stopped at [`MAX_DIRECTORY_ENTRIES`].
    pub truncated: bool,
}

/// A directory on the Server's machine and its subdirectories, for choosing a Root
/// Directory from a Client that cannot open a native picker there.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BrowsedDirectory {
    /// As asked, or the home directory when the request was empty.
    pub path: PathBuf,
    /// Named by the Server, since a Client cannot split a path of another platform.
    pub parent: Option<PathBuf>,
    /// Directories only.
    pub listing: DirectoryListing,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum FileContent {
    Text { text: String },
    Binary,
    TooLarge { bytes: u64 },
}

/// The Workspace root itself, or a directory inside it.
pub fn valid_directory_path(path: &RelativePath) -> bool {
    path.as_str().is_empty() || valid_diff_path(path)
}

/// `relative` under `root`, one level deep, `.git` left out. `relative` empty lists the root.
pub fn list_directory(root: &Path, relative: &RelativePath) -> Result<DirectoryListing, String> {
    if !valid_directory_path(relative) {
        return Err("invalid directory path".into());
    }
    let directory = relative.to_path(root);
    let read = fs::read_dir(&directory).map_err(|error| error.to_string())?;
    let mut entries = Vec::new();
    let mut truncated = false;
    for entry in read {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        if entries.len() >= MAX_DIRECTORY_ENTRIES {
            truncated = true;
            break;
        }
        // `metadata` follows symlinks, so a link to a directory expands like one.
        let kind = match fs::metadata(entry.path()) {
            Ok(meta) if meta.is_dir() => FileKind::Directory,
            _ => FileKind::File,
        };
        entries.push(DirectoryEntry {
            name,
            kind,
            ignored: false,
        });
    }
    entries.sort_by(|a, b| {
        (a.kind != FileKind::Directory)
            .cmp(&(b.kind != FileKind::Directory))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(DirectoryListing { entries, truncated })
}

/// The subdirectories of the absolute `path`, or of the home directory when it is empty.
pub fn browse_directory(path: &Path) -> Result<BrowsedDirectory, String> {
    let path = if path.as_os_str().is_empty() {
        dirs::home_dir().ok_or("cannot locate the home directory")?
    } else {
        path.to_path_buf()
    };
    if !path.is_absolute() {
        return Err(format!("not an absolute path: {}", path.display()));
    }
    let mut listing = list_directory(&path, RelativePath::new(""))?;
    listing
        .entries
        .retain(|entry| entry.kind == FileKind::Directory);
    Ok(BrowsedDirectory {
        parent: path.parent().map(Path::to_path_buf),
        path,
        listing,
    })
}

/// `relative`'s content under `root`, as text when it looks like text.
pub fn read_file(root: &Path, relative: &RelativePath) -> Result<FileContent, String> {
    if !valid_diff_path(relative) {
        return Err("invalid file path".into());
    }
    let path: PathBuf = relative.to_path(root);
    let meta = fs::metadata(&path).map_err(|error| error.to_string())?;
    if meta.is_dir() {
        return Err("is a directory".into());
    }
    if meta.len() > MAX_FILE_BYTES {
        return Ok(FileContent::TooLarge { bytes: meta.len() });
    }
    let bytes = fs::read(&path).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Ok(FileContent::TooLarge {
            bytes: bytes.len() as u64,
        });
    }
    if bytes[..bytes.len().min(BINARY_PROBE_BYTES)].contains(&0) {
        return Ok(FileContent::Binary);
    }
    Ok(FileContent::Text {
        text: String::from_utf8_lossy(&bytes).into_owned(),
    })
}
