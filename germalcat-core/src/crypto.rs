use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, Result};
use argon2::Argon2;
use rand::RngCore;

/// A 32-byte AES-256 key derived from the dashboard password.
#[derive(Clone)]
pub struct Key([u8; 32]);

impl Key {
    /// Argon2id(password, salt) -> 32-byte key. Same salt + password => same key.
    pub fn derive(password: &str, salt: &[u8]) -> Result<Key> {
        let mut key = [0u8; 32];
        Argon2::default()
            .hash_password_into(password.as_bytes(), salt, &mut key)
            .map_err(|e| anyhow!("argon2: {e}"))?;
        Ok(Key(key))
    }

    /// zstd-compress then AES-256-GCM seal. Output: `[12-byte nonce][ciphertext]`.
    pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let packed = zstd::encode_all(plaintext, 10)?;
        let cipher = Aes256Gcm::new((&self.0).into());
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let ct = cipher
            .encrypt(Nonce::from_slice(&nonce), packed.as_ref())
            .map_err(|e| anyhow!("seal: {e}"))?;
        let mut out = Vec::with_capacity(12 + ct.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    /// Reverse of `seal`. Fails on wrong key or tampering.
    pub fn open(&self, blob: &[u8]) -> Result<Vec<u8>> {
        if blob.len() < 12 {
            return Err(anyhow!("blob too short"));
        }
        let (nonce, ct) = blob.split_at(12);
        let cipher = Aes256Gcm::new((&self.0).into());
        let packed = cipher
            .decrypt(Nonce::from_slice(nonce), ct)
            .map_err(|_| anyhow!("decrypt failed (wrong password or corrupt file)"))?;
        Ok(zstd::decode_all(packed.as_slice())?)
    }
}

pub fn random_salt() -> [u8; 16] {
    let mut s = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut s);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_wrong_password() {
        let salt = random_salt();
        let k = Key::derive("hunter2", &salt).unwrap();
        let blob = k.seal(b"network logs galore").unwrap();
        assert_eq!(k.open(&blob).unwrap(), b"network logs galore");

        let wrong = Key::derive("hunter3", &salt).unwrap();
        assert!(wrong.open(&blob).is_err());
    }
}
