//! Match-day weather via open-meteo (free, no key). We geocode the venue city to
//! coordinates, then read the hourly forecast nearest kickoff. Cached in the DB
//! and NOT counted against the API-Football request meter (different provider).
//!
//! Weather is fed to the model as SOFT context (per the deterministic-numbers /
//! LLM-synthesis split) — we don't fabricate a probability from it.

use serde_json::Value;

use crate::apifootball as af;
use crate::{db, AppState};

const GEO_TTL: i64 = 30 * 24 * 3600; // city → coords is effectively static
const FC_TTL: i64 = 3 * 3600; // forecast refresh

/// WMO weather code → a plain condition word.
fn condition(code: f64) -> &'static str {
    match code as i64 {
        0 => "Clear/sunny",
        1 | 2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        51..=57 => "Drizzle",
        61..=67 => "Rain",
        71..=77 => "Snow",
        80..=82 => "Rain showers",
        85 | 86 => "Snow showers",
        95..=99 => "Thunderstorm",
        _ => "Cloudy",
    }
}

/// Geocode a city name to (lat, lon) via open-meteo's geocoder. Cached.
async fn geocode(state: &AppState, city: &str) -> Option<(f64, f64)> {
    let now = af::now_ts();
    let key = format!("geo:{}", city.to_lowercase());
    if let Some(s) = {
        let conn = state.db.lock().ok()?;
        db::cache_get(&conn, &key, now).ok().flatten()
    } {
        let mut it = s.split(',');
        let lat = it.next()?.parse().ok()?;
        let lon = it.next()?.parse().ok()?;
        return Some((lat, lon));
    }
    let resp = state
        .http
        .get("https://geocoding-api.open-meteo.com/v1/search")
        .query(&[("name", city), ("count", "1"), ("language", "en")])
        .send()
        .await
        .ok()?;
    let j: Value = resp.json().await.ok()?;
    let r = j.get("results")?.as_array()?.first()?;
    let lat = r.get("latitude")?.as_f64()?;
    let lon = r.get("longitude")?.as_f64()?;
    if let Ok(conn) = state.db.lock() {
        let _ = db::cache_put(&conn, &key, "geo", &format!("{lat},{lon}"), now, GEO_TTL);
    }
    Some((lat, lon))
}

/// One-line weather summary for a fixture, or None (past match / too far out /
/// no city / lookup failed).
pub async fn match_weather(state: &AppState, city: &str, date_utc: &str) -> Option<String> {
    let dt = chrono::DateTime::parse_from_rfc3339(date_utc).ok()?;
    let utc = dt.with_timezone(&chrono::Utc);
    let now = af::now_ts();
    let ts = utc.timestamp();
    // Skip already-played matches and anything beyond the forecast horizon (~16d).
    if ts < now - 86_400 || ts > now + 15 * 86_400 {
        return None;
    }
    let day = utc.format("%Y-%m-%d").to_string();
    let hour_str = utc.format("%Y-%m-%dT%H:00").to_string();

    let (lat, lon) = geocode(state, city).await?;
    let key = format!("fc:{lat:.2},{lon:.2}:{hour_str}");
    if let Some(s) = {
        let conn = state.db.lock().ok()?;
        db::cache_get(&conn, &key, now).ok().flatten()
    } {
        return Some(s);
    }

    let resp = state
        .http
        .get("https://api.open-meteo.com/v1/forecast")
        .query(&[
            ("latitude", lat.to_string()),
            ("longitude", lon.to_string()),
            (
                "hourly",
                "temperature_2m,precipitation,precipitation_probability,wind_speed_10m,weather_code".to_string(),
            ),
            ("timezone", "GMT".to_string()),
            ("start_date", day.clone()),
            ("end_date", day),
        ])
        .send()
        .await
        .ok()?;
    let j: Value = resp.json().await.ok()?;
    let hourly = j.get("hourly")?;
    let times = hourly.get("time")?.as_array()?;
    let idx = times.iter().position(|t| t.as_str() == Some(hour_str.as_str()))?;
    let at = |k: &str| hourly.get(k).and_then(|a| a.as_array()).and_then(|a| a.get(idx)).and_then(|v| v.as_f64());

    let mut parts = Vec::new();
    if let Some(code) = at("weather_code") {
        parts.push(condition(code).to_string());
    }
    if let Some(t) = at("temperature_2m") {
        parts.push(format!("{t:.0}°C"));
    }
    match at("precipitation_probability") {
        Some(p) => parts.push(format!("{p:.0}% rain")),
        None => {
            if let Some(p) = at("precipitation") {
                parts.push(if p >= 0.2 { format!("rain {p:.1}mm") } else { "dry".to_string() });
            }
        }
    }
    if let Some(w) = at("wind_speed_10m") {
        let tag = if w >= 30.0 { " (strong)" } else { "" };
        parts.push(format!("wind {w:.0} km/h{tag}"));
    }
    if parts.is_empty() {
        return None;
    }
    let summary = parts.join(", ");
    if let Ok(conn) = state.db.lock() {
        let _ = db::cache_put(&conn, &key, "fc", &summary, now, FC_TTL);
    }
    Some(summary)
}
