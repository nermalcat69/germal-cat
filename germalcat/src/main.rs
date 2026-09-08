//! Germal Cat — a local daemon that launches real browsers, records every
//! network request and console message of a session, and stores it encrypted.

mod agent;
mod api;
mod har;
mod recorder;
mod state;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde_json::Value;
use std::sync::Arc;

const API: &str = "http://127.0.0.1:8420";
const LABEL: &str = "com.graycup.germalcat";

#[derive(Parser)]
#[command(name = "germalcat", about = "Germal Cat — encrypted browser session recorder", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the daemon (invoked by launchd; also fine to run in a terminal).
    Serve,
    /// Unlock the vault (or create it on first run) with the dashboard password.
    Unlock,
    /// Forgot the password: wipe all recorded sessions and set a new one.
    ResetPassword,
    /// Start recording a browser session.
    Record {
        url: String,
        #[arg(long, default_value = "chrome")]
        browser: String,
        #[arg(long, default_value = "default")]
        project: String,
    },
    /// Stop a running recording by id.
    Stop { id: String },
    /// List recorded sessions.
    List,
    /// Install the launchd agent so the daemon starts at login.
    Install,
    /// Manage the launchd agent.
    Daemon {
        #[command(subcommand)]
        action: DaemonCmd,
    },
}

#[derive(Subcommand)]
enum DaemonCmd { Start, Stop, Restart, Status }

fn main() -> Result<()> {
    let cli = Cli::parse();
    if matches!(cli.command, Some(Cmd::Serve)) {
        return serve();
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(run_cli(cli.command))
}

async fn run_cli(cmd: Option<Cmd>) -> Result<()> {
    match cmd {
        None => {
            let v = get("/api/status").await?;
            println!("initialized: {}", v["initialized"]);
            println!("unlocked:    {}", v["unlocked"]);
            println!("active:      {}", v["active"]);
            println!("\ndashboard → run `bun start` in ./dashboard, then open http://localhost:8000");
            Ok(())
        }
        Some(Cmd::Serve) => unreachable!(),
        Some(Cmd::Unlock) => {
            let pw = prompt("Vault password: ")?;
            post("/api/unlock", serde_json::json!({ "password": pw })).await?;
            println!("unlocked");
            Ok(())
        }
        Some(Cmd::ResetPassword) => {
            eprintln!("This deletes ALL recorded sessions. They cannot be recovered.");
            if prompt("Type 'wipe' to confirm: ")? != "wipe" {
                bail!("aborted");
            }
            let pw = prompt("New password: ")?;
            if prompt("Confirm password: ")? != pw {
                bail!("passwords do not match");
            }
            post("/api/reset", serde_json::json!({ "password": pw })).await?;
            println!("vault reset");
            Ok(())
        }
        Some(Cmd::Record { url, browser, project }) => {
            let v = post("/api/record", serde_json::json!({ "url": url, "browser": browser, "project": project })).await?;
            println!("recording {} — stop with: germalcat stop {}", v["id"], v["id"]);
            Ok(())
        }
        Some(Cmd::Stop { id }) => {
            post(&format!("/api/sessions/{id}/stop"), Value::Null).await?;
            println!("stopped {id}");
            Ok(())
        }
        Some(Cmd::List) => {
            let v = get("/api/sessions").await?;
            for s in v.as_array().map(Vec::as_slice).unwrap_or_default() {
                println!(
                    "{}  {:<12}  {:<7}  reqs {:<5} console {:<4}  {}",
                    s["id"].as_str().unwrap_or("?"),
                    s["project"].as_str().unwrap_or("-"),
                    s["browser"].as_str().unwrap_or("?"),
                    s["request_count"], s["console_count"],
                    s["url"].as_str().unwrap_or("?"),
                );
            }
            Ok(())
        }
        Some(Cmd::Install) => install(),
        Some(Cmd::Daemon { action }) => daemon(action),
    }
}

fn serve() -> Result<()> {
    use tracing_subscriber::{fmt, EnvFilter};
    fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("germalcat=info".parse()?))
        .init();

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(async {
        let vault = germalcat_core::Vault::open(germalcat_core::data_dir());
        let daemon = Arc::new(state::Daemon::new(vault));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:8420").await?;
        tracing::info!("germalcat daemon on http://127.0.0.1:8420");
        axum::serve(listener, api::router(daemon)).await?;
        Ok(())
    })
}

// --- HTTP helpers to talk to the running daemon ---

async fn get(path: &str) -> Result<Value> {
    reqwest::get(format!("{API}{path}"))
        .await
        .context("daemon not reachable — run `germalcat daemon start`")?
        .error_for_status()?
        .json()
        .await
        .context("bad response")
}

async fn post(path: &str, body: Value) -> Result<Value> {
    let resp = reqwest::Client::new()
        .post(format!("{API}{path}"))
        .json(&body)
        .send()
        .await
        .context("daemon not reachable — run `germalcat daemon start`")?;
    if !resp.status().is_success() {
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        bail!("{}", v["error"].as_str().unwrap_or("request failed"));
    }
    Ok(resp.json().await.unwrap_or(Value::Null))
}

fn prompt(label: &str) -> Result<String> {
    if let Ok(p) = std::env::var("GERMALCAT_PASSWORD") {
        return Ok(p);
    }
    Ok(rpassword::prompt_password(label)?)
}

// --- launchd install (macOS) ---

fn plist_path() -> String {
    format!("{}/Library/LaunchAgents/{LABEL}.plist", home())
}
fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

fn install() -> Result<()> {
    let exe = std::env::current_exe()?.display().to_string();
    let dir = format!("{}/Library/LaunchAgents", home());
    std::fs::create_dir_all(&dir)?;
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key><array><string>{exe}</string><string>serve</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>/tmp/germalcat.log</string>
  <key>StandardErrorPath</key><string>/tmp/germalcat.err</string>
</dict></plist>
"#
    );
    let path = plist_path();
    std::fs::write(&path, plist)?;
    let _ = std::process::Command::new("launchctl").args(["unload", &path]).output();
    run("launchctl", &["load", "-w", &path])?;
    println!("installed → {path}\ndaemon running on http://127.0.0.1:8420");
    Ok(())
}

fn daemon(action: DaemonCmd) -> Result<()> {
    let p = plist_path();
    match action {
        DaemonCmd::Start => run("launchctl", &["load", "-w", &p]),
        DaemonCmd::Stop => run("launchctl", &["unload", &p]),
        DaemonCmd::Restart => {
            let _ = run("launchctl", &["unload", &p]);
            run("launchctl", &["load", "-w", &p])
        }
        DaemonCmd::Status => {
            let out = std::process::Command::new("launchctl").args(["list", LABEL]).output()?;
            if out.status.success() {
                print!("{}", String::from_utf8_lossy(&out.stdout));
            } else {
                println!("not loaded - run `germalcat daemon start`");
            }
            Ok(())
        }
    }
}

fn run(prog: &str, args: &[&str]) -> Result<()> {
    let s = std::process::Command::new(prog).args(args).status()?;
    if !s.success() {
        bail!("{prog} {args:?} failed");
    }
    Ok(())
}
