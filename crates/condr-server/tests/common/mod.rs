//! Helpers shared by the integration tests; each test file opts in with `mod common;`.

/// A short random suffix for per-test temporary paths.
///
/// Random rather than clock-based: the Windows system clock ticks coarsely, so parallel
/// tests used to share an endpoint path and talk to each other's Server. Eight hex digits
/// rather than the whole UUID: `sun_path` holds 104 bytes on macOS, and the temporary
/// directory there already takes half of them.
pub fn unique_suffix() -> String {
    format!("{:08x}", uuid::Uuid::new_v4().as_u128() as u32)
}
