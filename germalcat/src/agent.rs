//! Optional bridge to a local coding agent (Claude Code, opencode, codex, …).
//! Settings live unencrypted in `settings.json` — they hold no session data,
//! just which command to shell out to for the request chat.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    /// claude-code | opencode | codex | other
    pub agent: String,
    /// `sh -c` template; the prompt is passed as the `$PROMPT` env var.
    pub command: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { agent: "claude-code".into(), command: default_command("claude-code") }
    }
}

pub fn default_command(agent: &str) -> String {
    match agent {
        "opencode" => "opencode run \"$PROMPT\"",
        "codex" => "codex exec \"$PROMPT\"",
        "claude-code" => "claude -p \"$PROMPT\"",
        _ => "cat", // 'other' — user replaces this with their own command
    }
    .to_string()
}

fn path() -> PathBuf {
    germalcat_core::data_dir().join("settings.json")
}

pub fn load() -> Settings {
    std::fs::read(path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(s: &Settings) -> Result<()> {
    std::fs::create_dir_all(germalcat_core::data_dir())?;
    std::fs::write(path(), serde_json::to_vec_pretty(s)?)?;
    Ok(())
}

/// Run the configured agent once with `prompt` and return its stdout.
pub async fn chat(prompt: &str) -> Result<String> {
    let s = load();
    let fut = Command::new("sh")
        .arg("-c")
        .arg(&s.command)
        .env("PROMPT", prompt)
        .stdin(Stdio::null())
        .output();
    let out = tokio::time::timeout(Duration::from_secs(180), fut)
        .await
        .context("agent timed out after 180s")?
        .context("failed to spawn agent — is it installed and on PATH?")?;
    if !out.status.success() {
        bail!(
            "agent exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
