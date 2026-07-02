//! Grok (x.ai) precursor: searches X/Twitter + news for the latest on each
//! match — injuries, suspensions, lineup leaks, sentiment — and returns a
//! concise digest fed to the model as SOFT context.
//!
//! Cost control: a cheap fast model, capped searches, low output, and PER-MATCH
//! caching so a rebuild (or a different selection sharing a match) reuses the
//! digest instead of paying for a fresh search every time.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::apifootball as af;
use crate::{db, AppState};

const ENDPOINT: &str = "https://api.x.ai/v1/responses";
/// Cheap, fast model — far lower token + search cost than grok-4.3, and it
/// self-limits to a handful of searches.
const MODEL: &str = "grok-4-fast";

// Cost estimate (USD). grok-4-fast token rates + ~per-search-call rate.
const IN_PER_M: f64 = 0.20;
const OUT_PER_M: f64 = 0.50;
const PER_SOURCE: f64 = 0.025;

// Per-match digest cache TTL: news/injuries are stable for hours; refresh fast
// when a match is in play.
const TTL_MATCH: i64 = 3 * 3600;
const TTL_LIVE: i64 = 900;

pub fn grok_cost(input: i64, output: i64, sources: i64) -> f64 {
    let c = (input as f64) / 1_000_000.0 * IN_PER_M
        + (output as f64) / 1_000_000.0 * OUT_PER_M
        + (sources as f64) * PER_SOURCE;
    (c * 10000.0).round() / 10000.0
}

/// Extract confirmed-unavailable names from EVERY "UNAVAILABLE:" line in the
/// digest (one per match in a combined digest).
pub fn parse_unavailable(digest: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in digest.lines() {
        let upper = line.to_uppercase();
        if let Some(idx) = upper.find("UNAVAILABLE:") {
            let after = &line[idx + "UNAVAILABLE:".len()..];
            for name in after.split(|c| c == ';' || c == ',') {
                let n = name.trim_matches(|c: char| c == '*' || c == '_' || c.is_whitespace()).to_string();
                if !n.is_empty() && !n.eq_ignore_ascii_case("none") {
                    out.push(n);
                }
            }
        }
    }
    out
}

pub struct GrokResult {
    pub digest: String,
    pub input: i64,
    pub output: i64,
    pub sources: i64,
    /// Actual x.ai-billed cost (USD) for the fresh calls in this digest.
    pub cost_usd: f64,
}

/// Build the system prompt from the categories the user wants — fewer sections
/// means a tighter prompt and fewer (cheaper) searches.
fn build_system(categories: &[String]) -> String {
    let has = |c: &str| {
        if categories.is_empty() {
            c == "injuries" || c == "news"
        } else {
            categories.iter().any(|x| x.eq_ignore_ascii_case(c))
        }
    };
    let mut s = String::new();
    if has("injuries") {
        s.push_str("\n- OUT/DOUBTFUL: confirmed injuries, suspensions, illness or rotation — name the players. MOST IMPORTANT.");
    }
    if has("news") {
        s.push_str("\n- NEWS: lineup leaks, manager comments, returning players.");
    }
    if has("bets") {
        s.push_str("\n- RECOMMENDED BETS: specific picks sharps/public are backing for this match.");
    }
    if has("analysis") {
        s.push_str("\n- ANALYSIS: one short tactical read (the key matchup/battle).");
    }
    if has("tactics") {
        s.push_str("\n- TACTICS: any CONFIRMED tactical change vs usual — new formation, a switch to low-block/high-press, a key player's role change, or manager comments on approach for THIS match.");
    }
    if has("opinions") {
        s.push_str("\n- OPINIONS: notable pundit/expert opinion or fan sentiment.");
    }
    if has("predictions") {
        s.push_str("\n- PREDICTIONS: TEXT scoreline/result predictions only — most X prediction posts are graphics/cards you cannot read; skip those entirely.");
    }
    format!(
        "You are a football betting research assistant. Use web + X search for the MOST RECENT info on this ONE match (last 24-48h, plus in-play if underway). Do only a FEW focused searches. Cover ONLY these, concisely:{s}\nTEXT ONLY: ignore posts whose substance is an image/graphic (prediction cards, bet-slip screenshots, stat graphics) — you cannot read them and must NEVER guess their contents.\nIf unverified, say \"unconfirmed\" — never invent injuries or lineups.\n\nEnd with one line exactly:\nUNAVAILABLE: <semicolon-separated FULL names out/suspended for this match — ONLY names a specific recent report explicitly confirms. This line REMOVES players from the user's betting pool: when in ANY doubt (rumour, old news, unclear wording) leave the name out; \"none\" if none>"
    )
}

