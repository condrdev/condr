//! The Companion's one library (ADR 0039): every paired Device's connection, Session model
//! and Agents, behind a uniffi interface Swift and Kotlin share. The app implements
//! [`Listener`]; on `changed` it hops to its main thread, calls
//! [`Companion::take_changes`] and reads what it shows from the getters.

uniffi::setup_scaffolding!();

mod companion;
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

fn device_key(seed: &[u8]) -> Result<DeviceKey, MobileError> {
    let seed: [u8; 32] = seed
        .try_into()
        .map_err(|_| MobileError::Invalid("a Device key seed is 32 bytes".into()))?;
    Ok(DeviceKey::from_seed(seed))
}
