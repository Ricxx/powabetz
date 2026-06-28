//! Cache-first API-Football (api-sports.io v3) client. Every external call goes
//! through `cached_get`, which enforces the daily request meter (HARD CONSTRAINT).

use std::time::{Duration, Instant};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::db;
use crate::AppState;

const HOST: &str = "https://v3.football.api-sports.io";

/// Minimum spacing between fresh network calls. Paid plans allow hundreds per
/// minute, so this is light; the rate-limit retry below is the real safety net.
const MIN_INTERVAL: Duration = Duration::from_millis(300);
/// Back-off waits (seconds) when the API reports its per-minute rate limit.
const RATELIMIT_WAITS: [u64; 3] = [5, 10, 20];

pub fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// sha256(endpoint + sorted_params) — the cache key.
pub fn cache_key(endpoint: &str, params: &[(&str, String)]) -> String {
    let mut sorted: Vec<String> = params.iter().map(|(k, v)| format!("{k}={v}")).collect();
    sorted.sort();
    let mut hasher = Sha256::new();
    hasher.update(endpoint.as_bytes());
    hasher.update(b"?");
    hasher.update(sorted.join("&").as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Cache-first GET. Returns the parsed JSON body. On a miss, checks the meter,
/// performs the request, increments the meter, and stores the payload.
pub async fn cached_get(
    state: &AppState,
    endpoint: &str,
    params: Vec<(&str, String)>,
    ttl: i64,
) -> Result<Value, String> {
    cached_get_inner(state, endpoint, params, ttl, false).await
}

/// Like `cached_get` but ALWAYS hits the network (skips the cache read), then
/// stores the fresh body with `ttl`. Use when a cached row may be stale in a way
/// the TTL can't capture — e.g. a fixture cached while still live must be
/// re-pulled to see the final result.
pub async fn fetch_fresh(
    state: &AppState,
    endpoint: &str,
    params: Vec<(&str, String)>,
    ttl: i64,
) -> Result<Value, String> {
    cached_get_inner(state, endpoint, params, ttl, true).await
}

async fn cached_get_inner(
    state: &AppState,
    endpoint: &str,
    params: Vec<(&str, String)>,
    ttl: i64,
    force_refresh: bool,
) -> Result<Value, String> {
    let key = cache_key(endpoint, &params);
    let now = now_ts();

    // 1. cache read (lock scope contains no await)
    if !force_refresh {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        if let Some(payload) = db::cache_get(&conn, &key, now)? {
            return serde_json::from_str(&payload).map_err(|e| e.to_string());
        }
    }

    // 2. budget gate — block fresh calls at the configured daily limit
    let day = today();
    let limit = {
        let keys = state.keys.lock().map_err(|_| "keys lock")?;
        keys.daily_limit.unwrap_or(db::DEFAULT_DAILY_LIMIT)
    };
    {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        let count = db::meter_count(&conn, &day)?;
        if count >= limit {
            return Err(format!(
                "Daily request budget reached ({count}/{limit}). Cached data still works; fresh fetches resume tomorrow."
            ));
        }
    }

    // 3. key (or proxy). In server mode the proxy holds the real key.
    let (api_key, proxy) = {
        let keys = state.keys.lock().map_err(|_| "keys lock")?;
        (keys.api_football.clone(), keys.proxy())
    };
    if proxy.is_none() && api_key.is_none() {
        return Err("API-Football key not set. Add it in Settings.".to_string());
    }
    let api_key = api_key.unwrap_or_default();

    // 4. throttle + network. Hold the throttle lock across the request so fresh
    // calls are serialised and spaced out; retry the API's per-minute rate limit
    // (which arrives as a 200 body with errors.rateLimit, or occasionally a 429).
    let url = match &proxy {
        Some((base, _)) => format!("{base}/af{endpoint}"),
        None => format!("{HOST}{endpoint}"),
    };
    let mut last = state.throttle.lock().await;
    let elapsed = last.elapsed();
    if elapsed < MIN_INTERVAL {
        tokio::time::sleep(MIN_INTERVAL - elapsed).await;
    }

    let mut attempt = 0usize;
    let (text, json) = loop {
        let mut req = state.http.get(&url).query(&params);
        req = match &proxy {
            Some((_, token)) => req.header("x-proxy-token", token),
            None => req.header("x-apisports-key", &api_key),
        };
        let resp = req.send().await.map_err(|e| format!("request failed: {e}"))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("read body failed: {e}"))?;

        if !status.is_success() && status.as_u16() != 429 {
            *last = Instant::now();
            return Err(format!("API-Football {endpoint} returned {status}: {text}"));
        }

        let json: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;

        let rate_limited = status.as_u16() == 429
            || json.get("errors").and_then(|e| e.get("rateLimit")).is_some();
        if rate_limited {
            if attempt < RATELIMIT_WAITS.len() {
                tokio::time::sleep(Duration::from_secs(RATELIMIT_WAITS[attempt])).await;
                attempt += 1;
                continue;
            }
            *last = Instant::now();
            return Err(
                "API-Football per-minute rate limit hit. Wait ~a minute and retry (the free tier allows only a few requests per minute).".to_string(),
            );
        }

        // Other API errors (daily limit, bad params, etc.) come under "errors".
        if let Some(errors) = json.get("errors") {
            let nonempty = errors.as_object().map(|o| !o.is_empty()).unwrap_or(false)
                || errors.as_array().map(|a| !a.is_empty()).unwrap_or(false);
            if nonempty {
                *last = Instant::now();
                return Err(format!("API-Football error on {endpoint}: {errors}"));
            }
        }
        break (text, json);
    };
    *last = Instant::now();
    drop(last);

    // 5. count + store
    {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        db::meter_increment(&conn, &day)?;
        db::cache_put(&conn, &key, endpoint, &text, now, ttl)?;
    }

    Ok(json)
}

// ---------- TTLs (seconds) ----------
pub const TTL_FIXTURES: i64 = 12 * 3600;
pub const TTL_PLAYERS: i64 = 24 * 3600;
pub const TTL_SQUADS: i64 = 24 * 3600;
pub const TTL_INJURIES: i64 = 6 * 3600;
/// Confirmed lineups (start XI) — posted ~1h before kickoff; refresh hourly.
pub const TTL_LINEUPS: i64 = 3600;
pub const TTL_TEAMS: i64 = 24 * 3600;
pub const TTL_LEAGUES: i64 = 7 * 24 * 3600;
pub const TTL_ODDS: i64 = 3600;
pub const TTL_PREDICTIONS: i64 = 6 * 3600;
/// Live (in-play) fixture state — short TTL so the score/clock stays fresh.
pub const TTL_LIVE: i64 = 60;

/// Helper: pull the `response` array out of an API-Football body.
pub fn response_array(json: &Value) -> Vec<Value> {
    json.get("response")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default()
}
