//! Client-side, zero-knowledge encryption.
//!
//! Envelope scheme: a random 256-bit **data encryption key (DEK)** actually
//! encrypts save data. The DEK is wrapped twice — once by a key derived from the
//! user's passphrase (Argon2id), and once by a randomly-generated **recovery
//! key**. Either can unlock the DEK, so a forgotten passphrase is recoverable
//! but a lost keystore + lost recovery key is not (true zero-knowledge).
//!
//! Object encryption uses XChaCha20-Poly1305 (AEAD, 192-bit nonces — safe to
//! generate randomly per object). The DEK never leaves the device in the clear.

use argon2::Argon2;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::util::{from_hex, to_hex};

pub const KEY_LEN: usize = 32;
pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 24; // XChaCha20-Poly1305

/// Fill a buffer with cryptographically secure random bytes.
pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    getrandom::getrandom(&mut v).expect("system RNG must be available");
    v
}

/// The unwrapped data encryption key, held only in memory while unlocked.
#[derive(Clone)]
pub struct Dek([u8; KEY_LEN]);

impl Dek {
    /// Build the AEAD cipher used for object encryption.
    pub fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new_from_slice(&self.0).expect("32-byte key")
    }
}

fn derive_kek(passphrase: &[u8], salt: &[u8]) -> Result<[u8; KEY_LEN]> {
    let mut out = [0u8; KEY_LEN];
    Argon2::default()
        .hash_password_into(passphrase, salt, &mut out)
        .map_err(|e| Error::other(format!("argon2 kdf failed: {e}")))?;
    Ok(out)
}

/// Wrap (encrypt) the DEK with a key-encryption key. Output is `nonce || ct`.
fn wrap(kek: &[u8; KEY_LEN], dek: &[u8; KEY_LEN]) -> Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new_from_slice(kek).expect("32-byte key");
    let nonce = random_bytes(NONCE_LEN);
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce), dek.as_ref())
        .map_err(|_| Error::other("failed to wrap key"))?;
    let mut out = nonce;
    out.extend_from_slice(&ct);
    Ok(out)
}

fn unwrap(kek: &[u8; KEY_LEN], blob: &[u8]) -> Result<Dek> {
    if blob.len() <= NONCE_LEN {
        return Err(Error::Integrity("wrapped key too short".into()));
    }
    let (nonce, ct) = blob.split_at(NONCE_LEN);
    let cipher = XChaCha20Poly1305::new_from_slice(kek).expect("32-byte key");
    let pt = cipher
        .decrypt(XNonce::from_slice(nonce), ct)
        .map_err(|_| Error::other("wrong passphrase or recovery key"))?;
    if pt.len() != KEY_LEN {
        return Err(Error::Integrity("unwrapped key has wrong length".into()));
    }
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&pt);
    Ok(Dek(key))
}

/// The on-disk keystore. Contains only ciphertext + a salt — no secret is
/// recoverable from it without the passphrase or recovery key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyStore {
    pub version: u32,
    pub kdf: String,
    /// hex
    pub pass_salt: String,
    /// hex of `nonce || wrapped_dek`
    pub pass_wrapped_dek: String,
    /// hex of `nonce || wrapped_dek`
    pub recovery_wrapped_dek: String,
}

/// A freshly generated recovery key, shown to the user exactly once.
pub struct RecoveryKey(pub String);

impl RecoveryKey {
    /// Group the hex into readable blocks for display/printing.
    pub fn grouped(&self) -> String {
        self.0
            .as_bytes()
            .chunks(4)
            .map(|c| std::str::from_utf8(c).unwrap_or(""))
            .collect::<Vec<_>>()
            .join("-")
    }
}

impl KeyStore {
    /// Create a new keystore protecting a fresh random DEK. Returns the store to
    /// persist and the recovery key to show the user.
    pub fn init(passphrase: &str) -> Result<(KeyStore, RecoveryKey)> {
        let dek_bytes: [u8; KEY_LEN] = random_bytes(KEY_LEN).try_into().unwrap();
        let pass_salt = random_bytes(SALT_LEN);
        let pass_kek = derive_kek(passphrase.as_bytes(), &pass_salt)?;
        let pass_wrapped = wrap(&pass_kek, &dek_bytes)?;

        // The recovery key is itself a high-entropy 256-bit key, used directly
        // as a KEK (no KDF needed — it isn't a low-entropy human secret).
        let recovery_bytes: [u8; KEY_LEN] = random_bytes(KEY_LEN).try_into().unwrap();
        let recovery_wrapped = wrap(&recovery_bytes, &dek_bytes)?;

        let store = KeyStore {
            version: 1,
            kdf: "argon2id".to_string(),
            pass_salt: to_hex(&pass_salt),
            pass_wrapped_dek: to_hex(&pass_wrapped),
            recovery_wrapped_dek: to_hex(&recovery_wrapped),
        };
        Ok((store, RecoveryKey(to_hex(&recovery_bytes))))
    }

    pub fn unlock_with_passphrase(&self, passphrase: &str) -> Result<Dek> {
        let salt =
            from_hex(&self.pass_salt).ok_or_else(|| Error::other("corrupt keystore salt"))?;
        let kek = derive_kek(passphrase.as_bytes(), &salt)?;
        let blob =
            from_hex(&self.pass_wrapped_dek).ok_or_else(|| Error::other("corrupt keystore"))?;
        unwrap(&kek, &blob)
    }

    pub fn unlock_with_recovery(&self, recovery_hex: &str) -> Result<Dek> {
        let key = from_hex(recovery_hex).ok_or_else(|| Error::other("malformed recovery key"))?;
        let kek: [u8; KEY_LEN] = key
            .try_into()
            .map_err(|_| Error::other("recovery key must be 32 bytes"))?;
        let blob =
            from_hex(&self.recovery_wrapped_dek).ok_or_else(|| Error::other("corrupt keystore"))?;
        unwrap(&kek, &blob)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passphrase_and_recovery_both_unlock_same_dek() {
        let (store, recovery) = KeyStore::init("correct horse battery staple").unwrap();

        let dek_pass = store
            .unlock_with_passphrase("correct horse battery staple")
            .unwrap();
        let dek_rec = store.unlock_with_recovery(&recovery.0).unwrap();
        assert_eq!(dek_pass.0, dek_rec.0, "both paths must yield the same DEK");

        // Round-trip a payload to prove the DEK actually works.
        let cipher = dek_pass.cipher();
        let nonce = random_bytes(NONCE_LEN);
        let ct = cipher
            .encrypt(XNonce::from_slice(&nonce), b"secret save".as_ref())
            .unwrap();
        let pt = dek_rec
            .cipher()
            .decrypt(XNonce::from_slice(&nonce), ct.as_ref())
            .unwrap();
        assert_eq!(pt, b"secret save");
    }

    #[test]
    fn wrong_passphrase_is_rejected() {
        let (store, _) = KeyStore::init("right").unwrap();
        assert!(store.unlock_with_passphrase("wrong").is_err());
    }
}
