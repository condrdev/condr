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
