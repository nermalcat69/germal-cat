# Germal Cat

A local daemon that launches a real browser from your machine, drives a
session, and records **every network request/response and console message**
for research. Recordings are compressed (zstd) and encrypted at rest
(AES-256-GCM) with a key derived from your dashboard password (Argon2id).

Forget the password and the old sessions are gone — the daemon wipes them and
starts a fresh vault with a new password. There is no recovery path by design.

- **Daemon + CLI + recorder:** Rust (`germalcat`, `germalcat-core`)
- **Dashboard:** Bun (`dashboard/`)
- **Browsers:** Chrome, Chromium, Edge, Brave, Arc (anything Chromium, via CDP)

## Build

```sh
cargo build --release
```

## Run the daemon

```sh
./target/release/germalcat serve          # foreground
# or install as a login agent (macOS launchd):
./target/release/germalcat install
```

Daemon listens on `http://127.0.0.1:8420`.

## Unlock

First run sets the password (and creates the encrypted vault at
`~/Library/Application Support/GermalCat/`):

```sh
germalcat unlock
```

## Record a session

```sh
germalcat record https://example.com --browser chrome
germalcat list
germalcat stop <session-id>
```

## Dashboard

```sh
cd dashboard
bun start          # http://localhost:8000
```

The dashboard proxies `/api/*` to the daemon. It prompts for the password,
lists sessions, starts/stops recordings, and shows the captured network table
and console log per session. "Forgot password / wipe" resets the vault.

## Storage layout

```
~/Library/Application Support/GermalCat/
  vault.json                      # salt + password verifier
  sessions/<id>/
    meta.json.zst.aes
    network.har.zst.aes           # HAR 1.2, opens in any devtools once exported
    console.jsonl.zst.aes
```

## API

| Method | Path | |
|---|---|---|
| GET  | `/api/status` | initialized / unlocked / active recordings |
| POST | `/api/unlock` | `{password}` — creates the vault on first call |
| POST | `/api/reset` | `{password}` — **wipes all sessions**, new password |
| POST | `/api/record` | `{url, browser}` — returns session meta |
| POST | `/api/sessions/:id/stop` | stop a running recording |
| GET  | `/api/sessions` | list |
| GET  | `/api/sessions/:id/har` | decrypted HAR JSON |
| GET  | `/api/sessions/:id/console` | decrypted console JSONL |
