//! The Files sidebar's data (ADR 0018): one directory level of a Workspace root and one
//! file's content, text or an image's bytes (ADR 0037), read by the Server and answered
//! over the protocol. Plain filesystem reads: browsing a Workspace does not need it to be
//! a repository. Also the subdirectories of any absolute path, for choosing a Root
//! Directory on another Device.

use std::path::PathBuf;

use relative_path::RelativePath;
use serde::{Deserialize, Serialize};

use crate::valid_diff_path;

#[cfg(feature = "runtime")]
mod read;

#[cfg(feature = "runtime")]
pub use read::{browse_directory, list_directory, read_file};

/// The largest file the Preview Tab shows; larger ones yield a placeholder like a diff.
pub const MAX_FILE_BYTES: u64 = crate::MAX_DIFF_BYTES;
/// The largest image the Preview Tab shows (ADR 0037): its bytes ride one reply frame,
/// with room left for the reply's own fields.
// ponytail: one frame per image; chunk the reply if larger screenshots need showing.
pub const MAX_IMAGE_BYTES: u64 = (crate::protocol::MAX_FRAME_SIZE - 64 * 1024) as u64;
/// The extensions, lower case, whose files are sent as image bytes rather than probed for
/// text (ADR 0037). SVG is text and reaches the Client as such.
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tif", "tiff",
];
/// The most entries one directory listing carries; the rest are dropped and said so.
pub const MAX_DIRECTORY_ENTRIES: usize = 2000;

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
    /// As asked with a leading `~` expanded, or the home directory for an empty request.
    pub path: PathBuf,
    /// Named by the Server, since a Client cannot split a path of another platform.
    pub parent: Option<PathBuf>,
    /// Directories only.
    pub listing: DirectoryListing,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum FileContent {
    Text {
        text: String,
    },
    /// A file [`is_image`] names, undecoded: the Client decodes it.
    Image {
        bytes: Vec<u8>,
    },
    Binary,
    TooLarge {
        bytes: u64,
    },
}

/// The Workspace root itself, or a directory inside it.
pub fn valid_directory_path(path: &RelativePath) -> bool {
    path.as_str().is_empty() || valid_diff_path(path)
}

/// Whether the Server sends `path`'s bytes as an image, by its extension.
pub fn is_image(path: &RelativePath) -> bool {
    path.extension().is_some_and(|extension| {
        IMAGE_EXTENSIONS
            .iter()
            .any(|image| extension.eq_ignore_ascii_case(image))
    })
}
