//! Minimal crypto helpers for globalsend
//!
//! Implements device key generation, X25519 ECDH, HKDF key derivation and
//! ChaCha20-Poly1305 AEAD wrappers. This is intentionally small and meant as a
//! starting point for the real `globalsend-crypto` crate.

use chacha20poly1305::{aead, XChaCha20Poly1305, Key, XNonce};
use hkdf::Hkdf;
use rand_core::OsRng;
use x25519_dalek::{StaticSecret, PublicKey as XPublicKey};
use zeroize::Zeroize;

pub const AEAD_KEY_LEN: usize = 32;
pub const AEAD_NONCE_LEN: usize = 24; // XChaCha20 nonce

#[derive(Debug)]
pub struct DeviceKey {
    /// X25519 static secret used for ECDH (kept encrypted at rest)
    secret: StaticSecret,
}

impl DeviceKey {
    /// Generate a new device X25519 keypair
    pub fn generate() -> Self {
        let secret = StaticSecret::new(OsRng);
        Self { secret }
    }

    /// Public key corresponding to this device key
    pub fn public(&self) -> XPublicKey {
        XPublicKey::from(&self.secret)
    }

    /// Compute an ECDH shared secret with peer public key
    pub fn ecdh(&self, peer: &XPublicKey) -> [u8; 32] {
        let shared = self.secret.diffie_hellman(peer);
        shared.to_bytes()
    }
}

impl Drop for DeviceKey {
    fn drop(&mut self) {
        // StaticSecret implements zeroize on drop through inner representation
    }
}

/// Derive AEAD key and base nonce using HKDF-SHA256 from a shared secret
pub fn derive_aead(shared_secret: &[u8]) -> (Key, [u8; AEAD_NONCE_LEN]) {
    // info labels
    let hk = Hkdf::<sha2::Sha256>::new(None, shared_secret);
    let mut okm = [0u8; AEAD_KEY_LEN + AEAD_NONCE_LEN];
    hk.expand(b"globalsend v1", &mut okm).expect("hkdf expand");
    let key = Key::from_slice(&okm[..AEAD_KEY_LEN]);
    let mut nonce = [0u8; AEAD_NONCE_LEN];
    nonce.copy_from_slice(&okm[AEAD_KEY_LEN..]);
    (key.clone(), nonce)
}

/// AEAD encrypt helper using XChaCha20-Poly1305
pub fn aead_encrypt(key: &Key, base_nonce: &[u8; AEAD_NONCE_LEN], counter: u64, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, aead::Error> {
    let cipher = XChaCha20Poly1305::new(key);
    // Derive per-message nonce by xoring the base nonce with counter (simple construction)
    let mut nonce_bytes = [0u8; AEAD_NONCE_LEN];
    nonce_bytes.copy_from_slice(base_nonce);
    // XOR counter into the last 8 bytes
    let ctr_bytes = counter.to_be_bytes();
    for i in 0..8 {
        let idx = AEAD_NONCE_LEN - 8 + i;
        nonce_bytes[idx] ^= ctr_bytes[i];
    }
    let nonce = XNonce::from_slice(&nonce_bytes);
    cipher.encrypt(nonce, aead::Payload { msg: plaintext, aad })
}

/// AEAD decrypt helper using XChaCha20-Poly1305
pub fn aead_decrypt(key: &Key, base_nonce: &[u8; AEAD_NONCE_LEN], counter: u64, aad: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, aead::Error> {
    let cipher = XChaCha20Poly1305::new(key);
    let mut nonce_bytes = [0u8; AEAD_NONCE_LEN];
    nonce_bytes.copy_from_slice(base_nonce);
    let ctr_bytes = counter.to_be_bytes();
    for i in 0..8 {
        let idx = AEAD_NONCE_LEN - 8 + i;
        nonce_bytes[idx] ^= ctr_bytes[i];
    }
    let nonce = XNonce::from_slice(&nonce_bytes);
    cipher.decrypt(nonce, aead::Payload { msg: ciphertext, aad })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex::ToHex;

    #[test]
    fn ecdh_derive_encrypt_roundtrip() {
        // Alice
        let a = DeviceKey::generate();
        // Bob
        let b = DeviceKey::generate();

        let shared_a = a.ecdh(&b.public());
        let shared_b = b.ecdh(&a.public());
        assert_eq!(shared_a, shared_b);

        let (key, base_nonce) = derive_aead(&shared_a);
        let aad = b"meta";
        let msg = b"hello world from globalsend";
        let ct = aead_encrypt(&key, &base_nonce, 1, aad, msg).expect("encrypt");
        let pt = aead_decrypt(&key, &base_nonce, 1, aad, &ct).expect("decrypt");
        assert_eq!(pt, msg);
    }
}
