//! Local HTTP ingest endpoint for the browser extension. Binds to 127.0.0.1 only
//! and requires a shared token, so only your extension can post to it. The
//! extension sends the visible text of any page you're on (your real browser, so
//! no bot-detection); we just store it raw — Haiku structures it later, on demand.

use std::io::Read;
use std::path::PathBuf;

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::db;

/// A short random token for the localhost ingest endpoint.
pub fn gen_token() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut h = Sha256::new();
    h.update(nanos.to_le_bytes());
    h.update(std::process::id().to_le_bytes());
    format!("{:x}", h.finalize())[..24].to_string()
}

/// Spawn the ingest server on a background thread. Best-effort: a bind failure is
/// logged and the app keeps running (the extension just won't connect).
pub fn start(db_path: PathBuf, token: String, port: u16) {
    std::thread::spawn(move || {
        let server = match tiny_http::Server::http(("127.0.0.1", port)) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("ingest: could not bind 127.0.0.1:{port}: {e}");
                return;
            }
        };
        let conn = match Connection::open(&db_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("ingest: db open failed: {e}");
                return;
            }
        };
        // The app holds its own connection to this file — WAL + a busy timeout
        // stop a POST during a build's cache writes failing "database is locked".
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let _ = conn.pragma_update(None, "busy_timeout", 3000);
        eprintln!("ingest: listening on http://127.0.0.1:{port}/ingest");
        for mut req in server.incoming_requests() {
            let method = req.method().clone();
            // CORS preflight.
            if method == tiny_http::Method::Options {
                let _ = req.respond(cors(204, ""));
                continue;
            }
            // TEMP DIAGNOSTIC: the webview beacons JS boot progress + errors
            // here so frontend crashes are visible in this terminal.
            if req.url().starts_with("/jslog") {
                let q = req.url().split_once('?').map(|(_, q)| q).unwrap_or("");
                let msg = q.strip_prefix("m=").map(url_decode).unwrap_or_default();
                eprintln!("[webview] {}", msg.chars().take(600).collect::<String>());
                let _ = req.respond(cors(200, "ok"));
                continue;
            }
            let is_post_ingest = method == tiny_http::Method::Post && req.url().starts_with("/ingest");
            let is_status = method == tiny_http::Method::Get && req.url().starts_with("/status");
            let is_ticket = method == tiny_http::Method::Get && req.url().starts_with("/ticket");
            if !is_post_ingest && !is_status && !is_ticket {
                let status = if req.url() == "/" { 200 } else { 404 };
                let _ = req.respond(cors(status, "powabetz ingest"));
                continue;
            }
            // Token check.
            let tok = req
                .headers()
                .iter()
                .find(|h| h.field.equiv("X-Ingest-Token"))
                .map(|h| h.value.as_str().to_string())
                .unwrap_or_default();
            if tok != token {
                let _ = req.respond(cors(401, "{\"ok\":false,\"error\":\"unauthorized\"}"));
                continue;
            }
            // GET /ticket → the latest built slate (auto-saved on every fresh
            // build) for the extension's slip assistant on Bet365.
            if is_ticket {
                let payload = crate::db::latest_saved_ticket(&conn)
                    .ok()
                    .flatten()
                    .map(|(ts, json)| format!("{{\"ok\":true,\"created_at\":{ts},\"result\":{json}}}"))
                    .unwrap_or_else(|| "{\"ok\":false,\"error\":\"no builds yet\"}".to_string());
                let _ = req.respond(cors(200, &payload));
                continue;
            }
            // GET /status?url=<encoded> → counts + whether THIS page is already
            // ingested (drives the extension's floating research bar).
            if is_status {
                let url_param = req
                    .url()
                    .split_once('?')
                    .map(|(_, q)| q)
                    .unwrap_or("")
                    .split('&')
                    .find_map(|kv| kv.strip_prefix("url="))
                    .map(url_decode)
                    .unwrap_or_default();
                let _ = req.respond(cors(200, &status_json(&conn, &url_param)));
                continue;
            }
            let mut body = String::new();
            let _ = req.as_reader().read_to_string(&mut body);
            let resp = match handle(&conn, &body) {
                Ok(id) => format!("{{\"ok\":true,\"id\":{id}}}"),
                Err(e) => format!("{{\"ok\":false,\"error\":\"{}\"}}", e.replace('"', "'")),
            };
            let _ = req.respond(cors(200, &resp));
        }
    });
}

fn handle(conn: &Connection, body: &str) -> Result<i64, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let url = v.get("url").and_then(|x| x.as_str()).unwrap_or("").trim();
    if url.is_empty() {
        return Err("missing url".into());
    }
    let title = v.get("title").and_then(|x| x.as_str()).unwrap_or("");
    let note = v.get("note").and_then(|x| x.as_str()).unwrap_or("");
    let mut content = v.get("content").and_then(|x| x.as_str()).unwrap_or("").to_string();
    // Bound the stored/forwarded text so one giant page can't blow up tokens.
    // Walk back to a char boundary — truncate() PANICS mid-UTF-8 (any accented
    // player name near the cut killed the ingest server silently).
    if content.len() > 120_000 {
        let mut cut = 120_000;
        while cut > 0 && !content.is_char_boundary(cut) {
            cut -= 1;
        }
        content.truncate(cut);
    }
    let url_hash = format!("{:x}", Sha256::digest(url.as_bytes()));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    db::ingest_add(conn, now, url, &url_hash, title, &content, note)
}

/// Minimal percent-decoding for the status query param (no extra deps).
fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(v) = u8::from_str_radix(hex, 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Status payload for the floating bar: total ingested, unprocessed count, and
/// whether the given URL is already in (with its processing status).
fn status_json(conn: &Connection, url: &str) -> String {
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM ingested", [], |r| r.get(0))
        .unwrap_or(0);
    let new_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ingested WHERE status != 'processed'", [], |r| r.get(0))
        .unwrap_or(0);
    let page_status: Option<String> = if url.is_empty() {
        None
    } else {
        let hash = format!("{:x}", Sha256::digest(url.as_bytes()));
        conn.query_row("SELECT status FROM ingested WHERE url_hash = ?1", [&hash], |r| r.get(0))
            .ok()
    };
    format!(
        "{{\"ok\":true,\"count\":{total},\"new_count\":{new_count},\"ingested\":{},\"status\":{}}}",
        page_status.is_some(),
        page_status.map(|s| format!("\"{s}\"")).unwrap_or_else(|| "null".to_string())
    )
}

fn cors(status: u16, body: &str) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let mut resp = tiny_http::Response::from_string(body).with_status_code(status);
    for (k, val) in [
        ("Access-Control-Allow-Origin", "*"),
        ("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
        ("Access-Control-Allow-Headers", "content-type, x-ingest-token"),
        ("Content-Type", "application/json"),
    ] {
        if let Ok(h) = tiny_http::Header::from_bytes(k.as_bytes(), val.as_bytes()) {
            resp.add_header(h);
        }
    }
    resp
}