/// Cache key covers everything that changes the digest's CONTENT: live vs
/// pre-match (a pre-match digest must not be served mid-game and vice versa)
/// and the requested categories (different sections = a different digest).
fn cache_key(label: &str, date: &str, live: bool, categories: &[String]) -> String {
    let mut h = Sha256::new();
    h.update(b"grok|");
    h.update(MODEL.as_bytes());
    h.update(b"|");
    h.update(label.as_bytes());
    h.update(b"|");
    h.update(date.as_bytes());
    h.update(if live { b"|live" as &[u8] } else { b"|pre" });
    h.update(b"|");
    h.update(categories.join(",").as_bytes());
    format!("grok:{:x}", h.finalize())
}

/// Build the digest for the selected matches. Each match is cached independently
/// (TTL above), so only matches with no fresh cached digest hit the API. Tokens
/// + search count returned cover ONLY the fresh calls (cached reuse is free).
pub async fn fetch_digest(
    state: &AppState,
    matches: &[String],
    date: &str,
    live: bool,
    categories: &[String],
) -> Result<GrokResult, String> {
    let api_key = {
        let keys = state.keys.lock().map_err(|_| "keys lock".to_string())?;
        // In server mode the proxy holds the real key.
        match keys.grok.clone() {
            Some(k) => k,
            None if keys.proxy().is_some() => String::new(),
            None => return Err("no Grok key set".to_string()),
        }
    };
    if matches.is_empty() {
        return Err("no matches".to_string());
    }
    // Daily spend cap. Grok live-search calls are the app's most expensive per
    // unit and, unlike API-Football, had NO limiter — a heavy build day could
    // spend without bound. Cached digests still work when the cap is hit.
    const GROK_DAILY_CAP_USD: f64 = 2.0;
    {
        let conn = state.db.lock().map_err(|_| "db lock".to_string())?;
        let day_start = af::now_ts() - af::now_ts().rem_euclid(86_400);
        let spent: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0) FROM grok_usage WHERE created_at >= ?1",
                [day_start],
                |r| r.get(0),
            )
            .unwrap_or(0.0);
        if spent >= GROK_DAILY_CAP_USD {
            return Err(format!(
                "Grok daily spend cap reached (${spent:.2}/${GROK_DAILY_CAP_USD:.2}). Cached digests still work; fresh searches resume tomorrow."
            ));
        }
    }
    let system = build_system(categories);

    let mut parts: Vec<String> = Vec::new();
    let (mut t_in, mut t_out, mut t_src, mut t_cost) = (0i64, 0i64, 0i64, 0.0f64);
    let mut last_err: Option<String> = None;
    let now = af::now_ts();

    // Run the uncached matches concurrently (Grok searches are slow + unthrottled).
    let mut pending = Vec::new();
    for label in matches {
        let key = cache_key(label, date, live, categories);
        let cached = {
            let conn = state.db.lock().map_err(|_| "db lock".to_string())?;
            db::cache_get(&conn, &key, now).ok().flatten()
        };
        match cached {
            Some(txt) => parts.push(format!("### {label}\n{txt}")),
            None => pending.push((label.clone(), key)),
        }
    }
    let fetched = futures::future::join_all(
        pending
            .iter()
            .map(|(label, _)| fetch_one(state, &api_key, label, date, live, &system)),
    )
    .await;
    for ((label, key), res) in pending.iter().zip(fetched) {
        match res {
            Ok((digest, i, o, s, cost)) => {
                let ttl = if live { TTL_LIVE } else { TTL_MATCH };
                {
                    let conn = state.db.lock().map_err(|_| "db lock".to_string())?;
                    let _ = db::cache_put(&conn, key, "grok", &digest, now, ttl);
                }
                t_in += i;
                t_out += o;
                t_src += s;
                t_cost += cost;
                parts.push(format!("### {label}\n{digest}"));
            }
            Err(e) => last_err = Some(e),
        }
    }

    if parts.is_empty() {
        return Err(last_err.unwrap_or_else(|| "Grok returned nothing".to_string()));
    }
    Ok(GrokResult {
        digest: parts.join("\n\n"),
        input: t_in,
        output: t_out,
        sources: t_src,
        cost_usd: (t_cost * 10000.0).round() / 10000.0,
    })
}

