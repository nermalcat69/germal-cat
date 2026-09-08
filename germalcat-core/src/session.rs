use crate::crypto::Key;
use crate::vault::write_private;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone)]
pub struct SessionMeta {
    pub id: String,
    pub project: String,
    pub url: String,
    pub browser: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub request_count: u64,
    pub console_count: u64,
}

impl SessionMeta {
    pub fn new(project: &str, url: &str, browser: &str) -> SessionMeta {
        let now = Utc::now();
        SessionMeta {
            id: now.format("%Y%m%dT%H%M%S%3fZ").to_string(),
            project: if project.trim().is_empty() { "default".into() } else { project.trim().to_string() },
            url: url.to_string(),
            browser: browser.to_string(),
            started_at: now,
            ended_at: None,
            request_count: 0,
            console_count: 0,
        }
    }
}

/// Reads and writes the encrypted files under `sessions/<id>/`.
///
/// - `meta.json`     — [`SessionMeta`]
/// - `network.har`   — HAR 1.2 document (built by the recorder)
/// - `console.jsonl` — one JSON console entry per line
pub struct SessionStore {
    sessions_dir: PathBuf,
    key: Key,
}

impl SessionStore {
    pub fn new(sessions_dir: impl Into<PathBuf>, key: Key) -> SessionStore {
        SessionStore { sessions_dir: sessions_dir.into(), key }
    }

    fn dir(&self, id: &str) -> PathBuf {
        self.sessions_dir.join(id)
    }

    fn write_sealed(&self, path: &Path, plaintext: &[u8]) -> Result<()> {
        write_private(path, self.key.seal(plaintext)?)
    }

    fn read_sealed(&self, path: &Path) -> Result<Vec<u8>> {
        let blob = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        self.key.open(&blob)
    }

    pub fn save_meta(&self, m: &SessionMeta) -> Result<()> {
        let d = self.dir(&m.id);
        fs::create_dir_all(&d)?;
        self.write_sealed(&d.join("meta.json.zst.aes"), &serde_json::to_vec(m)?)
    }

    pub fn save_har(&self, id: &str, har: &[u8]) -> Result<()> {
        self.write_sealed(&self.dir(id).join("network.har.zst.aes"), har)
    }

    pub fn save_console(&self, id: &str, jsonl: &[u8]) -> Result<()> {
        self.write_sealed(&self.dir(id).join("console.jsonl.zst.aes"), jsonl)
    }

    pub fn load_meta(&self, id: &str) -> Result<SessionMeta> {
        Ok(serde_json::from_slice(
            &self.read_sealed(&self.dir(id).join("meta.json.zst.aes"))?,
        )?)
    }

    pub fn load_har(&self, id: &str) -> Result<Vec<u8>> {
        self.read_sealed(&self.dir(id).join("network.har.zst.aes"))
    }

    pub fn load_console(&self, id: &str) -> Result<Vec<u8>> {
        self.read_sealed(&self.dir(id).join("console.jsonl.zst.aes"))
    }

    /// All sessions, newest first. Entries that fail to decrypt are skipped
    /// (e.g. leftovers from a previous password after a partial reset).
    pub fn list(&self) -> Vec<SessionMeta> {
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(&self.sessions_dir) else {
            return out;
        };
        for e in entries.flatten() {
            if let Some(name) = e.file_name().to_str() {
                if let Ok(m) = self.load_meta(name) {
                    out.push(m);
                }
            }
        }
        out.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        out
    }
}
