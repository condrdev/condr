//! The protobuf wire form (ADR 0028): the types `build.rs` generates from
//! `proto/condr/v1`, and the conversions between them and the domain types. Generated
//! types never leave the crate; everything else speaks domain types.
//!
//! Encoding is `From`/`TryFrom<&Domain> for pb::T` (fallible only where a path must be
//! UTF-8). Decoding is `TryFrom<pb::T> for Domain` with [`WireError`], which tells a value
//! that is wrong on any protocol from one a newer peer sent.

use std::path::{Path, PathBuf};

use relative_path::RelativePathBuf;

#[allow(clippy::all, clippy::pedantic)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/condr.v1.rs"));
}
pub(crate) use generated::*;

mod handshake;
mod messages;
mod session;
mod terminal;

pub(crate) use messages::decode_bootstrap_record;

/// Why a wire value did not become a domain value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WireError {
    /// Wrong on any protocol: a missing field, a bad length, an out-of-range number.
    Malformed(String),
    /// A oneof member, or an enum value without a fallback, from a newer protocol. It
    /// makes the smallest enclosing message that has a rule for it unknown (ADR 0028).
    Unknown,
}

pub(crate) type WireResult<T> = Result<T, WireError>;

impl std::fmt::Display for WireError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(reason) => formatter.write_str(reason),
            Self::Unknown => formatter.write_str("a value from a newer protocol"),
        }
    }
}

pub(crate) fn malformed(reason: impl Into<String>) -> WireError {
    WireError::Malformed(reason.into())
}

/// A message field every message of its kind has always carried.
pub(crate) fn required<T>(value: Option<T>, field: &str) -> WireResult<T> {
    value.ok_or_else(|| malformed(format!("{field} is missing")))
}

/// A oneof: unset is indistinguishable from a member added later, so it is unknown.
pub(crate) fn member<T>(value: Option<T>) -> WireResult<T> {
    value.ok_or(WireError::Unknown)
}

/// An enum without a fallback. `0` is a peer that never set the field; any other value
/// this build does not define is newer.
pub(crate) fn enum_value<E: TryFrom<i32>>(raw: i32, field: &str) -> WireResult<E> {
    if raw == 0 {
        return Err(malformed(format!("{field} is unspecified")));
    }
    E::try_from(raw).map_err(|_| WireError::Unknown)
}

/// An enum whose unknown values decode to `fallback`, as its `.proto` comment says.
pub(crate) fn enum_or<E: TryFrom<i32>>(raw: i32, field: &str, fallback: E) -> WireResult<E> {
    match enum_value(raw, field) {
        Err(WireError::Unknown) => Ok(fallback),
        other => other,
    }
}

/// A protobuf `uint32` holding a narrower Rust integer.
pub(crate) fn narrow<T: TryFrom<u32>>(value: u32, field: &str) -> WireResult<T> {
    T::try_from(value).map_err(|_| malformed(format!("{field} is out of range")))
}

pub(crate) fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("path {} is not valid UTF-8", path.display()))
}

pub(crate) fn optional_path_text(path: Option<&PathBuf>) -> Result<Option<String>, String> {
    path.map(|path| path_text(path)).transpose()
}

pub(crate) fn relative_path(text: String) -> WireResult<RelativePathBuf> {
    if text.starts_with('/') || text.contains('\\') {
        return Err(malformed(format!("{text:?} is not a relative path")));
    }
    Ok(RelativePathBuf::from(text))
}

/// Rejects `count` items of `what` above `limit`. prost has already materialized them;
/// the frame limit bounds that.
pub(crate) fn bounded(count: usize, limit: usize, what: &str) -> WireResult<()> {
    if count > limit {
        return Err(malformed(format!(
            "{count} {what} exceed the limit of {limit}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
