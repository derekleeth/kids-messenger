//! How a device makes sure an unencrypted address really is its family's hub, before sending it anything secret.
//!
//! A home address like `192.168.1.20` can belong to a stranger's machine on someone else's network. The device sends a
//! fresh random nonce, and the hub answers with an HMAC of it keyed by the hash of that device's token. The hub stores
//! that hash and the device can work it out, so neither has to reveal anything to the other.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// `POST /prove`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProveRequest {
    pub device_id: String,
    pub nonce: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubProof {
    pub proof: String,
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// What the hub keeps instead of a device's token.
pub fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// The answer to `nonce` from a hub that knows this device's `token_hash`.
pub fn hub_proof(token_hash: &str, device_id: &str, nonce: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(token_hash.as_bytes()).expect("HMAC takes a key of any length");
    mac.update(b"kitchat-hub-proof\0");
    mac.update(device_id.as_bytes());
    mac.update(b"\0");
    mac.update(nonce.as_bytes());
    hex(&mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proof_depends_on_the_key_the_device_and_the_nonce() {
        let hash = token_hash("token");
        let proof = hub_proof(&hash, "device", "nonce");
        assert_eq!(proof.len(), 64);
        assert_eq!(proof, hub_proof(&hash, "device", "nonce"));
        assert_ne!(proof, hub_proof(&token_hash("other"), "device", "nonce"));
        assert_ne!(proof, hub_proof(&hash, "other", "nonce"));
        assert_ne!(proof, hub_proof(&hash, "device", "other"));
    }

    #[test]
    fn the_token_hash_is_plain_sha256_hex() {
        assert_eq!(token_hash("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
