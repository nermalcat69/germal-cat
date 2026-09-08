use crate::crypto::{random_salt, Key};
use anyhow::{anyhow, bail, Result};
use base64::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const VERIFIER_PLAINTEXT: &[u8] = b"germalcat-vault-v1";

#[derive(Serialize, Deserialize)]
struct VaultFile {
    salt: String,          // base64
    verifier: String,      // base64 of Key::seal(VERIFIER_PLAINTEXT)
}

/// On-disk encrypted store. One password unlocks it; resetting wipes every
/// recorded session and starts over with a new password.
pub struct Vault {
    root: PathBuf,
}

impl Vault {
    pub fn open(root: impl Into<PathBuf>) -> Vault {
        Vault { root: root.into() }
    }

    fn vault_path(&self) -> PathBuf {
        self.root.join("vault.json")
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }

    pub fn is_initialized(&self) -> bool {
        self.vault_path().exists()
    }

    /// Create the vault for the first time (fails if one already exists).
    pub fn init(&self, password: &str) -> Result<Key> {
        if self.is_initialized() {
            bail!("vault already exists — use unlock, or reset to wipe it");
        }
        self.write_new(password)
    }

    /// Derive the key and check it against the stored verifier.
    pub fn unlock(&self, password: &str) -> Result<Key> {
        let raw = fs::read_to_string(self.vault_path())
            .map_err(|_| anyhow!("vault not initialized"))?;
        let vf: VaultFile = serde_json::from_str(&raw)?;
        let salt = BASE64_STANDARD.decode(vf.salt)?;
        let key = Key::derive(password, &salt)?;
        let verifier = BASE64_STANDARD.decode(vf.verifier)?;
        if key.open(&verifier).ok().as_deref() != Some(VERIFIER_PLAINTEXT) {
            bail!("wrong password");
        }
        Ok(key)
    }

    /// Forgot-password path: delete all sessions and re-init with a new password.
    pub fn reset(&self, new_password: &str) -> Result<Key> {
        let s = self.sessions_dir();
        if s.exists() {
            fs::remove_dir_all(&s)?;
        }
        let _ = fs::remove_file(self.vault_path());
        self.write_new(new_password)
    }

    fn write_new(&self, password: &str) -> Result<Key> {
        fs::create_dir_all(&self.root)?;
        fs::create_dir_all(self.sessions_dir())?;
        let salt = random_salt();
        let key = Key::derive(password, &salt)?;
        let vf = VaultFile {
            salt: BASE64_STANDARD.encode(salt),
            verifier: BASE64_STANDARD.encode(key.seal(VERIFIER_PLAINTEXT)?),
        };
        write_private(&self.vault_path(), serde_json::to_vec_pretty(&vf)?)?;
        Ok(key)
    }
}

/// Write a file readable only by the owner (0600).
pub(crate) fn write_private(path: &Path, bytes: impl AsRef<[u8]>) -> Result<()> {
    fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_unlock_reset() {
        let dir = std::env::temp_dir().join(format!("gc-vault-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let v = Vault::open(&dir);

        v.init("pw1").unwrap();
        assert!(v.unlock("pw1").is_ok());
        assert!(v.unlock("nope").is_err());
        assert!(v.init("pw1").is_err()); // already exists

        fs::write(v.sessions_dir().join("marker"), b"x").unwrap();
        v.reset("pw2").unwrap();
        assert!(!v.sessions_dir().join("marker").exists());
        assert!(v.unlock("pw2").is_ok());
        assert!(v.unlock("pw1").is_err());

        fs::remove_dir_all(&dir).unwrap();
    }
}
