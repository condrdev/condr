//! The Server's side of the Files sidebar: reading a directory level or a file from disk.

use std::fs;
use std::path::{Path, PathBuf};

use relative_path::RelativePath;

use super::*;

/// How much of a file decides whether it is text, as git's own heuristic does.
const BINARY_PROBE_BYTES: usize = 8000;

/// `relative` under `root`, one level deep, `.git` left out. `relative` empty lists the root.
pub fn list_directory(root: &Path, relative: &RelativePath) -> Result<DirectoryListing, String> {
    if !valid_directory_path(relative) {
        return Err("invalid directory path".into());
    }
    read_listing(&relative.to_path(root), |_| true)
}

/// The subdirectories of the absolute `path`, a leading `~` standing for the home
/// directory, or of the home directory itself when `path` is empty.
pub fn browse_directory(path: &Path) -> Result<BrowsedDirectory, String> {
    let home = || dirs::home_dir().ok_or("cannot locate the home directory");
    let path = if path.as_os_str().is_empty() {
        home()?
    } else if let Ok(rest) = path.strip_prefix("~") {
        home()?.join(rest)
    } else {
        path.to_path_buf()
    };
    if !path.is_absolute() {
        return Err(format!("not an absolute path: {}", path.display()));
    }
    let listing = read_listing(&path, |kind| kind == FileKind::Directory)?;
    Ok(BrowsedDirectory {
        parent: path.parent().map(Path::to_path_buf),
        path,
        listing,
    })
}

/// One level of `directory`, `.git` left out, of the kinds `keep` accepts; the cap counts
/// only those.
fn read_listing(
    directory: &Path,
    keep: impl Fn(FileKind) -> bool,
) -> Result<DirectoryListing, String> {
    let read = fs::read_dir(directory).map_err(|error| error.to_string())?;
    let mut entries = Vec::new();
    let mut truncated = false;
    for entry in read {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        // `metadata` follows symlinks, so a link to a directory expands like one.
        let kind = match fs::metadata(entry.path()) {
            Ok(meta) if meta.is_dir() => FileKind::Directory,
            _ => FileKind::File,
        };
        if !keep(kind) {
            continue;
        }
        if entries.len() >= MAX_DIRECTORY_ENTRIES {
            truncated = true;
            break;
        }
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

/// `relative`'s content under `root`: an image's bytes, else text when it looks like text.
pub fn read_file(root: &Path, relative: &RelativePath) -> Result<FileContent, String> {
    if !valid_diff_path(relative) {
        return Err("invalid file path".into());
    }
    let path: PathBuf = relative.to_path(root);
    let meta = fs::metadata(&path).map_err(|error| error.to_string())?;
    if meta.is_dir() {
        return Err("is a directory".into());
    }
    let image = is_image(relative);
    let limit = if image {
        MAX_IMAGE_BYTES
    } else {
        MAX_FILE_BYTES
    };
    if meta.len() > limit {
        return Ok(FileContent::TooLarge { bytes: meta.len() });
    }
    let bytes = fs::read(&path).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit {
        return Ok(FileContent::TooLarge {
            bytes: bytes.len() as u64,
        });
    }
    if image {
        return Ok(FileContent::Image { bytes });
    }
    if bytes[..bytes.len().min(BINARY_PROBE_BYTES)].contains(&0) {
        return Ok(FileContent::Binary);
    }
    Ok(FileContent::Text {
        text: String::from_utf8_lossy(&bytes).into_owned(),
    })
}
