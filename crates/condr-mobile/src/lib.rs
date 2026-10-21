//! The Companion's one library (ADR 0039): every paired Device's connection, Session model
//! and Agents, behind a uniffi interface Swift and Kotlin share. The app implements
//! [`Listener`]; on `changed` it hops to its main thread, calls
//! [`Companion::take_changes`] and reads what it shows from the getters.

uniffi::setup_scaffolding!();

mod companion;
mod input;
mod views;

pub use companion::Companion;
pub use views::{
    AgentStatus, AgentView, Change, DeviceStatus, DeviceView, Event, Key, NeedsYouItem, PaneView,
    SavedDevice, ScreenCell, ScreenCursor, TerminalScreen, WorkspaceView,
};

use condr_client::DeviceKey;
use std::fmt;

/// What the app implements to hear that something changed. It is called from one
/// background thread, once per [`Companion::take_changes`], and never while the library
/// holds its state; the app hops to its main thread before reading.
#[uniffi::export(foreign)]
pub trait Listener: Send + Sync {
    fn changed(&self);
}

#[derive(Debug, uniffi::Error)]
#[uniffi(flat_error)]
pub enum MobileError {
    /// An address, link or key that does not parse.
    Invalid(String),
    Connection(String),
}

impl MobileError {
    fn invalid(error: std::io::Error) -> Self {
        Self::Invalid(error.to_string())
    }
}

impl fmt::Display for MobileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) | Self::Connection(reason) => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for MobileError {}

/// A fresh Device key's seed, which the app keeps in the Keychain or behind the Keystore
/// and hands back to [`Companion::new`] at every start (ADR 0040).
#[uniffi::export]
pub fn generate_seed() -> Result<Vec<u8>, MobileError> {
    DeviceKey::generate()
        .map(|key| key.seed().to_vec())
        .map_err(|error| MobileError::Connection(error.to_string()))
}

/// The fingerprint of the key `seed` holds, as a Device's Paired devices shows it.
#[uniffi::export]
pub fn fingerprint(seed: Vec<u8>) -> Result<String, MobileError> {
    device_key(&seed).map(|key| key.public().to_string())
}

/// Condr Cloud's base URL, which every [`Companion::cloud_token`] names (ADR 0047). The
/// phone always takes the default; only a desktop reads `CONDR_CLOUD_URL`.
#[uniffi::export]
pub fn cloud_url() -> String {
    condr_client::cloud::DEFAULT_URL.to_owned()
}

fn device_key(seed: &[u8]) -> Result<DeviceKey, MobileError> {
    let seed: [u8; 32] = seed
        .try_into()
        .map_err(|_| MobileError::Invalid("a Device key seed is 32 bytes".into()))?;
    Ok(DeviceKey::from_seed(seed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use condr_client::PublicKey;

    /// The seed the app hands in as bytes is the key a desktop derives from it: the
    /// same fingerprint, and plain Ed25519, which iroh's identity also is (ADR 0040).
    #[test]
    fn a_seed_handed_in_as_bytes_is_the_key_a_desktop_derives() {
        let desktop = DeviceKey::generate().unwrap();
        assert_eq!(
            fingerprint(desktop.seed().to_vec()).unwrap(),
            desktop.public().to_string()
        );

        // RFC 8032, section 7.1, test 1.
        let byte = |pair: &[u8]| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16);
        let bytes = |text: &str| -> [u8; 32] {
            let bytes: Vec<u8> = text
                .as_bytes()
                .chunks(2)
                .map(|p| byte(p).unwrap())
                .collect();
            bytes.try_into().unwrap()
        };
        let seed = bytes("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
        let public = bytes("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
        assert_eq!(
            fingerprint(seed.to_vec()).unwrap(),
            PublicKey::from_bytes(public).to_string()
        );

        assert!(matches!(
            fingerprint(vec![0; 31]),
            Err(MobileError::Invalid(_))
        ));
    }
}
