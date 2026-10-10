//! This machine's identity on disk (ADR 0011, ADR 0025, ADR 0033): its `device-key`, the
//! Devices it has paired and the pending invite. The key types and the Noise transport
//! are `condr-client`'s; this crate keeps the files and answers for them as an
//! [`Authority`].

mod identity;

use identity::{read_secret_file, write_secret_file};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use condr_client::{Authority, DeviceKey, NoiseStream, PublicKey, Secret};
pub(crate) use identity::LazyIdentity;
pub use identity::{
    AuthorizedClient, INVITE_TTL, Invite, ServerIdentity, create_invite, device_name,
    identity_directory, load_device_key, read_authorized, revoke,
};

/// Reads the hex seed at `path`, or generates one and stores it owner-only.
/// Two processes creating the same key at once both end up with the one that won.
pub(crate) fn load_or_create_key(path: &Path) -> io::Result<DeviceKey> {
    if let Some(text) = read_secret_file(path)? {
        return DeviceKey::from_seed_hex(&text);
    }
    let key = DeviceKey::generate()?;
    match write_secret_file(path, &key.seed_hex()) {
        Ok(()) => Ok(key),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let text = read_secret_file(path)?.ok_or(error)?;
            DeviceKey::from_seed_hex(&text)
        }
        Err(error) => Err(error),
    }
}

fn rejected(reason: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, reason.into())
}

#[cfg(test)]
mod tests {
    use super::identity::now;
    use super::*;

    #[test]
    fn the_device_key_file_is_created_once_and_read_back() {
        let path =
            std::env::temp_dir().join(format!("condr-noise-key-{}-{}", std::process::id(), now()));
        let stored = load_or_create_key(&path).unwrap();
        assert_eq!(load_or_create_key(&path).unwrap(), stored);
        let _ = fs::remove_file(path);
    }
}
