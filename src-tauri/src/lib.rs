//! powabet backend entry point. Wires up SQLite, the HTTP client, key loading,
//! and the Tauri command surface.

mod apifootball;
mod commands;
mod db;
mod features;
mod grok;
mod ingest;
mod ingeststats;
mod llm;
mod models;
mod montecarlo;
mod odds;
mod settle;
mod weather;

use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tauri::Manager;

/// App config (keys + preferences), persisted to settings.json with a
/// `.env`/env-var fallback for the keys in dev.
#[derive(Default, Serialize, Deserialize)]
pub struct Keys {
    pub api_football: Option<String>,
    pub anthropic: Option<String>,
    /// Grok (x.ai) key for X/Twitter sentiment precursor (optional).
    #[serde(default)]
    pub grok: Option<String>,
    /// OpenAI key — GPT models as a second analysis angle (optional).
    #[serde(default)]
    pub openai: Option<String>,
    /// DeepSeek key — cheap high-context model used as the default engine.
    #[serde(default)]
    pub deepseek: Option<String>,
    /// Parlay API key (parlay-api.com) — sharp odds, de-vig, +EV scanner.
    #[serde(default)]
    pub parlay: Option<String>,
    /// Selected Anthropic model id (defaults to Opus 4.8).
    #[serde(default)]
    pub model: Option<String>,
    /// Daily API-Football request budget (defaults to DEFAULT_DAILY_LIMIT).
    #[serde(default)]
    pub daily_limit: Option<i64>,
    /// Bookmaker names to line-shop for the price (empty = all). Pinnacle is
    /// always used for the sharp true probability regardless.
    #[serde(default)]
    pub books: Vec<String>,
    /// Fractional-Kelly multiplier for staking suggestions (0 = off, 0.25 = ¼).
    #[serde(default)]
    pub kelly_fraction: Option<f64>,
    /// Flat default stake to prefill the place box when Kelly is off (e.g. 0.50).
    #[serde(default)]
    pub default_stake: Option<f64>,
    /// IANA timezone for fixture times + date boundaries (default UTC-5).
    #[serde(default)]
    pub timezone: Option<String>,
    /// Server mode: route ALL external calls through this proxy URL (which holds
    /// the real keys), so this install needs no provider keys of its own.
    #[serde(default)]
    pub proxy_url: Option<String>,
    /// Per-user access token sent to the proxy (NOT a provider key).
    #[serde(default)]
    pub proxy_token: Option<String>,
    /// Browser-extension ingest endpoint: enabled, shared token, and local port.
    #[serde(default)]
    pub ingest_enabled: Option<bool>,
    #[serde(default)]
    pub ingest_token: Option<String>,
    #[serde(default)]
    pub ingest_port: Option<u16>,
}

