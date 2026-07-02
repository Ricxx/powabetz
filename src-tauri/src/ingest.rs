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
            let is_post_ingest = method == tiny_http::Method::Post && req.url().starts_with("/ingest");
            if !is_post_ingest {
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

fn cors(status: u16, body: &str) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let mut resp = tiny_http::Response::from_string(body).with_status_code(status);
    for (k, val) in [
        ("Access-Control-Allow-Origin", "*"),
        ("Access-Control-Allow-Methods", "POST, OPTIONS"),
        ("Access-Control-Allow-Headers", "content-type, x-ingest-token"),
        ("Content-Type", "application/json"),
    ] {
        if let Ok(h) = tiny_http::Header::from_bytes(k.as_bytes(), val.as_bytes()) {
            resp.add_header(h);
        }
    }
    resp
}
