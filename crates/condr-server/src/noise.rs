//! Authentication and encryption for TCP endpoints, modelled on WireGuard.
//!
//! Both sides hold a persistent X25519 static key. A connection is one
//! `Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s` handshake: the Client already knows the Server's
//! public key, sends its own static key encrypted in the first message, and the Server only
//! answers peers it recognises. An authorized peer uses the all-zero pre-shared key, as an
//! unconfigured WireGuard peer does. Pairing a new device uses a one-time, short-lived invite
//! secret as that pre-shared key instead; the Server records the device's public key once the
//! first transport message proves the Client held the secret.
//!
//! After the handshake every byte of the ordinary framed protocol travels inside Noise
//! transport records of at most 64 KiB (`u16` big-endian length, then ciphertext).

mod identity;
mod stream;

use identity::{read_secret_file, write_secret_file};
use snow::{HandshakeState, TransportState};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use identity::{
    AuthorizedClient, INVITE_TTL, Invite, ServerIdentity, create_invite, device_name,
    identity_directory, load_device_key, read_authorized, revoke,
};
pub use stream::NoiseStream;

/// An X25519 public key; its hex form is the fingerprint shown to people.
#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub struct PublicKey([u8; 32]);

/// An X25519 static key pair. Debug output never shows the private half.
#[derive(Clone, Eq, PartialEq)]
pub struct StaticKey {
    private: [u8; 32],
    public: PublicKey,
}

/// A 32-byte pre-shared secret: an invite. Debug output never shows it.
#[derive(Clone, Eq, PartialEq)]
pub struct Secret([u8; 32]);

impl PublicKey {
    pub fn parse(hex: &str) -> io::Result<Self> {
        hex_decode(hex).map(Self)
    }

    pub fn to_hex(&self) -> String {
        hex_encode(&self.0)
    }

    /// True when `prefix` is a hex prefix of this key, so people can name a key by its
    /// first characters as Git does with commits.
    pub fn matches_prefix(&self, prefix: &str) -> bool {
        !prefix.is_empty() && self.to_hex().starts_with(&prefix.to_ascii_lowercase())
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({})", self.to_hex())
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl StaticKey {
    pub fn generate() -> io::Result<Self> {
        random().map(Self::from_private)
    }

    pub fn from_private(private: [u8; 32]) -> Self {
        let public = curve25519_dalek::MontgomeryPoint::mul_base_clamped(private).to_bytes();
        Self {
            private,
            public: PublicKey(public),
        }
    }

    pub fn public(&self) -> PublicKey {
        self.public
    }

    /// Reads the hex private key at `path`, or generates one and stores it owner-only.
    /// Two processes creating the same key at once both end up with the one that won.
    pub fn load_or_create(path: &Path) -> io::Result<Self> {
        if let Some(text) = read_secret_file(path)? {
            return hex_decode(text.trim()).map(Self::from_private);
        }
        let key = Self::generate()?;
        match write_secret_file(path, &hex_encode(&key.private)) {
            Ok(()) => Ok(key),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let text = read_secret_file(path)?.ok_or(error)?;
                hex_decode(text.trim()).map(Self::from_private)
            }
            Err(error) => Err(error),
        }
    }
}

impl fmt::Debug for StaticKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StaticKey({}, private redacted)", self.public)
    }
}

impl Secret {
    pub fn generate() -> io::Result<Self> {
        random().map(Self)
    }

    pub fn parse(hex: &str) -> io::Result<Self> {
        hex_decode(hex).map(Self)
    }

    pub fn to_hex(&self) -> String {
        hex_encode(&self.0)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(redacted)")
    }
}

fn random() -> io::Result<[u8; 32]> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(bytes)
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_decode(hex: &str) -> io::Result<[u8; 32]> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "expected 64 hex characters");
    if hex.len() != 64 || !hex.is_ascii() {
        return Err(invalid());
    }
    let mut bytes = [0; 32];
    for (byte, pair) in bytes.iter_mut().zip(hex.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).map_err(|_| invalid())?, 16)
            .map_err(|_| invalid())?;
    }
    Ok(bytes)
}

fn rejected(reason: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, reason.into())
}

#[cfg(test)]
mod tests {
    use super::identity::now;
    use super::*;
    #[test]
    fn keys_and_secrets_round_trip_through_hex_and_files() {
        let key = StaticKey::generate().unwrap();
        assert_eq!(
            PublicKey::parse(&key.public().to_hex()).unwrap(),
            key.public()
        );
        assert!(PublicKey::parse("abc").is_err());
        assert!(key.public().matches_prefix(&key.public().to_hex()[..6]));
        assert!(!key.public().matches_prefix(""));

        let path =
            std::env::temp_dir().join(format!("condr-noise-key-{}-{}", std::process::id(), now()));
        let stored = StaticKey::load_or_create(&path).unwrap();
        assert_eq!(StaticKey::load_or_create(&path).unwrap(), stored);
        assert!(!format!("{stored:?}").contains(&hex_encode(&stored.private)));
        let _ = fs::remove_file(path);
    }
}