impl Keys {
    fn load(settings_path: &PathBuf) -> Self {
        // 1. start from the persisted settings.json (user-supplied keys).
        let mut keys: Keys = std::fs::read_to_string(settings_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        // 2. fall back to env (.env in dev) for any key not already set.
        if keys.api_football.is_none() {
            keys.api_football = std::env::var("API_FOOTBALL_KEY").ok().filter(|s| !s.is_empty());
        }
        if keys.anthropic.is_none() {
            keys.anthropic = std::env::var("ANTHROPIC_API_KEY").ok().filter(|s| !s.is_empty());
        }
        if keys.grok.is_none() {
            keys.grok = std::env::var("GROK_API_KEY").ok().filter(|s| !s.is_empty());
        }
        if keys.openai.is_none() {
            keys.openai = std::env::var("OPENAI_API_KEY").ok().filter(|s| !s.is_empty());
        }
        if keys.deepseek.is_none() {
            keys.deepseek = std::env::var("DEEPSEEK_API_KEY").ok().filter(|s| !s.is_empty());
        }
        if keys.parlay.is_none() {
            keys.parlay = std::env::var("PARLAY_API_KEY").ok().filter(|s| !s.is_empty());
        }
        if keys.proxy_url.is_none() {
            keys.proxy_url = std::env::var("POWABET_PROXY_URL").ok().filter(|s| !s.is_empty());
        }
        if keys.proxy_token.is_none() {
            keys.proxy_token = std::env::var("POWABET_PROXY_TOKEN").ok().filter(|s| !s.is_empty());
        }
        keys
    }

    /// Proxy base URL (trailing slash trimmed) + token, when server mode is on.
    pub fn proxy(&self) -> Option<(String, String)> {
        let url = self.proxy_url.as_ref().filter(|s| !s.is_empty())?;
        Some((
            url.trim_end_matches('/').to_string(),
            self.proxy_token.clone().unwrap_or_default(),
        ))
    }

    pub fn persist(&self, settings_path: &PathBuf) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(settings_path, json).map_err(|e| e.to_string())?;
        // Best-effort: lock the file down on unix (it holds secrets).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(settings_path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

pub struct AppState {
    pub db: Mutex<Connection>,
    pub http: reqwest::Client,
    pub keys: Mutex<Keys>,
    pub settings_path: PathBuf,
    /// Timestamp of the last fresh network call — used to space out requests so
    /// the free-tier per-minute rate limit isn't tripped.
    pub throttle: tokio::sync::Mutex<std::time::Instant>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Load .env (dev convenience). Harmless if absent.
    let _ = dotenvy::dotenv();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("no app data dir: {e}"))?;
            std::fs::create_dir_all(&data_dir)?;

            let db_path = data_dir.join("powabet.db");
            let settings_path = data_dir.join("settings.json");

            let conn = db::open(&db_path).map_err(std::io::Error::other)?;
            let mut keys = Keys::load(&settings_path);

            // Ensure an ingest token exists (for the browser extension), then start
            // the local ingest server unless the user disabled it.
            if keys.ingest_token.is_none() {
                keys.ingest_token = Some(ingest::gen_token());
                let _ = keys.persist(&settings_path);
            }
            if keys.ingest_enabled.unwrap_or(true) {
                ingest::start(
                    db_path.clone(),
                    keys.ingest_token.clone().unwrap_or_default(),
                    keys.ingest_port.unwrap_or(8765),
                );
            }

            let http = reqwest::Client::builder()
                // Generous: a thinking-capable model (Sonnet 5 / Opus) on a big
                // Scout prompt can take a while; a short timeout surfaces as the
                // confusing "error sending request for url".
                .timeout(std::time::Duration::from_secs(300))
                .user_agent("powabet/0.1")
                .build()
                .map_err(std::io::Error::other)?;

            app.manage(AppState {
                db: Mutex::new(conn),
                http,
                keys: Mutex::new(keys),
                settings_path,
                // Start in the past so the first request isn't delayed. Instant
                // is boot-relative — a bare subtraction panics if the machine
                // booted <60s ago.
                throttle: tokio::sync::Mutex::new(
                    std::time::Instant::now()
                        .checked_sub(std::time::Duration::from_secs(60))
                        .unwrap_or_else(std::time::Instant::now),
                ),
            });

            // CLOSING-LINE snapshot loop: every 10 minutes, check open bets'
            // fixtures — if one is within its kickoff window, snapshot the odds
            // (once) so CLV later compares against a TRUE closing line instead
            // of the post-match approximation. Best-effort; only runs while the
            // app is open, which is exactly when it costs nothing to have.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(600)).await;
                    let state = handle.state::<AppState>();
                    let _ = commands::closing_snapshot_tick(&state).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::get_meter,
            commands::fetch_leagues,
            commands::bump_leagues,
            commands::fetch_fixtures,
            commands::fetch_squads,
            commands::build_tickets,
            commands::get_picks,
            commands::build_ladder,
            commands::prewarm_plausibility,
            commands::evaluate_tickets,
            commands::settle_generated,
            commands::generated_report,
            commands::darwin_sweep,
            commands::generated_report_by_kind,
            commands::generated_report_by_market,
            commands::export_data,
            commands::import_data,
            commands::reset_data,
            commands::live_fixtures,
            commands::live_snapshot,
            commands::live_ticket,
            commands::price_sgp,
            commands::get_bankers,
            commands::usage_by_purpose,
            commands::export_extension,
            commands::ingest_info,
            commands::list_ingested,
            commands::process_ingested,
            commands::delete_ingested,
            commands::update_ingest_note,
            commands::assign_ingest_fixture,
            commands::save_ticket,
            commands::list_tickets,
            commands::list_grok_log,
            commands::inspect_fixtures,
            commands::inspect_player,
            commands::inspect_team_stats,
            commands::usage_breakdown,
            commands::get_bankroll,
            commands::set_bankroll,
            commands::calibration,
            commands::place_bet,
            commands::list_bets,
            commands::delete_bet,
            commands::settle_bet,
            commands::settle_all,
            commands::set_bet_odds,
        ])
        .run(tauri::generate_context!())
        .expect("error while running powabet");
}
