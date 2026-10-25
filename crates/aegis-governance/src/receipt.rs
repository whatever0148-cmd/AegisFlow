//! Ed25519 receipts over the SHA-256 digest of canonical JSON.
//!
//! Canonical JSON = object keys sorted at every level, no insignificant whitespace, scalars
//! serialized by `serde_json`. The signature covers the 32-byte digest and is encoded as
//! `c2pa:ed25519:<base64url, no padding>`.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand_core::{OsRng, RngCore};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const SIGNATURE_PREFIX: &str = "c2pa:ed25519:";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReceiptError {
    #[error("signature must start with `{SIGNATURE_PREFIX}`")]
    Prefix,
    #[error("signature is not 64 bytes of base64url")]
    Encoding,
    #[error("public key is not 32 bytes of hex")]
    PublicKey,
    #[error("payload digest does not match the receipt")]
    Digest,
    #[error("signature does not verify for this public key")]
    Signature,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Receipt {
    /// Hex SHA-256 of the canonical payload.
    pub payload_sha256: String,
    /// `c2pa:ed25519:<base64url signature over the digest>`.
    pub signature: String,
    /// Hex Ed25519 verifying key.
    pub public_key: String,
}

pub struct ReceiptSigner {
    key: SigningKey,
}

impl ReceiptSigner {
    pub fn generate() -> Self {
        Self { key: SigningKey::generate(&mut OsRng) }
    }

    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self { key: SigningKey::from_bytes(&seed) }
    }

    /// Loads a 32-byte seed from `path`, creating it (mode 0600) on first use.
    pub fn load_or_create(path: &Path) -> io::Result<Self> {
        if let Ok(bytes) = fs::read(path) {
            let seed: [u8; 32] = bytes
                .try_into()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "signing seed must be 32 bytes"))?;
            return Ok(Self::from_seed(seed));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
        file.write_all(&seed)?;
        Ok(Self::from_seed(seed))
    }

    pub fn public_key_hex(&self) -> String {
        to_hex(self.key.verifying_key().as_bytes())
    }

    pub fn sign_bytes(&self, payload: &[u8]) -> Receipt {
        let digest = Sha256::digest(payload);
        let signature = self.key.sign(&digest);
        Receipt {
            payload_sha256: to_hex(&digest),
            signature: format!("{SIGNATURE_PREFIX}{}", URL_SAFE_NO_PAD.encode(signature.to_bytes())),
            public_key: self.public_key_hex(),
        }
    }

    pub fn sign_json(&self, payload: &Value) -> Receipt {
        self.sign_bytes(canonical_json(payload).as_bytes())
    }
}

/// Deterministic JSON encoding with sorted object keys, independent of map ordering features.
pub fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).expect("a JSON string key always serializes"));
                out.push(':');
                write_canonical(&map[key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        scalar => out.push_str(&serde_json::to_string(scalar).expect("a JSON scalar always serializes")),
    }
}

pub fn verify_bytes(receipt: &Receipt, payload: &[u8]) -> Result<(), ReceiptError> {
    let digest = Sha256::digest(payload);
    if to_hex(&digest) != receipt.payload_sha256 {
        return Err(ReceiptError::Digest);
    }
    let encoded = receipt.signature.strip_prefix(SIGNATURE_PREFIX).ok_or(ReceiptError::Prefix)?;
    let raw: [u8; 64] = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| ReceiptError::Encoding)?
        .try_into()
        .map_err(|_| ReceiptError::Encoding)?;
    let key: [u8; 32] = from_hex(&receipt.public_key)
        .and_then(|k| k.try_into().ok())
        .ok_or(ReceiptError::PublicKey)?;
    let key = VerifyingKey::from_bytes(&key).map_err(|_| ReceiptError::PublicKey)?;
    key.verify_strict(&digest, &Signature::from_bytes(&raw)).map_err(|_| ReceiptError::Signature)
}

pub fn verify_json(receipt: &Receipt, payload: &Value) -> Result<(), ReceiptError> {
    verify_bytes(receipt, canonical_json(payload).as_bytes())
}

/// Hex SHA-256 of the canonical JSON of `payload`: the `payload_sha256` of a receipt over it, so a
/// verifier holding only the detached signature and the public key can rebuild the [`Receipt`].
pub fn digest_json(payload: &Value) -> String {
    to_hex(&Sha256::digest(canonical_json(payload).as_bytes()))
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sign_and_verify_round_trip() {
        let signer = ReceiptSigner::generate();
        let payload = json!({"amount": {"value": "42.00", "currency_code": "USD"}, "ref": "ord-1"});
        let receipt = signer.sign_json(&payload);
        assert!(receipt.signature.starts_with(SIGNATURE_PREFIX));
        assert!(receipt.signature.len() <= 127, "must fit PayPal custom_id");
        assert_eq!(verify_json(&receipt, &payload), Ok(()));
    }

    #[test]
    fn tampered_payload_fails() {
        let signer = ReceiptSigner::generate();
        let receipt = signer.sign_json(&json!({"value": "42.00"}));
        assert_eq!(verify_json(&receipt, &json!({"value": "4200.00"})), Err(ReceiptError::Digest));
    }

    #[test]
    fn digest_json_matches_the_receipt_digest() {
        let payload = json!({"b": [1, 2], "a": "x"});
        let receipt = ReceiptSigner::generate().sign_json(&payload);
        assert_eq!(digest_json(&payload), receipt.payload_sha256);
        assert_ne!(digest_json(&json!({"a": "y"})), receipt.payload_sha256);
    }

    #[test]
    fn foreign_key_fails() {
        let payload = json!({"value": "1.00"});
        let mut receipt = ReceiptSigner::generate().sign_json(&payload);
        receipt.public_key = ReceiptSigner::generate().public_key_hex();
        assert_eq!(verify_json(&receipt, &payload), Err(ReceiptError::Signature));
    }

    #[test]
    fn canonical_json_ignores_key_order() {
        let a: Value = serde_json::from_str(r#"{"b":1,"a":{"y":[1,{"q":2,"p":3}],"x":null}}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"a":{"x":null,"y":[1,{"p":3,"q":2}]},"b":1}"#).unwrap();
        assert_eq!(canonical_json(&a), canonical_json(&b));
        assert_eq!(canonical_json(&a), r#"{"a":{"x":null,"y":[1,{"p":3,"q":2}]},"b":1}"#);
    }

    #[test]
    fn seed_file_is_private_and_stable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../target/test-fixtures/keys-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("receipt.seed");
        let first = ReceiptSigner::load_or_create(&path).unwrap();
        let second = ReceiptSigner::load_or_create(&path).unwrap();
        assert_eq!(first.public_key_hex(), second.public_key_hex());
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        fs::remove_dir_all(&dir).unwrap();
    }
}
