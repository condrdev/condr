//! The caller's side of Condr Cloud's authentication (ADR 0047): a compact JWS the Device
//! signs with its own key, sent as `Authorization: Condr <token>` by the Server and the
//! Companion alike. Condr Cloud issues nothing; the token's `kid` is the caller's Device
//! id, which is its public key.

use crate::DeviceKey;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// Condr Cloud's base URL, which every token names as its `aud`.
pub const DEFAULT_URL: &str = "https://push.condr.dev";

/// How long a token holds after its `iat`; Condr Cloud refuses one that holds longer.
const LIFETIME_SECS: u64 = 300;

/// The base URL on a desktop: `CONDR_CLOUD_URL` when set, for development and tests, else
/// [`DEFAULT_URL`], never with a trailing slash. The phone reads no environment and takes
/// [`DEFAULT_URL`].
pub fn base_url() -> String {
    std::env::var("CONDR_CLOUD_URL")
        .ok()
        .map(|url| url.trim_end_matches('/').to_owned())
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| DEFAULT_URL.to_owned())
}

/// A token for `key` to call the Condr Cloud at `audience`, its base URL, issued at
/// `issued_at` in Unix seconds and good for five minutes. It is a bearer credential for
/// those minutes: never log it.
pub fn token(key: &DeviceKey, audience: &str, issued_at: u64) -> String {
    let header = serde_json::json!({
        "alg": "EdDSA",
        "typ": "JWT",
        "kid": key.public().to_string(),
    });
    let claims = serde_json::json!({
        "aud": audience,
        "iat": issued_at,
        "exp": issued_at + LIFETIME_SECS,
    });
    let signed = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    // Ed25519 over the Device key's seed, as its Peer-to-peer identity signs.
    let signature = iroh::SecretKey::from_bytes(key.seed()).sign(signed.as_bytes());
    format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vector Condr Cloud's tests keep as a fixture: Ed25519 is deterministic, so this
    /// seed, audience and time always sign to this token.
    #[test]
    fn a_token_is_a_compact_jws_signed_by_the_device_key() {
        let key = DeviceKey::from_seed([7; 32]);
        let token = token(&key, DEFAULT_URL, 1_700_000_000);
        assert_eq!(
            token,
            concat!(
                "eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCIsImtpZCI6IjZrcHNZLUtjVWdxLTlWQjdFeTdGLVpWSGRxNi",
                "12bnVTUWg3cWFSUkcwaXcifQ.",
                "eyJhdWQiOiJodHRwczovL3B1c2guY29uZHIuZGV2IiwiaWF0IjoxNzAwMDAwMDAwLCJleHAiOjE3MDAwMD",
                "AzMDB9.",
                "I2YGod04q4zzK0_9voNgsJ0Hod4RAFlEp0WCwo-iGi9GZPE2KnyPX3qxwwNgqw2otY6NkiqM9BdTQ9Z1U8",
                "5zDQ",
            )
        );

        let parts: Vec<&str> = token.split('.').collect();
        let [header, claims, signature] = parts[..] else {
            panic!("three parts: {token}");
        };
        let json = |part: &str| -> serde_json::Value {
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap()
        };
        assert_eq!(
            json(header),
            serde_json::json!({"alg": "EdDSA", "typ": "JWT", "kid": key.public().to_string()})
        );
        assert_eq!(
            json(claims),
            serde_json::json!({
                "aud": "https://push.condr.dev",
                "iat": 1_700_000_000,
                "exp": 1_700_000_300,
            })
        );
        let signature: [u8; 64] = URL_SAFE_NO_PAD
            .decode(signature)
            .unwrap()
            .try_into()
            .unwrap();
        let public = iroh::PublicKey::from_bytes(key.public().as_bytes()).unwrap();
        let signed = &token[..token.rfind('.').unwrap()];
        public
            .verify(signed.as_bytes(), &iroh::Signature::from_bytes(&signature))
            .unwrap();
        assert!(
            public
                .verify(b"tampered", &iroh::Signature::from_bytes(&signature))
                .is_err()
        );
    }
}
