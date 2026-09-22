//! Authentication and encryption for TCP endpoints, modelled on WireGuard.
//!
//! Each Device holds one persistent Ed25519 key (ADR 0025); the X25519 static key Noise
//! speaks with is derived from it. A connection is one
//! `Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s` handshake: the Client already knows the Server's
//! key, sends its own static key encrypted in the first message together with the Ed25519
//! key it derives from, and the Server only answers peers it recognises by that Ed25519 key
//! after checking the two agree. An authorized peer uses the all-zero pre-shared key, as an
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

/// A Device's Ed25519 public key; its 43-character base64url form is the fingerprint shown
/// to people and carried in `tcp://` and `p2p://` links, and the same 32 bytes are the
/// Device's Peer-to-peer endpoint id (ADR 0025). The identity files keep hex.
#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub struct PublicKey([u8; 32]);

/// A Device's one key: an Ed25519 seed, from which both the Ed25519 public key and the
/// X25519 key Noise speaks with are derived. Debug output never shows the seed.
#[derive(Clone, Eq, PartialEq)]
pub struct DeviceKey {
    seed: [u8; 32],
    /// The clamped SHA-512 prefix of the seed: Ed25519's own scalar, and the X25519
    /// private key libsodium's `crypto_sign_ed25519_sk_to_curve25519` returns.
    noise_private: [u8; 32],
    public: PublicKey,
}

/// A 32-byte pre-shared secret: an invite. Debug output never shows it.
#[derive(Clone, Eq, PartialEq)]
pub struct Secret([u8; 32]);

impl PublicKey {
    /// Parses the fingerprint form, as a person pastes it.
    pub fn parse(text: &str) -> io::Result<Self> {
        decode(text).map(Self)
    }

    /// Parses the hex form the identity files store.
    pub fn parse_hex(hex: &str) -> io::Result<Self> {
        hex_decode(hex).map(Self)
    }

    pub fn to_hex(&self) -> String {
        hex_encode(&self.0)
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The X25519 public key this Ed25519 key corresponds to, which is what a Noise peer
    /// presents; `None` when the bytes are not a valid Edwards point.
    pub fn montgomery(&self) -> Option<[u8; 32]> {
        curve25519_dalek::edwards::CompressedEdwardsY(self.0)
            .decompress()
            .map(|point| point.to_montgomery().to_bytes())
    }

    /// True when `prefix` starts this key's fingerprint, so people can name a key by its
    /// first characters as Git does with commits.
    pub fn matches_prefix(&self, prefix: &str) -> bool {
        !prefix.is_empty() && self.to_string().starts_with(prefix)
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({self})")
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&encode(&self.0))
    }
}

impl DeviceKey {
    pub fn generate() -> io::Result<Self> {
        random().map(Self::from_seed)
    }

    pub fn from_seed(seed: [u8; 32]) -> Self {
        use sha2::Digest as _;
        let hash = sha2::Sha512::digest(seed);
        let mut noise_private = [0; 32];
        noise_private.copy_from_slice(&hash[..32]);
        noise_private[0] &= 248;
        noise_private[31] &= 127;
        noise_private[31] |= 64;
        let public = curve25519_dalek::EdwardsPoint::mul_base_clamped(noise_private)
            .compress()
            .to_bytes();
        Self {
            seed,
            noise_private,
            public: PublicKey(public),
        }
    }

    pub fn public(&self) -> PublicKey {
        self.public
    }

    /// The X25519 private key Noise uses; its public half is `self.public().montgomery()`.
    pub(crate) fn noise_private(&self) -> &[u8; 32] {
        &self.noise_private
    }

    /// The Ed25519 seed, which is also this Device's Peer-to-peer secret key.
    pub(crate) fn seed(&self) -> &[u8; 32] {
        &self.seed
    }

    /// Reads the hex seed at `path`, or generates one and stores it owner-only.
    /// Two processes creating the same key at once both end up with the one that won.
    pub fn load_or_create(path: &Path) -> io::Result<Self> {
        if let Some(text) = read_secret_file(path)? {
            return hex_decode(text.trim()).map(Self::from_seed);
        }
        let key = Self::generate()?;
        match write_secret_file(path, &hex_encode(&key.seed)) {
            Ok(()) => Ok(key),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let text = read_secret_file(path)?.ok_or(error)?;
                hex_decode(text.trim()).map(Self::from_seed)
            }
            Err(error) => Err(error),
        }
    }
}

impl fmt::Debug for DeviceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceKey({}, seed redacted)", self.public)
    }
}

impl Secret {
    pub fn generate() -> io::Result<Self> {
        random().map(Self)
    }

    /// Parses the link form, as a person pastes it.
    pub fn parse(text: &str) -> io::Result<Self> {
        decode(text).map(Self)
    }

    /// The link form; deliberately not `Display`, so a secret never reaches a log by accident.
    pub fn encode(&self) -> String {
        encode(&self.0)
    }

    /// Parses the hex form `pending-invite` stores.
    pub fn parse_hex(hex: &str) -> io::Result<Self> {
        hex_decode(hex).map(Self)
    }

    pub fn to_hex(&self) -> String {
        hex_encode(&self.0)
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
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

/// Unpadded base64url: 43 characters for 32 bytes, against 64 in hex, and no `.`, `@`
/// or `/`, so it sits inside a link untouched.
fn encode(bytes: &[u8; 32]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn decode(text: &str) -> io::Result<[u8; 32]> {
    use base64::Engine as _;
    let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "expected a 43-character key");
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|_| invalid())?
        .try_into()
        .map_err(|_| invalid())
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
    fn keys_and_secrets_round_trip_through_text_and_files() {
        let key = DeviceKey::generate().unwrap();
        let fingerprint = key.public().to_string();
        assert_eq!(fingerprint.len(), 43);
        assert_eq!(PublicKey::parse(&fingerprint).unwrap(), key.public());
        assert_eq!(
            PublicKey::parse_hex(&key.public().to_hex()).unwrap(),
            key.public()
        );
        assert!(PublicKey::parse("abc").is_err());
        assert!(PublicKey::parse(&key.public().to_hex()).is_err());
        assert!(key.public().matches_prefix(&fingerprint[..6]));
        assert!(!key.public().matches_prefix(""));
        let secret = Secret::generate().unwrap();
        assert_eq!(Secret::parse(&secret.encode()).unwrap(), secret);
        assert_eq!(Secret::parse_hex(&secret.to_hex()).unwrap(), secret);

        let path =
            std::env::temp_dir().join(format!("condr-noise-key-{}-{}", std::process::id(), now()));
        let stored = DeviceKey::load_or_create(&path).unwrap();
        assert_eq!(DeviceKey::load_or_create(&path).unwrap(), stored);
        assert!(!format!("{stored:?}").contains(&hex_encode(&stored.seed)));
        let _ = fs::remove_file(path);
    }

    /// RFC 8032 test vector 1: the public key iroh will derive from the same seed, and the
    /// X25519 key a Noise peer sees must be that key's Montgomery form.
    #[test]
    fn device_key_derives_the_ed25519_public_key_and_its_x25519_form() {
        let seed =
            hex_decode("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60").unwrap();
        let key = DeviceKey::from_seed(seed);
        assert_eq!(
            key.public().to_hex(),
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
        );
        let noise_public =
            curve25519_dalek::MontgomeryPoint::mul_base_clamped(*key.noise_private()).to_bytes();
        assert_eq!(key.public().montgomery(), Some(noise_public));
    }
}
