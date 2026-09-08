//! Drives a real browser via CDP and records every network request/response
//! and console message for the lifetime of a session.

use crate::har::HarBuilder;
use crate::state::Daemon;
use anyhow::{anyhow, Context, Result};
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::network::{
    EnableParams, EventLoadingFailed, EventLoadingFinished, EventRequestWillBeSent,
    EventResponseReceived, GetResponseBodyParams,
};
use chromiumoxide::cdp::js_protocol::runtime::EventConsoleApiCalled;
use chrono::Utc;
use futures::StreamExt;
use germalcat_core::SessionMeta;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

/// Only pull response bodies up to this size (bytes). Bigger ones are logged
/// with headers/status only.
const MAX_BODY: f64 = 3_000_000.0;

/// Known browser app locations on macOS, keyed by the name the user passes.
fn browser_path(name: &str) -> Result<String> {
    let candidates: &[&str] = match name.to_lowercase().as_str() {
        "chrome" | "" => &["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"],
        "chromium" => &["/Applications/Chromium.app/Contents/MacOS/Chromium"],
        "edge" => &["/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"],
        "brave" => &["/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"],
        "arc" => &["/Applications/Arc.app/Contents/MacOS/Arc"],
        other => return Err(anyhow!("unknown browser '{other}' (chrome|chromium|edge|brave|arc)")),
    };
    candidates
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .map(|p| p.to_string())
        .ok_or_else(|| anyhow!("{name} is not installed at its expected path"))
}

/// Launch `browser`, navigate to `url`, and record until the session is stopped
/// via [`Daemon::stop`]. Blocks until recording is flushed to the vault.
pub async fn run(daemon: Arc<Daemon>, mut meta: SessionMeta) -> Result<SessionMeta> {
    let url = meta.url.clone();
    let browser_name = meta.browser.clone();
    let exe = browser_path(&browser_name)?;
    let config = BrowserConfig::builder()
        .with_head()
        .chrome_executable(exe)
        .build()
        .map_err(|e| anyhow!("browser config: {e}"))?;

    let (mut browser, mut handler) = Browser::launch(config).await.context("launch browser")?;
    let pump = tokio::spawn(async move { while let Some(h) = handler.next().await { if h.is_err() { break; } } });

    let page = Arc::new(browser.new_page("about:blank").await?);
    page.execute(EnableParams::default()).await.context("Network.enable")?;

    let har = Arc::new(Mutex::new(HarBuilder::default()));
    let console: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));

    let mut tasks = Vec::new();

    // --- request sent ---
    {
        let (har, mut ev) = (har.clone(), page.event_listener::<EventRequestWillBeSent>().await?);
        tasks.push(tokio::spawn(async move {
            while let Some(e) = ev.next().await {
                har.lock().unwrap().request(
                    &id(&e.request_id),
                    e.request.method.clone(),
                    e.request.url.clone(),
                    serde_json::to_value(&e.request.headers).unwrap_or(Value::Null),
                    // ponytail: dumped as the raw CDP postDataEntries JSON —
                    // decode the base64 `bytes` per entry if you need exact bytes.
                    e.request.post_data_entries.as_ref().and_then(|v| {
                        serde_json::to_value(v).ok().map(|j| j.to_string())
                    }),
                    Utc::now(),
                );
            }
        }));
    }

    // --- response headers ---
    {
        let (har, mut ev) = (har.clone(), page.event_listener::<EventResponseReceived>().await?);
        tasks.push(tokio::spawn(async move {
            while let Some(e) = ev.next().await {
                har.lock().unwrap().response(
                    &id(&e.request_id),
                    e.response.status,
                    e.response.status_text.clone(),
                    serde_json::to_value(&e.response.headers).unwrap_or(Value::Null),
                    e.response.mime_type.clone(),
                );
            }
        }));
    }

    // --- loading finished: grab the body while it's still in the browser cache ---
    {
        let (har, page2, mut ev) =
            (har.clone(), page.clone(), page.event_listener::<EventLoadingFinished>().await?);
        tasks.push(tokio::spawn(async move {
            while let Some(e) = ev.next().await {
                let rid = id(&e.request_id);
                har.lock().unwrap().finished(&rid, e.encoded_data_length as i64, Utc::now());
                if e.encoded_data_length <= MAX_BODY {
                    if let Ok(resp) = page2
                        .execute(GetResponseBodyParams::new(e.request_id.clone()))
                        .await
                    {
                        har.lock().unwrap().body(&rid, resp.result.body.clone(), resp.result.base64_encoded);
                    }
                }
            }
        }));
    }

    // --- loading failed ---
    {
        let (har, mut ev) = (har.clone(), page.event_listener::<EventLoadingFailed>().await?);
        tasks.push(tokio::spawn(async move {
            while let Some(e) = ev.next().await {
                har.lock().unwrap().failed(&id(&e.request_id), e.error_text.clone(), Utc::now());
            }
        }));
    }

    // --- console ---
    {
        let (console, mut ev) =
            (console.clone(), page.event_listener::<EventConsoleApiCalled>().await?);
        tasks.push(tokio::spawn(async move {
            while let Some(e) = ev.next().await {
                console.lock().unwrap().push(json!({
                    "ts": Utc::now().to_rfc3339(),
                    "type": format!("{:?}", e.r#type),
                    "args": e.args.iter().map(|a| a.value.clone().unwrap_or(Value::Null)).collect::<Vec<_>>(),
                }));
            }
        }));
    }

    let (stop_tx, stop_rx) = oneshot::channel();
    daemon.register(meta.id.clone(), stop_tx);

    page.goto(&url).await.context("navigate")?;
    tracing::info!(id = %meta.id, %url, "recording started");

    // ponytail: stop signal only — a user closing the window by hand is caught
    // on the next daemon shutdown, not instantly. Add a browser-closed watch if
    // that matters.
    let _ = stop_rx.await;
    daemon.finish(&meta.id);

    // Give in-flight body fetches a moment, then tear down listeners.
    tokio::time::sleep(Duration::from_millis(300)).await;
    for t in tasks {
        t.abort();
    }

    meta.ended_at = Some(Utc::now());
    let har_json = har.lock().unwrap().to_json(&url, meta.started_at);
    meta.request_count = har.lock().unwrap().len();
    let console_lines = {
        let c = console.lock().unwrap();
        meta.console_count = c.len() as u64;
        c.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\n")
    };

    let store = daemon.store()?;
    store.save_har(&meta.id, serde_json::to_vec_pretty(&har_json)?.as_slice())?;
    store.save_console(&meta.id, console_lines.as_bytes())?;
    store.save_meta(&meta)?;

    let _ = browser.close().await;
    pump.abort();
    tracing::info!(id = %meta.id, requests = meta.request_count, "recording saved");
    Ok(meta)
}

/// CDP `RequestId` -> plain string (it serializes transparently as one).
fn id<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}
