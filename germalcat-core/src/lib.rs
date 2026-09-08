//! Germal Cat core: encrypted-at-rest storage for browser research sessions.
//!
//! Everything a session captures (metadata, network HAR, console logs) is
//! zstd-compressed then sealed with AES-256-GCM. The key is derived from the
//! dashboard password via Argon2id. Lose the password and the old sessions are
//! unreadable — `Vault::reset` wipes them and starts a fresh vault.

mod crypto;
mod session;
mod vault;

pub use crypto::Key;
pub use session::{SessionMeta, SessionStore};
pub use vault::Vault;

use std::path::PathBuf;

/// `~/Library/Application Support/GermalCat` (or the platform data dir).
pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("GermalCat")
}
