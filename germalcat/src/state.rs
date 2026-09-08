use anyhow::{bail, Result};
use germalcat_core::{Key, SessionStore, Vault};
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::oneshot;

/// Everything the daemon shares between the HTTP API and the recorder tasks.
pub struct Daemon {
    pub vault: Vault,
    key: Mutex<Option<Key>>,
    /// session id -> "please stop now" signal for the running recorder.
    stoppers: Mutex<HashMap<String, oneshot::Sender<()>>>,
}

impl Daemon {
    pub fn new(vault: Vault) -> Daemon {
        Daemon {
            vault,
            key: Mutex::new(None),
            stoppers: Mutex::new(HashMap::new()),
        }
    }

    pub fn unlock(&self, password: &str) -> Result<()> {
        let key = if self.vault.is_initialized() {
            self.vault.unlock(password)?
        } else {
            self.vault.init(password)?
        };
        *self.key.lock().unwrap() = Some(key);
        Ok(())
    }

    pub fn reset(&self, new_password: &str) -> Result<()> {
        let key = self.vault.reset(new_password)?;
        *self.key.lock().unwrap() = Some(key);
        Ok(())
    }

    pub fn is_unlocked(&self) -> bool {
        self.key.lock().unwrap().is_some()
    }

    /// A [`SessionStore`] bound to the unlocked key, or an error if still locked.
    pub fn store(&self) -> Result<SessionStore> {
        match &*self.key.lock().unwrap() {
            Some(k) => Ok(SessionStore::new(self.vault.sessions_dir(), k.clone())),
            None => bail!("locked — POST /api/unlock first"),
        }
    }

    pub fn register(&self, id: String, stop: oneshot::Sender<()>) {
        self.stoppers.lock().unwrap().insert(id, stop);
    }

    pub fn finish(&self, id: &str) {
        self.stoppers.lock().unwrap().remove(id);
    }

    /// Signal a running recorder to stop. Returns false if no such session.
    pub fn stop(&self, id: &str) -> bool {
        match self.stoppers.lock().unwrap().remove(id) {
            Some(tx) => {
                let _ = tx.send(());
                true
            }
            None => false,
        }
    }

    pub fn active_ids(&self) -> Vec<String> {
        self.stoppers.lock().unwrap().keys().cloned().collect()
    }
}