/// One match → (digest, input_tokens, output_tokens, search_calls, cost_usd).
async fn fetch_one(
    state: &AppState,
    api_key: &str,
    label: &str,
    date: &str,
    live: bool,
    system: &str,
) -> Result<(String, i64, i64, i64, f64), String> {
    let recency = if live {
        " This match is ALREADY UNDERWAY — prioritise the very latest in-play news and momentum."
    } else {
        ""
    };
    let user = format!("Today is {date}. Match: {label}.{recency}");

    let body = json!({
        "model": MODEL,
        "instructions": system,
        "input": [{"role": "user", "content": user}],
        "tools": [{"type": "web_search"}, {"type": "x_search"}],
        "max_tool_calls": 4,
        "max_output_tokens": 800
    });

    let proxy = state.keys.lock().ok().and_then(|k| k.proxy());
    let endpoint = match &proxy {
        Some((base, _)) => format!("{base}/xai/v1/responses"),
        None => ENDPOINT.to_string(),
    };
    let mut req = state.http.post(&endpoint).header("content-type", "application/json");
    req = match &proxy {
        Some((_, token)) => req.header("x-proxy-token", token),
        None => req.header("Authorization", format!("Bearer {api_key}")),
    };
    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        let snippet: String = text.chars().take(300).collect();
        return Err(format!("x.ai {status}: {snippet}"));
    }
    let j: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;

    let mut content = String::new();
    if let Some(items) = j.get("output").and_then(|o| o.as_array()) {
        for it in items {
            if it.get("type").and_then(|t| t.as_str()) != Some("message") {
                continue;
            }
            if let Some(blocks) = it.get("content").and_then(|c| c.as_array()) {
                for b in blocks {
                    if b.get("type").and_then(|t| t.as_str()) == Some("output_text") {
                        if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
                            content.push_str(t);
                        }
                    }
                }
            }
        }
    }
    let content = content.trim().to_string();
    if content.is_empty() {
        let snippet: String = text.chars().take(200).collect();
        return Err(format!("Grok returned no text: {snippet}"));
    }

    let usage = j.get("usage");
    let input = usage.and_then(|u| u.get("input_tokens")).and_then(|v| v.as_i64()).unwrap_or(0);
    let output = usage.and_then(|u| u.get("output_tokens")).and_then(|v| v.as_i64()).unwrap_or(0);
    let num_sources = usage.and_then(|u| u.get("num_sources_used")).and_then(|v| v.as_i64()).unwrap_or(0);
    let num_tools = usage.and_then(|u| u.get("num_server_side_tools_used")).and_then(|v| v.as_i64()).unwrap_or(0);
    // x.ai reports the actual billed cost in "ticks" (nano-USD) — use it so our
    // meter matches the x.ai dashboard instead of estimating.
    let cost = usage
        .and_then(|u| u.get("cost_in_usd_ticks"))
        .and_then(|v| v.as_f64())
        .map(|t| t / 1_000_000_000.0)
        .unwrap_or_else(|| grok_cost(input, output, num_sources.max(num_tools)));
    Ok((content, input, output, num_sources.max(num_tools), cost))
}
