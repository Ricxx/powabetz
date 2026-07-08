//! PropLine (prop-line.com) client — US sports (MLB/NBA) odds + props.
//! Cache-first like apifootball: same cache table, keys prefixed "pl|" so the
//! two providers can never collide. No ticket generation for these sports —
//! the product is an EVIDENCE LIST (fixture / market / side / odds / sharp /
//! prob / implied / hit chance) the user copies out.

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{db, AppState};

const BASE: &str = "https://api.prop-line.com/v1";

pub const TTL_EVENTS: i64 = 1800; // 30 min — slates move slowly
pub const TTL_ODDS: i64 = 900; // 15 min — lines move; refresh is cheap
pub const TTL_TRENDS: i64 = 24 * 3600; // player hit-rates: daily is plenty

fn cache_key(path: &str, params: &[(&str, String)]) -> String {
    let mut sorted: Vec<String> = params.iter().map(|(k, v)| format!("{k}={v}")).collect();
    sorted.sort();
    let mut h = Sha256::new();
    h.update(b"pl|");
    h.update(path.as_bytes());
    h.update(b"?");
    h.update(sorted.join("&").as_bytes());
    format!("{:x}", h.finalize())
}

/// American odds → decimal. (-122 → 1.82, +145 → 2.45)
pub fn american_to_decimal(a: f64) -> f64 {
    if a >= 100.0 {
        1.0 + a / 100.0
    } else if a <= -100.0 {
        1.0 + 100.0 / -a
    } else {
        0.0 // not a valid american price
    }
}

/// Cache-first GET. PropLine has its own tier rate limits — every fresh call
/// shares the app-wide throttle so bursts can't trip a 429.
pub async fn get(state: &AppState, path: &str, params: Vec<(&str, String)>, ttl: i64) -> Result<Value, String> {
    let key = cache_key(path, &params);
    let now = crate::apifootball::now_ts();
    {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        if let Some(p) = db::cache_get(&conn, &key, now)? {
            return serde_json::from_str(&p).map_err(|e| e.to_string());
        }
    }
    // LOCAL key wins; otherwise server-mode routes via the proxy worker, which
    // holds the real PROPLINE_KEY (wrangler secret) — same pattern as DeepSeek.
    let (api_key, proxy) = {
        let keys = state.keys.lock().map_err(|_| "keys lock")?;
        let local = keys.propline.clone();
        let proxy = if local.is_none() { keys.proxy() } else { None };
        (local, proxy)
    };
    if api_key.is_none() && proxy.is_none() {
        return Err("PropLine key not set — add it in Settings (or configure the proxy).".to_string());
    }

    // Space fresh calls out (shared throttle with the football client).
    {
        let mut last = state.throttle.lock().await;
        let since = last.elapsed();
        let min = std::time::Duration::from_millis(300);
        if since < min {
            tokio::time::sleep(min - since).await;
        }
        *last = std::time::Instant::now();
    }

    let mut req = match &proxy {
        Some((base, token)) => state
            .http
            .get(format!("{base}/propline/v1{path}"))
            .header("x-proxy-token", token.clone()),
        None => state.http.get(format!("{BASE}{path}")),
    };
    if let Some(k) = &api_key {
        req = req.header("X-API-Key", k.clone());
    }
    let resp = req
        .query(&params)
        .send()
        .await
        .map_err(|e| format!("PropLine request failed: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.map_err(|e| e.to_string())?;
    if status.as_u16() == 429 {
        return Err("PropLine rate limit hit — wait a minute and try again.".to_string());
    }
    if !status.is_success() {
        return Err(format!("PropLine {status}: {}", body.chars().take(160).collect::<String>()));
    }
    {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        let _ = db::cache_put(&conn, &key, path, &body, now, ttl);
        db::pl_meter_bump(&conn, &crate::apifootball::today());
    }
    serde_json::from_str(&body).map_err(|e| e.to_string())
}
