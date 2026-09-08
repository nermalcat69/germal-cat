//! Minimal HAR 1.2 builder. Fed primitive fields by the recorder so this file
//! stays free of CDP type imports.

use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

#[derive(Default)]
struct Entry {
    started: Option<DateTime<Utc>>,
    req_method: String,
    req_url: String,
    req_headers: Value,
    req_post: Option<String>,
    status: i64,
    status_text: String,
    res_headers: Value,
    mime: String,
    body: Option<String>,
    body_base64: bool,
    bytes: i64,
    finished: Option<DateTime<Utc>>,
    error: Option<String>,
}

#[derive(Default)]
pub struct HarBuilder {
    entries: HashMap<String, Entry>,
    order: Vec<String>,
}

impl HarBuilder {
    fn slot(&mut self, id: &str) -> &mut Entry {
        if !self.entries.contains_key(id) {
            self.order.push(id.to_string());
            self.entries.insert(id.to_string(), Entry::default());
        }
        self.entries.get_mut(id).unwrap()
    }

    pub fn request(
        &mut self,
        id: &str,
        method: String,
        url: String,
        headers: Value,
        post: Option<String>,
        ts: DateTime<Utc>,
    ) {
        let e = self.slot(id);
        e.started = Some(ts);
        e.req_method = method;
        e.req_url = url;
        e.req_headers = headers;
        e.req_post = post;
    }

    pub fn response(
        &mut self,
        id: &str,
        status: i64,
        status_text: String,
        headers: Value,
        mime: String,
    ) {
        let e = self.slot(id);
        e.status = status;
        e.status_text = status_text;
        e.res_headers = headers;
        e.mime = mime;
    }

    pub fn body(&mut self, id: &str, body: String, base64: bool) {
        let e = self.slot(id);
        e.body = Some(body);
        e.body_base64 = base64;
    }

    pub fn finished(&mut self, id: &str, bytes: i64, ts: DateTime<Utc>) {
        let e = self.slot(id);
        e.bytes = bytes;
        e.finished = Some(ts);
    }

    pub fn failed(&mut self, id: &str, error: String, ts: DateTime<Utc>) {
        let e = self.slot(id);
        e.error = Some(error);
        e.finished = Some(ts);
    }

    pub fn len(&self) -> u64 {
        self.order.len() as u64
    }

    pub fn to_json(&self, page_url: &str, started: DateTime<Utc>) -> Value {
        let entries: Vec<Value> = self
            .order
            .iter()
            .filter_map(|id| self.entries.get(id))
            .map(|e| {
                let time_ms = match (e.started, e.finished) {
                    (Some(a), Some(b)) => (b - a).num_milliseconds().max(0),
                    _ => 0,
                };
                let mut content = Map::new();
                content.insert("size".into(), json!(e.bytes.max(0)));
                content.insert("mimeType".into(), json!(e.mime));
                if let Some(b) = &e.body {
                    content.insert("text".into(), json!(b));
                    if e.body_base64 {
                        content.insert("encoding".into(), json!("base64"));
                    }
                }
                json!({
                    "startedDateTime": e.started.unwrap_or(started).to_rfc3339(),
                    "time": time_ms,
                    "request": {
                        "method": e.req_method,
                        "url": e.req_url,
                        "httpVersion": "HTTP/1.1",
                        "headers": headers_array(&e.req_headers),
                        "queryString": [],
                        "cookies": [],
                        "headersSize": -1,
                        "bodySize": e.req_post.as_ref().map(|p| p.len() as i64).unwrap_or(0),
                        "postData": e.req_post.as_ref().map(|p| json!({
                            "mimeType": "application/octet-stream",
                            "text": p,
                        })),
                    },
                    "response": {
                        "status": e.status,
                        "statusText": e.status_text,
                        "httpVersion": "HTTP/1.1",
                        "headers": headers_array(&e.res_headers),
                        "cookies": [],
                        "content": content,
                        "redirectURL": "",
                        "headersSize": -1,
                        "bodySize": e.bytes.max(0),
                        "_error": e.error,
                    },
                    "cache": {},
                    "timings": { "send": 0, "wait": time_ms, "receive": 0 },
                })
            })
            .collect();

        json!({
            "log": {
                "version": "1.2",
                "creator": { "name": "germalcat", "version": env!("CARGO_PKG_VERSION") },
                "pages": [{
                    "startedDateTime": started.to_rfc3339(),
                    "id": "page_1",
                    "title": page_url,
                    "pageTimings": {},
                }],
                "entries": entries,
            }
        })
    }
}

/// CDP headers come as a `{ "Name": "value" }` object; HAR wants `[{name,value}]`.
fn headers_array(v: &Value) -> Value {
    match v.as_object() {
        Some(map) => Value::Array(
            map.iter()
                .map(|(k, val)| json!({ "name": k, "value": val.as_str().unwrap_or("") }))
                .collect(),
        ),
        None => json!([]),
    }
}
