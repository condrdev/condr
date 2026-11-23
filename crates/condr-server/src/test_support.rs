//! Helpers shared by the crate's unit tests. Integration tests have their own copy in
//! `tests/common`, since `#[cfg(test)]` items are invisible to them.

/// A short random suffix for per-test temporary paths.
///
/// Random rather than clock-based: the Windows system clock ticks coarsely, so parallel
/// tests used to share an endpoint path and talk to each other's Server. Eight hex digits
/// rather than the whole UUID: `sun_path` holds 104 bytes on macOS, and the temporary
/// directory there already takes half of them.
pub(crate) fn unique_suffix() -> String {
    // The low bits: a UUIDv7 leads with the time, so only these are random.
    format!("{:08x}", uuid::Uuid::now_v7().as_u128() as u32)
}

/// An empty directory any test may make a Workspace's Root. The system's temp directory
/// itself would do, but a Server looks for a repository in every directory right under a
/// Root (ADR 0073), and one full of other programs' leftovers made that slow enough for
/// tests to time out. Nothing writes into it, so every test and every run shares it.
pub(crate) fn empty_root() -> std::path::PathBuf {
    let root = std::env::temp_dir().join("condr-empty-root");
    std::fs::create_dir_all(&root).unwrap();
    root
}
