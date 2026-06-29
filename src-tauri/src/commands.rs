//! Tauri commands — the only surface the frontend can reach. The UI never calls
//! an external API; it invokes these. Orchestrates cache-first fetches, the
//! deterministic feature engine, and the single cached model call.

use std::collections::{HashMap, HashSet};

use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::State;

use crate::apifootball::{self as af, response_array};
use crate::features::{self, FixtureCtx};
use crate::llm;
use crate::models::*;
use crate::settle;
use crate::{db, AppState};

// ---------- settings / meter ----------

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Result<SettingsView, String> {
    let (has_af, has_anthropic, has_grok, has_openai, has_parlay, model, limit, books, kelly, default_stake, timezone, proxy_url, has_proxy_token) = {
        let keys = state.keys.lock().map_err(|_| "keys lock")?;
        (
            keys.api_football.is_some(),
            keys.anthropic.is_some(),
            keys.grok.is_some(),
            keys.openai.is_some(),
            keys.parlay.is_some(),
            keys.model.clone().unwrap_or_else(|| llm::DEFAULT_MODEL.to_string()),
            keys.daily_limit.unwrap_or(db::DEFAULT_DAILY_LIMIT),
            keys.books.clone(),
            keys.kelly_fraction.unwrap_or(0.0), // off by default — flat staking is safer
            keys.default_stake.unwrap_or(0.50),
            keys.timezone.clone().unwrap_or_else(|| "Etc/GMT+5".to_string()),
            keys.proxy_url.clone().unwrap_or_default(),
            keys.proxy_token.is_some(),
        )
    };
    let (meter, by_model) = {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        (db::meter(&conn, &af::today(), limit)?, db::usage_by_model(&conn)?)
    };
    let mut input = 0i64;
    let mut output = 0i64;
    let mut cost = 0.0;
    for (m, i, o) in by_model {
        input += i;
        output += o;
        cost += llm::cost_usd(&m, i, o);
    }
    Ok(SettingsView {
        has_api_football_key: has_af,
        has_anthropic_key: has_anthropic,
        has_grok_key: has_grok,
        has_openai_key: has_openai,
        has_parlay_key: has_parlay,
        model,
        books,
        kelly_fraction: kelly,
        default_stake,
        timezone,
        proxy_url,
        has_proxy_token,
        meter,
        usage: UsageTotal {
            input_tokens: input,
            output_tokens: output,
            cost_usd: (cost * 10000.0).round() / 10000.0,
        },
    })
}

#[tauri::command]
pub fn save_settings(
    state: State<AppState>,
    api_football_key: Option<String>,
    anthropic_key: Option<String>,
    grok_key: Option<String>,
    openai_key: Option<String>,
    parlay_key: Option<String>,
    model: Option<String>,
    daily_limit: Option<i64>,
    books: Option<Vec<String>>,
    kelly_fraction: Option<f64>,
    default_stake: Option<f64>,
    timezone: Option<String>,
    proxy_url: Option<String>,
    proxy_token: Option<String>,
    ingest_enabled: Option<bool>,
) -> Result<SettingsView, String> {
    {
        let mut keys = state.keys.lock().map_err(|_| "keys lock")?;
        if let Some(b) = books {
            keys.books = b;
        }
        if let Some(k) = kelly_fraction {
            keys.kelly_fraction = Some(k.clamp(0.0, 1.0));
        }
        if let Some(d) = default_stake {
            keys.default_stake = Some(d.max(0.0));
        }
        if let Some(tz) = timezone {
            if !tz.trim().is_empty() {
                keys.timezone = Some(tz);
            }
        }
        if let Some(k) = api_football_key {
            keys.api_football = non_empty(k);
        }
        if let Some(k) = anthropic_key {
            keys.anthropic = non_empty(k);
        }
        if let Some(k) = grok_key {
            keys.grok = non_empty(k);
        }
        if let Some(k) = openai_key {
            keys.openai = non_empty(k);
        }
        if let Some(k) = parlay_key {
            keys.parlay = non_empty(k);
        }
        if let Some(u) = proxy_url {
            keys.proxy_url = non_empty(u);
        }
        if let Some(t) = proxy_token {
            keys.proxy_token = non_empty(t);
        }
        if let Some(e) = ingest_enabled {
            keys.ingest_enabled = Some(e);
        }
        if let Some(m) = model {
            if llm::is_allowed_model(&m) {
                keys.model = Some(m);
            }
        }
        if let Some(d) = daily_limit {
            if d > 0 {
                keys.daily_limit = Some(d);
            }
        }
        keys.persist(&state.settings_path)?;
    }
    get_settings(state)
}

#[tauri::command]
pub fn get_meter(state: State<AppState>) -> Result<RequestMeter, String> {
    let limit = {
        let keys = state.keys.lock().map_err(|_| "keys lock")?;
        keys.daily_limit.unwrap_or(db::DEFAULT_DAILY_LIMIT)
    };
    let conn = state.db.lock().map_err(|_| "db lock")?;
    db::meter(&conn, &af::today(), limit)
}

fn non_empty(s: String) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

// ---------- leagues ----------

/// All leagues in one cached request, sorted by how often the user has picked
/// them (most-picked first), then by a popularity tiebreak, then name.
#[tauri::command]
pub async fn fetch_leagues(state: State<'_, AppState>) -> Result<Vec<LeagueOption>, String> {
    let json = af::cached_get(&state, "/leagues", vec![], af::TTL_LEAGUES).await?;

    let counts = {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        db::league_pick_counts(&conn)?
    };

    let mut out: Vec<LeagueOption> = Vec::new();
    for item in response_array(&json) {
        let league = match item.get("league") {
            Some(l) => l,
            None => continue,
        };
        let id = match league.get("id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => continue,
        };
        out.push(LeagueOption {
            id,
            name: league.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            country: item
                .get("country")
                .and_then(|c| c.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            picks: counts.get(&id).copied().unwrap_or(0),
        });
    }

    out.sort_by(|a, b| {
        b.picks
            .cmp(&a.picks)
            .then(popularity_rank(a.id).cmp(&popularity_rank(b.id)))
            .then(a.name.cmp(&b.name))
    });
    Ok(out)
}

/// Increment the pick counter for each league the user selected.
#[tauri::command]
pub fn bump_leagues(state: State<AppState>, ids: Vec<i64>) -> Result<(), String> {
    let conn = state.db.lock().map_err(|_| "db lock")?;
    for id in ids {
        db::bump_league(&conn, id)?;
    }
    Ok(())
}

/// Lower rank = more prominent. Used only as a tiebreak before the user has
/// built up their own pick history.
fn popularity_rank(id: i64) -> i32 {
    match id {
        1 => 0,    // World Cup
        2 => 1,    // Champions League
        3 => 2,    // Europa League
        39 => 3,   // Premier League
        140 => 4,  // La Liga
        135 => 5,  // Serie A
        78 => 6,   // Bundesliga
        61 => 7,   // Ligue 1
        40 => 8,   // Championship
        88 => 9,   // Eredivisie
        94 => 10,  // Primeira Liga
        253 => 11, // MLS
        4 => 12,   // Euro Championship
        9 => 13,   // Copa America
        _ => 1000,
    }
}

// ---------- fixtures ----------

#[tauri::command]
pub async fn fetch_fixtures(
    state: State<'_, AppState>,
    date: String,
    league: Option<i64>,
    season: Option<i64>,
    timezone: Option<String>,
) -> Result<Vec<Fixture>, String> {
    let mut params: Vec<(&str, String)> = vec![("date", date.clone())];
    // Align the date boundary to the user's local day (API defaults to UTC).
    if let Some(tz) = timezone.filter(|t| !t.is_empty()) {
        params.push(("timezone", tz));
    }
    if let Some(l) = league {
        params.push(("league", l.to_string()));
    }
    if let Some(s) = season {
        params.push(("season", s.to_string()));
    }
    let json = af::cached_get(&state, "/fixtures", params, af::TTL_FIXTURES).await?;

    let mut out = Vec::new();
    for item in response_array(&json) {
        if let Some(f) = parse_fixture(&item) {
            out.push(f);
        }
    }
    out.sort_by(|a, b| a.date_utc.cmp(&b.date_utc));
    Ok(out)
}

fn parse_fixture(item: &Value) -> Option<Fixture> {
    let fixture = item.get("fixture")?;
    let league = item.get("league")?;
    let teams = item.get("teams")?;
    Some(Fixture {
        fixture_id: fixture.get("id")?.as_i64()?,
        league_id: league.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
        league_name: league
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        season: league.get("season").and_then(|v| v.as_i64()).unwrap_or(0),
        date_utc: fixture
            .get("date")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        home_team_id: teams.get("home").and_then(|t| t.get("id")).and_then(|v| v.as_i64()).unwrap_or(0),
        home_team: teams
            .get("home")
            .and_then(|t| t.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        away_team_id: teams.get("away").and_then(|t| t.get("id")).and_then(|v| v.as_i64()).unwrap_or(0),
        away_team: teams
            .get("away")
            .and_then(|t| t.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        status: fixture
            .get("status")
            .and_then(|s| s.get("short"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        venue_city: fixture
            .get("venue")
            .and_then(|v| v.get("city"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        venue_name: fixture
            .get("venue")
            .and_then(|v| v.get("name"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
        referee: fixture
            .get("referee")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string()),
    })
}

// ---------- squads (player chips) ----------

#[tauri::command]
pub async fn fetch_squads(
    state: State<'_, AppState>,
    fixtures: Vec<FixtureInput>,
) -> Result<Vec<TeamSquad>, String> {
    let mut out = Vec::new();
    for fx in &fixtures {
        // Injuries once per fixture (cached) — badges the chips.
        let injuries = fetch_injury_map(&state, fx.fixture_id).await.unwrap_or_default();

        for (team_id, team_name) in [
            (fx.home_team_id, fx.home_team.clone()),
            (fx.away_team_id, fx.away_team.clone()),
        ] {
            let players = fetch_team_squad(&state, team_id, &team_name, fx.fixture_id, &injuries).await?;
            out.push(TeamSquad {
                team_id,
                team_name,
                fixture_id: fx.fixture_id,
                players,
            });
        }
    }
    Ok(out)
}

async fn fetch_team_squad(
    state: &AppState,
    team_id: i64,
    team_name: &str,
    fixture_id: i64,
    injuries: &HashMap<i64, String>,
) -> Result<Vec<SquadPlayer>, String> {
    let json = af::cached_get(
        state,
        "/players/squads",
        vec![("team", team_id.to_string())],
        af::TTL_SQUADS,
    )
    .await?;

    let mut players = Vec::new();
    if let Some(entry) = response_array(&json).first() {
        if let Some(arr) = entry.get("players").and_then(|p| p.as_array()) {
            for p in arr {
                let pid = match p.get("id").and_then(|v| v.as_i64()) {
                    Some(id) => id,
                    None => continue,
                };
                players.push(SquadPlayer {
                    player_id: pid,
                    name: p.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown").to_string(),
                    position: p.get("position").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    team_id,
                    team_name: team_name.to_string(),
                    availability: injuries.get(&pid).cloned().unwrap_or_else(|| "unknown".to_string()),
                });
            }
        }
    }
    let _ = fixture_id;
    Ok(players)
}

/// In-play state for a fixture (fetched fresh once kickoff has elapsed).
struct LiveState {
    status: String, // short code: 1H, HT, 2H, ET, P, FT, AET, PEN, …
    elapsed: i64,
    home_goals: i64,
    away_goals: i64,
}

impl LiveState {
    fn is_live(&self) -> bool {
        matches!(self.status.as_str(), "1H" | "HT" | "2H" | "ET" | "BT" | "P" | "INT" | "LIVE")
    }
    fn is_finished(&self) -> bool {
        matches!(self.status.as_str(), "FT" | "AET" | "PEN")
    }
}

/// Has kickoff already passed? (RFC3339 string vs now.)
fn kickoff_elapsed(date_utc: &Option<String>) -> bool {
    date_utc
        .as_deref()
        .and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok())
        .map(|d| d.timestamp() <= af::now_ts())
        .unwrap_or(false)
}

/// Fresh live state from `/fixtures?id=` (short TTL). None if unavailable.
async fn fetch_live_state(state: &AppState, fixture_id: i64) -> Option<LiveState> {
    let json = af::cached_get(state, "/fixtures", vec![("id", fixture_id.to_string())], af::TTL_LIVE)
        .await
        .ok()?;
    let item = response_array(&json).into_iter().next()?;
    let status = item
        .get("fixture")
        .and_then(|f| f.get("status"))
        .and_then(|s| s.get("short"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let elapsed = item
        .get("fixture")
        .and_then(|f| f.get("status"))
        .and_then(|s| s.get("elapsed"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let goals = item.get("goals");
    let home_goals = goals.and_then(|g| g.get("home")).and_then(|v| v.as_i64()).unwrap_or(0);
    let away_goals = goals.and_then(|g| g.get("away")).and_then(|v| v.as_i64()).unwrap_or(0);
    Some(LiveState { status, elapsed, home_goals, away_goals })
}

/// Confirmed starting XI per team for a fixture (player ids), if lineups are
/// posted yet. Empty map → not available (use season minutes as before).
async fn fetch_starters(state: &AppState, fixture_id: i64) -> HashMap<i64, HashSet<i64>> {
    let mut out: HashMap<i64, HashSet<i64>> = HashMap::new();
    let json = match af::cached_get(
        state,
        "/fixtures/lineups",
        vec![("fixture", fixture_id.to_string())],
        af::TTL_LINEUPS,
    )
    .await
    {
        Ok(j) => j,
        Err(_) => return out,
    };
    for team in response_array(&json) {
        let team_id = match team.get("team").and_then(|t| t.get("id")).and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => continue,
        };
        let mut starters = HashSet::new();
        if let Some(xi) = team.get("startXI").and_then(|x| x.as_array()) {
            for p in xi {
                if let Some(pid) = p.get("player").and_then(|x| x.get("id")).and_then(|v| v.as_i64()) {
                    starters.insert(pid);
                }
            }
        }
        if !starters.is_empty() {
            out.insert(team_id, starters);
        }
    }
    out
}

async fn fetch_injury_map(state: &AppState, fixture_id: i64) -> Result<HashMap<i64, String>, String> {
    let json = af::cached_get(
        state,
        "/injuries",
        vec![("fixture", fixture_id.to_string())],
        af::TTL_INJURIES,
    )
    .await?;
    Ok(injury_map_from(&json))
}

fn injury_map_from(json: &Value) -> HashMap<i64, String> {
    let mut map = HashMap::new();
    for item in response_array(json) {
        if let Some(pid) = item.get("player").and_then(|p| p.get("id")).and_then(|v| v.as_i64()) {
            let kind = item
                .get("player")
                .and_then(|p| p.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let reason = item
                .get("player")
                .and_then(|p| p.get("reason"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            map.insert(pid, map_availability(kind, reason));
        }
    }
    map
}

async fn fetch_team_stats(
    state: &AppState,
    team_id: i64,
    name: &str,
    league_id: i64,
    season: i64,
) -> Result<Option<crate::features::TeamStats>, String> {
    let json = af::cached_get(
        state,
        "/teams/statistics",
        vec![
            ("team", team_id.to_string()),
            ("league", league_id.to_string()),
            ("season", season.to_string()),
        ],
        af::TTL_TEAMS,
    )
    .await?;
    Ok(features::parse_team_stats(&json, name))
}

#[derive(Default)]
struct TeamForm {
    xg_for: Option<f64>,
    xg_against: Option<f64>,
    corners_for: Option<f64>,
    corners_against: Option<f64>,
    shots_for: Option<f64>,
    shots_against: Option<f64>,
    outbox_for: Option<f64>,
    outbox_against: Option<f64>,
    inbox_for: Option<f64>,
    inbox_against: Option<f64>,
    offsides_for: Option<f64>,
    offsides_against: Option<f64>,
}

fn stat_val(stats: &Value, ty: &str) -> Option<f64> {
    stats
        .get("statistics")
        .and_then(|s| s.as_array())
        .and_then(|arr| arr.iter().find(|s| s.get("type").and_then(|t| t.as_str()) == Some(ty)))
        .and_then(|s| s.get("value"))
        .and_then(|v| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()).or_else(|| v.as_f64()))
}

/// Recent-form averages (xG, corners, shots — for & against) from a team's last
/// finished fixtures via `/fixtures/statistics`. Cached. None if too few games.
async fn fetch_team_form(state: &AppState, team_id: i64) -> Option<TeamForm> {
    let lj = af::cached_get(
        state,
        "/fixtures",
        vec![("team", team_id.to_string()), ("last", "8".to_string())],
        af::TTL_ODDS,
    )
    .await
    .ok()?;
    // (sum, n) per metric (for & against interleaved).
    let metrics = ["expected_goals", "Corner Kicks", "Total Shots", "Shots outsidebox", "Shots insidebox", "Offsides"];
    let mut acc: [(f64, f64); 12] = [(0.0, 0.0); 12];
    for f in response_array(&lj) {
        let short = f.get("fixture").and_then(|x| x.get("status")).and_then(|s| s.get("short")).and_then(|v| v.as_str()).unwrap_or("");
        if !matches!(short, "FT" | "AET" | "PEN") {
            continue;
        }
        let fid = match f.get("fixture").and_then(|x| x.get("id")).and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => continue,
        };
        let sj = match af::cached_get(
            state,
            "/fixtures/statistics",
            vec![("fixture", fid.to_string())],
            af::TTL_INJURIES * 28,
        )
        .await
        {
            Ok(j) => j,
            Err(_) => continue,
        };
        let arr = response_array(&sj);
        let me = arr.iter().find(|t| t.get("team").and_then(|x| x.get("id")).and_then(|v| v.as_i64()) == Some(team_id));
        let opp = arr.iter().find(|t| t.get("team").and_then(|x| x.get("id")).and_then(|v| v.as_i64()) != Some(team_id));
        if let (Some(me), Some(opp)) = (me, opp) {
            for (i, ty) in metrics.iter().enumerate() {
                if let Some(v) = stat_val(me, ty) {
                    acc[i * 2].0 += v;
                    acc[i * 2].1 += 1.0;
                }
                if let Some(v) = stat_val(opp, ty) {
                    acc[i * 2 + 1].0 += v;
                    acc[i * 2 + 1].1 += 1.0;
                }
            }
        }
    }
    let avg = |s: (f64, f64)| if s.1 >= 2.0 { Some(s.0 / s.1) } else { None };
    let form = TeamForm {
        xg_for: avg(acc[0]),
        xg_against: avg(acc[1]),
        corners_for: avg(acc[2]),
        corners_against: avg(acc[3]),
        shots_for: avg(acc[4]),
        shots_against: avg(acc[5]),
        outbox_for: avg(acc[6]),
        outbox_against: avg(acc[7]),
        inbox_for: avg(acc[8]),
        inbox_against: avg(acc[9]),
        offsides_for: avg(acc[10]),
        offsides_against: avg(acc[11]),
    };
    if form.xg_for.is_none() && form.corners_for.is_none() && form.shots_for.is_none() && form.outbox_for.is_none() {
        return None;
    }
    Some(form)
}

/// Current coach name for a team (API-Football /coachs). Cached.
async fn fetch_coach(state: &AppState, team_id: i64) -> Option<String> {
    let j = af::cached_get(state, "/coachs", vec![("team", team_id.to_string())], af::TTL_TEAMS).await.ok()?;
    response_array(&j)
        .first()?
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Folded names of this league's top scorers + top assisters — an "in-form"
/// signal we flag on the matching scorer/assist candidates. Two cached calls
/// per league/season (free on rebuilds).
async fn fetch_inform(state: &AppState, league: i64, season: i64) -> HashSet<String> {
    let mut set = HashSet::new();
    for ep in ["/players/topscorers", "/players/topassists"] {
        if let Ok(j) = af::cached_get(
            state,
            ep,
            vec![("league", league.to_string()), ("season", season.to_string())],
            af::TTL_PLAYERS,
        )
        .await
        {
            for item in response_array(&j) {
                if let Some(n) = item.get("player").and_then(|p| p.get("name")).and_then(|v| v.as_str()) {
                    set.insert(crate::odds::fold(n));
                }
            }
        }
    }
    set
}

/// Per-player CONSISTENCY (recent hit-rates) for a team — fetched from the team's
/// recent fixtures' /fixtures/players (cached, shared across all the team's
/// players). Keyed by folded player name. ~7 cached calls per team.
async fn fetch_consistency(
    state: &AppState,
    team_id: i64,
    _season: i64,
) -> HashMap<String, features::Consistency> {
    let mut acc: HashMap<String, (u32, [u32; 9])> = HashMap::new();
    let lj = match af::cached_get(
        state,
        "/fixtures",
        vec![("team", team_id.to_string()), ("last", "6".to_string())],
        af::TTL_ODDS,
    )
    .await
    {
        Ok(j) => j,
        Err(_) => return HashMap::new(),
    };
    for f in response_array(&lj) {
        let short = f.get("fixture").and_then(|x| x.get("status")).and_then(|s| s.get("short")).and_then(|v| v.as_str()).unwrap_or("");
        if !matches!(short, "FT" | "AET" | "PEN") {
            continue;
        }
        let fid = match f.get("fixture").and_then(|x| x.get("id")).and_then(|v| v.as_i64()) {
            Some(i) => i,
            None => continue,
        };
        let pj = match af::cached_get(state, "/fixtures/players", vec![("fixture", fid.to_string())], af::TTL_PLAYERS * 7).await {
            Ok(j) => j,
            Err(_) => continue,
        };
        for team in response_array(&pj) {
            if team.get("team").and_then(|t| t.get("id")).and_then(|v| v.as_i64()) != Some(team_id) {
                continue;
            }
            let players = match team.get("players").and_then(|p| p.as_array()) {
                Some(p) => p,
                None => continue,
            };
            for p in players {
                let name = p.get("player").and_then(|x| x.get("name")).and_then(|v| v.as_str()).unwrap_or("");
                if name.is_empty() {
                    continue;
                }
                let st = match p.get("statistics").and_then(|x| x.as_array()).and_then(|a| a.first()) {
                    Some(s) => s,
                    None => continue,
                };
                let mins = st.get("games").and_then(|g| g.get("minutes")).and_then(|v| v.as_f64()).unwrap_or(0.0);
                if mins <= 0.0 {
                    continue; // didn't actually play → don't count the appearance
                }
                let g = |a: &str, b: &str| st.get(a).and_then(|x| x.get(b)).and_then(|v| v.as_f64()).unwrap_or(0.0);
                let (cards, shots, sot) = (g("cards", "yellow") + g("cards", "red"), g("shots", "total"), g("shots", "on"));
                let (goals, assists, tackles, fouls) = (g("goals", "total"), g("goals", "assists"), g("tackles", "total"), g("fouls", "committed"));
                let e = acc.entry(crate::odds::fold(name)).or_insert((0, [0u32; 9]));
                e.0 += 1;
                let hit = |c: bool, slot: &mut u32| if c { *slot += 1 };
                hit(cards >= 1.0, &mut e.1[0]);
                hit(shots >= 1.0, &mut e.1[1]);
                hit(shots >= 2.0, &mut e.1[2]);
                hit(sot >= 1.0, &mut e.1[3]);
                hit(sot >= 2.0, &mut e.1[4]);
                hit(goals >= 1.0, &mut e.1[5]);
                hit(assists >= 1.0, &mut e.1[6]);
                hit(tackles >= 2.0, &mut e.1[7]);
                hit(fouls >= 1.0, &mut e.1[8]);
            }
        }
    }
    acc.into_iter()
        .map(|(name, (apps, h))| {
            let r = |i: usize| if apps > 0 { h[i] as f64 / apps as f64 } else { 0.0 };
            (
                name,
                features::Consistency {
                    apps,
                    card_rate: r(0),
                    shot1_rate: r(1),
                    shot2_rate: r(2),
                    sot1_rate: r(3),
                    sot2_rate: r(4),
                    goal_rate: r(5),
                    assist_rate: r(6),
                    tackle2_rate: r(7),
                    foul1_rate: r(8),
                },
            )
        })
        .collect()
}

/// Team card model from recent fixtures (cached /fixtures/players, shared with
/// `fetch_consistency`): (cards_for_avg, cards_against_avg, both-carded rate,
/// most-cards rate). None if too few games.
async fn fetch_team_cards(
    state: &AppState,
    team_id: i64,
    _season: i64,
) -> (Option<f64>, Option<f64>, Option<f64>, Option<f64>) {
    let lj = match af::cached_get(
        state,
        "/fixtures",
        vec![("team", team_id.to_string()), ("last", "6".to_string())],
        af::TTL_ODDS,
    )
    .await
    {
        Ok(j) => j,
        Err(_) => return (None, None, None, None),
    };
    let (mut n, mut cf, mut ca, mut both, mut most) = (0u32, 0.0f64, 0.0f64, 0u32, 0u32);
    for f in response_array(&lj) {
        let short = f.get("fixture").and_then(|x| x.get("status")).and_then(|s| s.get("short")).and_then(|v| v.as_str()).unwrap_or("");
        if !matches!(short, "FT" | "AET" | "PEN") {
            continue;
        }
        let fid = match f.get("fixture").and_then(|x| x.get("id")).and_then(|v| v.as_i64()) {
            Some(i) => i,
            None => continue,
        };
        let pj = match af::cached_get(state, "/fixtures/players", vec![("fixture", fid.to_string())], af::TTL_PLAYERS * 7).await {
            Ok(j) => j,
            Err(_) => continue,
        };
        let (mut our, mut opp) = (0.0f64, 0.0f64);
        for team in response_array(&pj) {
            let is_us = team.get("team").and_then(|t| t.get("id")).and_then(|v| v.as_i64()) == Some(team_id);
            let sum: f64 = team
                .get("players")
                .and_then(|p| p.as_array())
                .map(|players| {
                    players
                        .iter()
                        .map(|p| {
                            let st = p.get("statistics").and_then(|x| x.as_array()).and_then(|a| a.first());
                            let g = |b: &str| st.and_then(|s| s.get("cards")).and_then(|c| c.get(b)).and_then(|v| v.as_f64()).unwrap_or(0.0);
                            g("yellow") + g("red")
                        })
                        .sum()
                })
                .unwrap_or(0.0);
            if is_us {
                our += sum;
            } else {
                opp += sum;
            }
        }
        n += 1;
        cf += our;
        ca += opp;
        if our >= 1.0 && opp >= 1.0 {
            both += 1;
        }
        if our > opp {
            most += 1;
        }
    }
    if n < 2 {
        return (None, None, None, None);
    }
    let nf = n as f64;
    (Some(cf / nf), Some(ca / nf), Some(both as f64 / nf), Some(most as f64 / nf))
}

/// Estimate a referee's strictness (avg cards/game) via a cheap cached Haiku call.
/// There's no referee-stats endpoint, so this is a MODEL ESTIMATE (flagged as
/// such on the picks) — used to nudge card markets, not presented as measured.
async fn fetch_ref_strictness(state: &AppState, referee: &str) -> Option<f64> {
    let key = format!("ref-cards-v1:{}", crate::odds::fold(referee));
    if let Ok(conn) = state.db.lock() {
        if let Ok(Some(v)) = db::ai_get(&conn, &key) {
            return v.parse::<f64>().ok();
        }
    }
    let model = "claude-haiku-4-5";
    let sys = "You estimate a football referee's disciplinary strictness. Reply with ONLY a single number: the average total cards (yellows + reds) this referee shows per match. If you don't know the referee, reply 4.";
    let user = format!("Referee: {referee}. Average cards per match?");
    let (txt, gin, gout) = llm::chat_call(state, model, sys, &user, 20).await.ok()?;
    let num: f64 = txt.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect::<String>().parse().ok()?;
    let num = num.clamp(2.0, 7.5);
    if let Ok(conn) = state.db.lock() {
        let _ = db::usage_add(&conn, af::now_ts(), model, gin, gout, "referee");
        let _ = db::ai_put(&conn, &key, &num.to_string(), model, af::now_ts());
    }
    Some(num)
}

/// Formation per team from the (cached) lineups response.
async fn fetch_formations(state: &AppState, fixture_id: i64) -> HashMap<i64, String> {
    let mut out = HashMap::new();
    if let Ok(j) = af::cached_get(state, "/fixtures/lineups", vec![("fixture", fixture_id.to_string())], af::TTL_LINEUPS).await {
        for team in response_array(&j) {
            if let (Some(tid), Some(f)) = (
                team.get("team").and_then(|t| t.get("id")).and_then(|v| v.as_i64()),
                team.get("formation").and_then(|v| v.as_str()).filter(|s| !s.is_empty()),
            ) {
                out.insert(tid, f.to_string());
            }
        }
    }
    out
}

/// The short style tag from a raw tactics reply ("STYLE: low-block\n...").
fn parse_style(raw: &str) -> (String, String) {
    let mut lines = raw.lines();
    let first = lines.next().unwrap_or("");
    if let Some(pos) = first.to_uppercase().find("STYLE:") {
        let tag = first[pos + 6..].trim().trim_matches(|c: char| c == '*' || c == '.' || c == '—' || c == '-').trim().to_string();
        let profile = lines.collect::<Vec<_>>().join(" ").trim().to_string();
        let profile = if profile.is_empty() { raw.trim().to_string() } else { profile };
        return (tag, profile);
    }
    (String::new(), raw.trim().to_string())
}

/// Cached short style tag for a team (board/ladder lookup — no model call).
async fn cached_tactics_tag(state: &AppState, team: &str) -> Option<String> {
    let conn = state.db.lock().ok()?;
    db::cache_get(&conn, &format!("tactics_tag:{}", team.to_lowercase()), af::now_ts())
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
}

/// Cheap (Haiku) tactical play-style profile + short style tag for a team, cached
/// by team+coach+formation. Returns (profile, tag).
async fn team_tactics(state: &AppState, team: &str, coach: Option<&str>, formation: Option<&str>) -> Option<(String, String)> {
    let key = format!(
        "tactics:{}|{}|{}",
        team.to_lowercase(),
        coach.unwrap_or("").to_lowercase(),
        formation.unwrap_or("")
    );
    let now = af::now_ts();
    let raw = if let Some(s) = {
        let conn = state.db.lock().ok()?;
        db::cache_get(&conn, &key, now).ok().flatten()
    } {
        s
    } else {
        let coach_s = coach.map(|c| format!(" under coach {c}")).unwrap_or_default();
        let form_s = formation.map(|f| format!(" (lined up {f})")).unwrap_or_default();
        let user = format!(
            "Describe {team}'s typical tactical style{coach_s}{form_s}. Reply in exactly this shape:\nSTYLE: <2-3 word label, e.g. low-block / high-press / possession / counter-attacking>\n<one or two short sentences: how they build up, where they create and concede shots (inside vs outside box), and pace on transitions>. Factual and concise."
        );
        let (text, gin, gout) =
            llm::anthropic_call(state, "claude-haiku-4-5", "You are a concise, factual football tactics analyst.", &user, 220)
                .await
                .ok()?;
        let text = text.trim().to_string();
        if text.is_empty() {
            return None;
        }
        let conn = state.db.lock().ok()?;
        let _ = db::usage_add(&conn, now, "claude-haiku-4-5", gin, gout, "tactics");
        let _ = db::cache_put(&conn, &key, "tactics", &text, now, 14 * 24 * 3600);
        text
    };
    let (tag, profile) = parse_style(&raw);
    if !tag.is_empty() {
        if let Ok(conn) = state.db.lock() {
            let _ = db::cache_put(&conn, &format!("tactics_tag:{}", team.to_lowercase()), "tactics_tag", &tag, now, 14 * 24 * 3600);
        }
    }
    Some((profile, tag))
}

/// Player-level market toggle keys (everything else is a team/match line).
fn is_player_market(m: &str) -> bool {
    matches!(
        m,
        "scorer" | "sot" | "tackles" | "fouls" | "cards" | "passes" | "assists" | "pshots" | "saves"
    )
}

/// Whether a candidate's player name matches any Grok-flagged unavailable name.
fn name_flagged(subject: &str, names: &[String]) -> bool {
    let s = subject.to_lowercase();
    let s_last = s.rsplit(' ').next().unwrap_or(&s).to_string();
    names.iter().any(|n| {
        let n = n.to_lowercase();
        let n_last = n.rsplit(' ').next().unwrap_or(&n).to_string();
        s.contains(&n) || n.contains(&s) || (n_last.len() >= 4 && s_last == n_last)
    })
}

fn map_availability(kind: &str, reason: &str) -> String {
    let r = reason.to_lowercase();
    if r.contains("suspend") {
        return "suspended".to_string();
    }
    let k = kind.to_lowercase();
    if k.contains("questionable") || r.contains("doubt") {
        return "doubtful".to_string();
    }
    if k.contains("missing") || k.contains("out") || !r.is_empty() {
        return "injured".to_string();
    }
    "unknown".to_string()
}

// ---------- build tickets ----------

const ALL_MARKETS: [&str; 28] = [
    "scorer", "sot", "pshots", "assists", "tackles", "fouls", "cards", "passes", "win", "dc",
    "btts", "half1", "half2", "ou25", "tgoals", "tcorners", "tshots", "ahandicap", "h1goals",
    "h2goals", "exactscore", "goalsrange", "saves", "toffsides", "tcards", "bothcards", "mostcards",
    "firstscore",
];

/// Fetch a team's full player list + season stats (paged), for auto candidate
/// generation — no manual player selection needed.
async fn fetch_team_players(
    state: &AppState,
    team_id: i64,
    season: i64,
) -> Result<Vec<Value>, String> {
    let mut entries = Vec::new();
    let mut page = 1;
    loop {
        let json = af::cached_get(
            state,
            "/players",
            vec![
                ("team", team_id.to_string()),
                ("season", season.to_string()),
                ("page", page.to_string()),
            ],
            af::TTL_PLAYERS,
        )
        .await?;
        entries.extend(response_array(&json));
        let total = json
            .get("paging")
            .and_then(|p| p.get("total"))
            .and_then(|v| v.as_i64())
            .unwrap_or(1);
        if page >= total || page >= 3 {
            break;
        }
        page += 1;
    }
    Ok(entries)
}

fn entry_minutes(e: &Value) -> f64 {
    e.get("statistics")
        .and_then(|s| s.as_array())
        .map(|arr| {
            arr.iter()
                .map(|s| {
                    s.get("games")
                        .and_then(|g| g.get("minutes"))
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0)
                })
                .sum()
        })
        .unwrap_or(0.0)
}

/// Most-involved players by minutes — the ones worth pricing.
fn top_players(mut entries: Vec<Value>, n: usize) -> Vec<Value> {
    entries.sort_by(|a, b| {
        entry_minutes(b)
            .partial_cmp(&entry_minutes(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    entries.truncate(n);
    entries
}

/// Re-derive each ticket's leg numbers and combined prob/odds/EV from our own
/// candidate data (deterministic — the model never invents these).
/// The team to badge on a leg: a player's team, or the team itself for a
/// team-specific market (so "Team Corners Over 5.5" shows which team). Match-
/// level lines (BTTS, total goals O/U) have no single team.
fn team_badge_of(c: &Candidate) -> Option<String> {
    if c.subject_kind == "player" {
        return Some(c.team.clone());
    }
    if !c.team.is_empty() && c.subject != "Match" && c.subject != "Both Teams" {
        return Some(c.team.clone());
    }
    None
}

/// A short, faithful phrase for a leg (used to rebuild ticket titles).
fn leg_short(l: &crate::models::TicketLeg) -> String {
    let line = l.line.clone().unwrap_or_default();
    let sel = l.selection.as_str();
    match l.market.as_str() {
        "BTTS" => format!("BTTS {}", line.to_lowercase()),
        "Match Result" => format!("{sel} {line}"),
        "Double Chance" => sel.to_string(),
        "Asian Handicap" => format!("{sel} {line}"),
        "Team Corners" => format!("{sel} corners {line}"),
        "Team Shots" => format!("{sel} shots {line}"),
        "Team Total Goals" => format!("{sel} {line} goals"),
        "Anytime Scorer" => format!("{sel} to score"),
        "Anytime Assist" => format!("{sel} assist"),
        "Win 1st Half" => format!("{sel} 1st half"),
        "Win 2nd Half" => format!("{sel} 2nd half"),
        m if m.ends_with("Goals") => format!("{line} goals"), // total O/U (sel = "Match")
        _ => format!("{sel} {line}"),                          // SOT / shots / tackles / cards…
    }
}

fn reground_tickets(
    result: &mut BuildResult,
    cands: &[Candidate],
    allowed_types: &[String],
    expected: usize,
    user_cap: Option<u32>,
) {
    for t in result.tickets.iter_mut() {
        let mut fixtures: HashSet<String> = HashSet::new();
        // Keep only legs that match a real candidate; overwrite EVERY number with
        // ours so a model-invented odd/prob can never reach the UI.
        let mut kept: Vec<crate::models::TicketLeg> = Vec::new();
        let mut leg_sigs: HashSet<String> = HashSet::new();
        // A player's goal/SOT/shots legs are nested (a goal implies a shot) — keep
        // at most one shot-family leg per player within a ticket.
        let mut shot_family: HashSet<String> = HashSet::new();
        for mut leg in t.legs.drain(..) {
            let sel = leg.selection.to_lowercase();
            let mkt = leg.market.to_lowercase();
            let cand = cands
                .iter()
                .find(|c| c.subject.to_lowercase() == sel && c.market.to_lowercase() == mkt)
                .or_else(|| cands.iter().find(|c| c.subject.to_lowercase() == sel));
            if let Some(c) = cand {
                let sig = format!("{}|{}|{}", c.market, c.subject, c.line);
                if !leg_sigs.insert(sig) {
                    continue; // duplicate leg within this ticket
                }
                if matches!(c.market_group.as_str(), "scorer" | "sot" | "pshots")
                    && !shot_family.insert(c.subject.to_lowercase())
                {
                    continue; // nested shot-family leg for the same player
                }
                leg.selection = c.subject.clone();
                leg.market = c.market.clone();
                leg.team = team_badge_of(c);
                leg.line = Some(c.line.clone());
                leg.est_prob = Some(c.est_prob);
                leg.pinnacle_prob = c.pinnacle_prob;
                leg.book_odds = c.book_odds;
                leg.book = c.book.clone();
                leg.ev = c.ev;
                leg.ev_source = c.ev_source.clone();
                leg.r#match = c.fixture.clone();
                leg.fixture_id = c.fixture_id;
                fixtures.insert(c.fixture.clone());
                kept.push(leg);
            }
            // unmatched → dropped (phantom leg)
        }
        // Group legs from the same fixture together — easier to enter by hand.
        kept.sort_by(|a, b| a.r#match.cmp(&b.r#match));
        t.legs = kept;
        if t.legs.is_empty() {
            continue;
        }

        let probs: Vec<f64> = t.legs.iter().filter_map(|l| l.est_prob).collect();
        t.combined_prob = if probs.len() == t.legs.len() && !probs.is_empty() {
            Some(round4(probs.iter().product()))
        } else {
            None
        };
        let book: Vec<f64> = t.legs.iter().filter_map(|l| l.book_odds).collect();
        let all_priced = book.len() == t.legs.len() && !book.is_empty();
        t.combined_odds = if all_priced {
            Some(round2(book.iter().product()))
        } else {
            None
        };
        // Combined EV uses Pinnacle true-prob where available, else our model prob.
        let truep: Vec<f64> = t
            .legs
            .iter()
            .filter_map(|l| l.pinnacle_prob.or(l.est_prob))
            .collect();
        t.combined_ev = if all_priced && truep.len() == t.legs.len() {
            let o: f64 = book.iter().product();
            let p: f64 = truep.iter().product();
            Some(round4(o * p - 1.0))
        } else {
            None
        };

        let mut per_fix: HashMap<String, usize> = HashMap::new();
        for l in &t.legs {
            *per_fix.entry(l.r#match.clone()).or_insert(0) += 1;
        }
        let max_pf = per_fix.values().copied().max().unwrap_or(0);
        t.kind = if t.legs.len() <= 1 {
            "Single".to_string()
        } else if per_fix.len() <= 1 {
            "SGP".to_string()
        } else if max_pf >= 2 {
            "SGP+".to_string() // a real same-game core combined across fixtures
        } else {
            "Acca".to_string() // one leg per fixture — cross-game parlay, not SGP+
        };
        if t.legs.len() > 1 && t.combined_odds.is_some() {
            t.flags.push("estimated parlay price".to_string());
        }
        // Rebuild the title from the ACTUAL (regrounded) legs so it can never
        // disagree with the lines the model wrote in its free-text title.
        let parts: Vec<String> = t.legs.iter().take(3).map(leg_short).collect();
        let mut title = parts.join(" + ");
        if t.legs.len() > 3 {
            title += &format!(" +{}", t.legs.len() - 3);
        }
        if !title.is_empty() {
            t.title = title;
        }
    }
    let model_count = result.tickets.len();

    // Enforce the allowed ticket types (e.g. drop singles the model slipped in
    // despite being disabled).
    if !allowed_types.is_empty() {
        result.tickets.retain(|t| allowed_types.iter().any(|a| a.eq_ignore_ascii_case(&t.kind)));
    }
    let after_types = result.tickets.len();

    // Drop empty tickets and any duplicate tickets (same set of legs).
    let mut seen: HashSet<String> = HashSet::new();
    result.tickets.retain(|t| {
        if t.legs.is_empty() {
            return false;
        }
        let mut sig: Vec<String> = t
            .legs
            .iter()
            .map(|l| format!("{}|{}|{}", l.market, l.selection, l.line.clone().unwrap_or_default()))
            .collect();
        sig.sort();
        seen.insert(sig.join("##"))
    });
    let after_dedupe = result.tickets.len();

    // Anti-correlation cap: a subject should appear in at most `cap` tickets. The
    // user's diversity setting wins; otherwise a lenient default that won't gut a
    // big slate. CRUCIAL: this is a PREFERENCE — we never drop below the requested
    // count to honour it. Over-cap tickets are held back, then added back in if we
    // fall short, so the slate keeps the count the user asked for.
    let cap = match user_cap {
        Some(c) if c > 0 => c as usize,
        _ => ((after_dedupe as f64 / 3.0).ceil() as usize).max(3),
    };
    let mut sub_counts: HashMap<String, usize> = HashMap::new();
    let mut kept: Vec<Ticket> = Vec::new();
    let mut held: Vec<Ticket> = Vec::new();
    for t in result.tickets.drain(..) {
        let mut subs: Vec<String> = t.legs.iter().map(|l| l.selection.to_lowercase()).collect();
        subs.sort();
        subs.dedup();
        if subs.iter().any(|s| *sub_counts.get(s).unwrap_or(&0) >= cap) {
            held.push(t); // over the cap — hold back, only used if we'd fall short
        } else {
            for s in subs {
                *sub_counts.entry(s).or_insert(0) += 1;
            }
            kept.push(t);
        }
    }
    // Backfill from the held-back pile until we reach the requested count.
    while kept.len() < expected && !held.is_empty() {
        kept.push(held.remove(0));
    }
    let corr = held.len();
    result.tickets = kept;

    let final_count = result.tickets.len();
    if final_count < model_count {
        let wrong_type = model_count - after_types;
        let dup = after_types - after_dedupe;
        result.data_quality_notes.push(format!(
            "Kept {final_count} of {model_count} model tickets ({wrong_type} wrong type, {dup} duplicate, {corr} over-correlated trimmed, subject cap {cap}/ticket). Add more matches or markets for greater variety."
        ));
    }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}
fn round4(x: f64) -> f64 {
    (x * 10000.0).round() / 10000.0
}

/// Stable PER-LINE cache key for a candidate's plausibility, independent of which
/// other lines are in the batch — so a background prewarm and the later build/
/// ladder share the same cache even after filtering changes the set.
fn plaus_key(model: &str, c: &Candidate) -> String {
    let mut h = Sha256::new();
    h.update(c.fixture_id.to_le_bytes());
    h.update(crate::odds::fold(&c.subject).as_bytes());
    h.update(c.market.as_bytes());
    h.update(c.line.as_bytes());
    h.update(model.as_bytes());
    h.update(b"plaus-line-v2");
    format!("{:x}", h.finalize())
}

/// Per-fixture Haiku plausibility pre-score (1-5 + reason) for each candidate
/// line — one cheap call PER FIXTURE (never per player). Scores are cached
/// PER LINE so a background prewarm warms exactly what the build/ladder reads.
/// `call_if_missing=false` means cache-only (used by the deterministic ladder, so
/// it stays model-call-free). Returns (input_tokens, output_tokens) spent.
async fn attach_plausibility(
    state: &AppState,
    candidates: &mut [Candidate],
    model: &str,
    call_if_missing: bool,
) -> (i64, i64) {
    let mut by_fix: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, c) in candidates.iter().enumerate() {
        by_fix.entry(c.fixture_id).or_default().push(i);
    }
    let (mut tin, mut tout) = (0i64, 0i64);
    for (_fix, idxs) in by_fix {
        if idxs.is_empty() {
            continue;
        }
        // 1) Apply whatever is already cached per line; collect the misses.
        let mut uncached: Vec<usize> = Vec::new();
        for &i in &idxs {
            let key = plaus_key(model, &candidates[i]);
            let hit = state.db.lock().ok().and_then(|c| db::ai_get(&c, &key).ok().flatten());
            match hit.and_then(|js| serde_json::from_str::<Value>(&js).ok()) {
                Some(v) => apply_plaus(&mut candidates[i], &v),
                None => uncached.push(i),
            }
        }
        if uncached.is_empty() || !call_if_missing {
            continue; // fully cached, or cache-only mode (ladder) — no model call
        }
        // 2) One Haiku call for this fixture's missing lines; cache each per line.
        let label = candidates[idxs[0]].fixture.clone();
        let lines: Vec<Value> = uncached
            .iter()
            .map(|&i| {
                let c = &candidates[i];
                serde_json::json!({
                    "subject": c.subject, "market": c.market, "line": c.line,
                    "est": c.est_prob, "odds": c.book_odds, "flags": c.flags
                })
            })
            .collect();
        let lines_compact = serde_json::to_string(&lines).unwrap_or_default();
        if let Ok((sc, gin, gout)) = llm::score_plausibility(state, model, &label, "", &lines_compact).await {
            tin += gin;
            tout += gout;
            if let Ok(conn) = state.db.lock() {
                let _ = db::usage_add(&conn, af::now_ts(), model, gin, gout, "plausibility");
            }
            for (subj, market, line, score, reason) in sc {
                let sl = crate::odds::fold(&subj);
                for &i in &uncached {
                    if crate::odds::fold(&candidates[i].subject) == sl
                        && candidates[i].market == market
                        && (line.is_empty() || candidates[i].line == line)
                    {
                        let v = serde_json::json!({ "s": score, "r": reason });
                        apply_plaus(&mut candidates[i], &v);
                        let key = plaus_key(model, &candidates[i]);
                        if let Ok(conn) = state.db.lock() {
                            let _ = db::ai_put(&conn, &key, &v.to_string(), model, af::now_ts());
                        }
                    }
                }
            }
        }
    }
    (tin, tout)
}

/// Apply a cached/returned plausibility value `{s, r}` to a candidate.
fn apply_plaus(c: &mut Candidate, v: &Value) {
    let score = v.get("s").and_then(|x| x.as_i64()).unwrap_or(3).clamp(1, 5) as u8;
    let reason = v.get("r").and_then(|x| x.as_str()).unwrap_or("");
    c.plausibility = Some(score);
    if !reason.is_empty() {
        c.support.push(format!("plausibility {score}/5: {reason}"));
    }
}

/// Background pre-score: warm the per-line plausibility cache for ONE fixture so
/// the later build/ladder reads it instantly. The frontend calls this once per
/// fixture (to drive a 1/x progress bar). Returns how many lines were scored.
#[tauri::command]
pub async fn prewarm_plausibility(
    state: State<'_, AppState>,
    fixture: FixtureInput,
    markets: Vec<String>,
) -> Result<usize, String> {
    let books = {
        let keys = state.keys.lock().map_err(|_| "keys lock")?;
        keys.books.clone()
    };
    let mkts: Vec<String> = if markets.is_empty() {
        ALL_MARKETS.iter().map(|s| s.to_string()).collect()
    } else {
        markets
    };
    let mut cands = gather_candidates(&state, &[fixture], &mkts, &books).await;
    if cands.is_empty() {
        return Ok(0);
    }
    let _ = attach_plausibility(&state, &mut cands, "claude-haiku-4-5", true).await;
    Ok(cands.iter().filter(|c| c.plausibility.is_some()).count())
}

/// League standings context (rank, points, form) for both teams — a motivation
/// signal for the model. Cached per league.
async fn standings_note(
    state: &AppState,
    league_id: i64,
    season: i64,
    home_id: i64,
    away_id: i64,
    home: &str,
    away: &str,
) -> Option<String> {
    let j = af::cached_get(
        state,
        "/standings",
        vec![("league", league_id.to_string()), ("season", season.to_string())],
        af::TTL_TEAMS,
    )
    .await
    .ok()?;
    let resp = response_array(&j);
    let standings = resp.first()?.get("league")?.get("standings")?.as_array()?.clone();
    let find = |tid: i64| -> Option<(i64, i64, String)> {
        for group in &standings {
            for e in group.as_array().into_iter().flatten() {
                if e.get("team").and_then(|t| t.get("id")).and_then(|v| v.as_i64()) == Some(tid) {
                    let rank = e.get("rank").and_then(|v| v.as_i64()).unwrap_or(0);
                    let pts = e.get("points").and_then(|v| v.as_i64()).unwrap_or(0);
                    let form = e.get("form").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    return Some((rank, pts, form));
                }
            }
        }
        None
    };
    match (find(home_id), find(away_id)) {
        (Some(h), Some(a)) => Some(format!(
            "Standings: {home} {}th {}pts (form {}); {away} {}th {}pts (form {}).",
            h.0, h.1, h.2, a.0, a.1, a.2
        )),
        _ => None,
    }
}

/// Head-to-head summary of recent meetings.
async fn h2h_note(state: &AppState, home_id: i64, away_id: i64, home: &str, away: &str) -> Option<String> {
    let j = af::cached_get(
        state,
        "/fixtures/headtohead",
        vec![("h2h", format!("{home_id}-{away_id}")), ("last", "6".to_string())],
        af::TTL_TEAMS,
    )
    .await
    .ok()?;
    let arr = response_array(&j);
    let (mut hw, mut aw, mut d, mut goals, mut n) = (0, 0, 0, 0.0_f64, 0);
    for it in &arr {
        let hg = it.get("goals").and_then(|g| g.get("home")).and_then(|v| v.as_i64());
        let ag = it.get("goals").and_then(|g| g.get("away")).and_then(|v| v.as_i64());
        let hid = it.get("teams").and_then(|t| t.get("home")).and_then(|x| x.get("id")).and_then(|v| v.as_i64());
        if let (Some(hg), Some(ag), Some(hid)) = (hg, ag, hid) {
            n += 1;
            goals += (hg + ag) as f64;
            let (oh, oa) = if hid == home_id { (hg, ag) } else { (ag, hg) };
            if oh > oa {
                hw += 1;
            } else if oa > oh {
                aw += 1;
            } else {
                d += 1;
            }
        }
    }
    if n == 0 {
        return None;
    }
    Some(format!(
        "H2H last {n}: {home} {hw}W, {away} {aw}W, {d}D, avg {:.1} goals.",
        goals / n as f64
    ))
}

fn pois_pmf_c(k: u32, l: f64) -> f64 {
    let mut t = (-l).exp();
    for i in 1..=k {
        t *= l / i as f64;
    }
    t
}

/// Live-adjusted goal forecast: given the CURRENT score and the remaining-time
/// goal expectations, rebuild Result / final scorelines / Goals from
/// (current score + additional goals). Replaces the stale pre-match versions so
/// a 1-1 game never shows "0-0 34%".
fn live_goal_sections(home: &str, away: &str, gh: i64, ga: i64, lh: f64, la: f64) -> Vec<crate::models::ForecastSection> {
    use crate::models::{ForecastLine, ForecastSection};
    let max = 6usize;
    let mut m = vec![vec![0.0f64; max + 1]; max + 1];
    for (x, row) in m.iter_mut().enumerate() {
        for (y, cell) in row.iter_mut().enumerate() {
            *cell = pois_pmf_c(x as u32, lh) * pois_pmf_c(y as u32, la);
        }
    }
    let pct = |p: f64| (p * 100.0).round();
    let mk = |label: String, p: f64| ForecastLine { label, pct: pct(p) };

    // Result from the FINAL score (current + additional).
    let (mut ph, mut pd, mut pa) = (0.0, 0.0, 0.0);
    for (x, row) in m.iter().enumerate() {
        for (y, c) in row.iter().enumerate() {
            let (fh, fa) = (gh + x as i64, ga + y as i64);
            if fh > fa {
                ph += c;
            } else if fh == fa {
                pd += c;
            } else {
                pa += c;
            }
        }
    }

    // Most likely FINAL scorelines.
    let mut scores: std::collections::HashMap<(i64, i64), f64> = std::collections::HashMap::new();
    for (x, row) in m.iter().enumerate() {
        for (y, c) in row.iter().enumerate() {
            *scores.entry((gh + x as i64, ga + y as i64)).or_insert(0.0) += c;
        }
    }
    let mut score_v: Vec<((i64, i64), f64)> = scores.into_iter().collect();
    score_v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Goals: total = current + remaining.
    let cur = gh + ga;
    let mut over25 = 0.0;
    for (x, row) in m.iter().enumerate() {
        for (y, c) in row.iter().enumerate() {
            if cur + x as i64 + y as i64 >= 3 {
                over25 += c;
            }
        }
    }
    let btts = if gh >= 1 && ga >= 1 {
        1.0
    } else if gh >= 1 {
        1.0 - (-la).exp()
    } else if ga >= 1 {
        1.0 - (-lh).exp()
    } else {
        (1.0 - (-lh).exp()) * (1.0 - (-la).exp())
    };
    let any_more = 1.0 - m[0][0];

    vec![
        ForecastSection {
            title: "Result (from here)".into(),
            lines: vec![mk(format!("{home} win"), ph), mk("Draw".into(), pd), mk(format!("{away} win"), pa)],
        },
        ForecastSection {
            title: "Goals (live-adjusted)".into(),
            lines: vec![
                mk("Another goal coming".into(), any_more),
                mk("Over 2.5 total".into(), over25),
                mk("Both teams to score".into(), btts),
            ],
        },
        ForecastSection {
            title: "Most likely FINAL score".into(),
            lines: score_v.iter().take(4).map(|((h, a), p)| mk(format!("{h}-{a}"), *p)).collect(),
        },
    ]
}

/// Best (highest-prob) candidate in a market group matching a predicate.
fn fc_best<'a>(cands: &'a [Candidate], group: &str, pred: impl Fn(&Candidate) -> bool) -> Option<&'a Candidate> {
    cands
        .iter()
        .filter(|c| c.market_group == group && pred(c))
        .max_by(|a, b| a.est_prob.partial_cmp(&b.est_prob).unwrap_or(std::cmp::Ordering::Equal))
}

/// Build a deterministic single-match forecast from the computed candidate
/// probabilities — likely result, scorelines, goals, cards/corners, key players.
/// No model call: every % is read straight off our engine's numbers.
fn forecast_from_candidates(all: &[Candidate], fixture_label: &str, home: &str, away: &str) -> crate::models::MatchForecast {
    use crate::models::{ForecastLine, ForecastSection, MatchForecast};
    // Only this fixture's candidates (so a multi-match build doesn't bleed across).
    let owned: Vec<Candidate> = all.iter().filter(|c| c.fixture == fixture_label).cloned().collect();
    let cands = owned.as_slice();
    let mk = |label: String, p: f64| ForecastLine { label, pct: (p * 100.0).round() };
    let prob = |group: &str, p: &dyn Fn(&Candidate) -> bool| fc_best(cands, group, |c| p(c)).map(|c| c.est_prob).unwrap_or(0.0);
    let top_n = |group: &str, n: usize| -> Vec<&Candidate> {
        let mut v: Vec<&Candidate> = cands.iter().filter(|c| c.market_group == group).collect();
        v.sort_by(|a, b| b.est_prob.partial_cmp(&a.est_prob).unwrap_or(std::cmp::Ordering::Equal));
        v.truncate(n);
        v
    };
    let mut sections: Vec<ForecastSection> = Vec::new();

    // Result
    let hw = prob("win", &|c| c.subject == home);
    let aw = prob("win", &|c| c.subject == away);
    let draw = (1.0 - hw - aw).max(0.0);
    sections.push(ForecastSection {
        title: "Result".into(),
        lines: vec![mk(format!("{home} win"), hw), mk("Draw".into(), draw), mk(format!("{away} win"), aw)],
    });

    // Goals
    let over25 = prob("ou25", &|c| c.line.to_lowercase().starts_with("over 2.5"));
    let mut goals = vec![mk("Over 2.5 goals".into(), over25), mk("Both teams to score".into(), prob("btts", &|_| true))];
    if let Some(gr) = fc_best(cands, "goalsrange", |_| true) {
        goals.push(mk(format!("Most likely: {}", gr.line), gr.est_prob));
    }
    let fh = prob("firstscore", &|c| c.subject == home);
    if fh > 0.0 {
        goals.push(mk(format!("{home} score first"), fh));
        goals.push(mk(format!("{away} score first"), prob("firstscore", &|c| c.subject == away)));
    }
    sections.push(ForecastSection { title: "Goals".into(), lines: goals });

    // Most likely scorelines
    let scores = top_n("exactscore", 4);
    if !scores.is_empty() {
        sections.push(ForecastSection {
            title: "Most likely scores".into(),
            lines: scores.iter().map(|c| mk(c.line.clone(), c.est_prob)).collect(),
        });
    }

    // Cards & corners
    let mut cc = Vec::new();
    if let Some(m) = fc_best(cands, "mostcards", |_| true) {
        cc.push(mk(format!("Most cards: {}", m.subject), m.est_prob));
    }
    let both = prob("bothcards", &|_| true);
    if both > 0.0 {
        cc.push(mk("Both teams carded".into(), both));
    }
    if let Some(co) = fc_best(cands, "tcorners", |c| c.line.to_lowercase().starts_with("over")) {
        cc.push(mk(format!("{} {}", co.subject, co.line.to_lowercase()), co.est_prob));
    }
    if !cc.is_empty() {
        sections.push(ForecastSection { title: "Cards & corners".into(), lines: cc });
    }

    // Likely players
    let mut players = Vec::new();
    for c in top_n("scorer", 3) {
        players.push(mk(format!("{} to score", c.subject), c.est_prob));
    }
    for c in top_n("sot", 2) {
        players.push(mk(format!("{} — {}", c.subject, c.line), c.est_prob));
    }
    for c in top_n("cards", 2) {
        players.push(mk(format!("{} to be carded", c.subject), c.est_prob));
    }
    if !players.is_empty() {
        sections.push(ForecastSection { title: "Likely players".into(), lines: players });
    }

    let lean = if hw > aw + 0.12 {
        format!("{home} favoured")
    } else if aw > hw + 0.12 {
        format!("{away} favoured")
    } else {
        "Tight match".to_string()
    };
    let goalsy = if over25 >= 0.55 { "high-scoring" } else if over25 <= 0.42 { "low-scoring" } else { "moderate goals" };
    crate::models::MatchForecast {
        home: home.to_string(),
        away: away.to_string(),
        headline: format!("{lean} · {goalsy}"),
        sections,
    }
}

#[tauri::command]
pub async fn build_tickets(
    state: State<'_, AppState>,
    selection: BuildSelection,
) -> Result<BuildResponse, String> {
    if selection.fixtures.is_empty() {
        return Err("Select at least one match first.".to_string());
    }

    // Match Predictor: a deep read of ONE game — force every market so the
    // forecast and the SGP variations have the full picture.
    let predictor = selection.strategy.as_deref() == Some("predictor") && selection.fixtures.len() == 1;
    let markets: Vec<String> = if selection.markets.is_empty() || predictor {
        ALL_MARKETS.iter().map(|s| s.to_string()).collect()
    } else {
        selection.markets.clone()
    };
    let player_groups: Vec<String> = markets.iter().filter(|m| is_player_market(m)).cloned().collect();
    let team_groups: Vec<String> = markets.iter().filter(|m| !is_player_market(m)).cloned().collect();

    let books = {
        let keys = state.keys.lock().map_err(|_| "keys lock")?;
        keys.books.clone()
    };

    let mut candidates: Vec<Candidate> = Vec::new();
    let mut pred_notes: Vec<String> = Vec::new();
    let mut live_notes: Vec<String> = Vec::new();
    let mut any_live = false;
    let mut tactics_tags: HashMap<String, String> = HashMap::new();

    // Calibration shrink learned from settled bets (1.0 = none).
    let (calib_lambda, calib_n) = calibration_shrink(&state);
    let calib_on = (calib_lambda - 1.0).abs() > 1e-6;

    for fx in &selection.fixtures {
        let fixture_label = format!("{} vs {}", fx.home_team, fx.away_team);
        let fx_start = candidates.len();

        // If kickoff has passed, pull fresh in-play state so the model reasons
        // off the current scoreline/clock, not pre-match season rates.
        if kickoff_elapsed(&fx.date_utc) {
            if let Some(live) = fetch_live_state(&state, fx.fixture_id).await {
                if live.is_live() {
                    any_live = true;
                    let line = format!(
                        "LIVE NOW — {} {}-{} {} ({}′, {}). Account for the current scoreline and the time remaining; do not suggest lines that are already settled.",
                        fx.home_team, live.home_goals, live.away_goals, fx.away_team, live.elapsed, live.status
                    );
                    pred_notes.push(line);
                    live_notes.push(format!(
                        "Live: {} {}-{} {} ({}′) — pre-match season rates shown are NOT live-adjusted; weigh the in-play state.",
                        fx.home_team, live.home_goals, live.away_goals, fx.away_team, live.elapsed
                    ));
                } else if live.is_finished() {
                    live_notes.push(format!(
                        "Note: {} appears finished ({} {}-{}); its lines are likely settled.",
                        fixture_label, live.home_goals, live.away_goals, live.status
                    ));
                }
            }
        }

        let injuries = fetch_injury_map(&state, fx.fixture_id).await.unwrap_or_default();

        // Odds (Pinnacle + Bet365) and predictions — best-effort.
        let fixture_odds = af::cached_get(
            &state,
            "/odds",
            vec![("fixture", fx.fixture_id.to_string())],
            af::TTL_ODDS,
        )
        .await
        .ok()
        .map(|j| crate::odds::parse_fixture_odds(&j, &books))
        .unwrap_or_default();

        if selection.use_predictions.unwrap_or(true) {
            if let Ok(pj) = af::cached_get(
                &state,
                "/predictions",
                vec![("fixture", fx.fixture_id.to_string())],
                af::TTL_PREDICTIONS,
            )
            .await
            {
                if let Some(s) = crate::odds::predictions_summary(&pj, &fixture_label) {
                    pred_notes.push(s);
                }
            }
        }

        // Context signals fed to the model (it weighs them; the engine stays
        // numeric). Each is toggleable so the user can trade depth for speed.
        if selection.use_weather.unwrap_or(true) {
            if let (Some(city), Some(date)) = (&fx.venue_city, &fx.date_utc) {
                if let Some(w) = crate::weather::match_weather(&state, city, date).await {
                    pred_notes.push(format!("{fixture_label}: weather ~kickoff — {w}."));
                }
            }
        }
        if selection.use_standings.unwrap_or(true) {
            if let Some(s) =
                standings_note(&state, fx.league_id, fx.season, fx.home_team_id, fx.away_team_id, &fx.home_team, &fx.away_team).await
            {
                pred_notes.push(format!("{fixture_label}: {s}"));
            }
        }
        if selection.use_h2h.unwrap_or(true) {
            if let Some(h) = h2h_note(&state, fx.home_team_id, fx.away_team_id, &fx.home_team, &fx.away_team).await {
                pred_notes.push(format!("{fixture_label}: {h}"));
            }
        }
        if let Some(r) = &fx.referee {
            pred_notes.push(format!(
                "{fixture_label}: referee {r} — use your knowledge of this ref's card tendency when weighing card markets (a strict ref lifts to-be-carded / team-cards / both-teams-card; a lenient one lowers them).",
            ));
        }

        // Coach / formation / play-style profile (cheap Haiku, cached). Helps the
        // model weigh low-block sides, pace on counters, shot location, etc.
        if selection.use_tactics.unwrap_or(false) {
            let formations = fetch_formations(&state, fx.fixture_id).await;
            for (tid, tname) in [(fx.home_team_id, &fx.home_team), (fx.away_team_id, &fx.away_team)] {
                let coach = fetch_coach(&state, tid).await;
                let formation = formations.get(&tid).cloned();
                if let Some((profile, tag)) = team_tactics(&state, tname, coach.as_deref(), formation.as_deref()).await {
                    let f = formation.map(|x| format!(" [{x}]")).unwrap_or_default();
                    pred_notes.push(format!("{tname}{f} tactics: {profile}"));
                    if !tag.is_empty() {
                        tactics_tags.insert(tname.clone(), tag);
                    }
                }
            }
        }

        // Confirmed lineups (if posted) — restrict to the starting XI so we don't
        // build bets around players who won't start.
        let starters = if selection.use_lineups.unwrap_or(true) {
            fetch_starters(&state, fx.fixture_id).await
        } else {
            HashMap::new()
        };
        if !starters.is_empty() {
            pred_notes.push(format!(
                "{}: confirmed lineups posted — only the starting XI is used for player props.",
                fixture_label
            ));
        }

        // Player legs — auto: top players per team by minutes (or the start XI).
        if !player_groups.is_empty() {
            let in_form = fetch_inform(&state, fx.league_id, fx.season).await;
            for (team_id, team_name, is_home, opp) in [
                (fx.home_team_id, fx.home_team.clone(), true, fx.away_team.clone()),
                (fx.away_team_id, fx.away_team.clone(), false, fx.home_team.clone()),
            ] {
                let consistency = fetch_consistency(&state, team_id, fx.season).await;
                let entries = fetch_team_players(&state, team_id, fx.season).await.unwrap_or_default();
                let baselines = features::squad_baselines(&entries, fx.league_id);
                let team_starters = starters.get(&team_id);
                // If lineups are out for this team, keep only confirmed starters.
                let entries: Vec<Value> = match team_starters {
                    Some(set) => entries
                        .into_iter()
                        .filter(|e| {
                            e.get("player")
                                .and_then(|p| p.get("id"))
                                .and_then(|v| v.as_i64())
                                .map(|pid| set.contains(&pid))
                                .unwrap_or(false)
                        })
                        .collect(),
                    None => entries,
                };
                for entry in top_players(entries, 16) {
                    let pid = entry.get("player").and_then(|p| p.get("id")).and_then(|v| v.as_i64());
                    let availability = if team_starters.map(|s| pid.map(|id| s.contains(&id)).unwrap_or(false)).unwrap_or(false) {
                        "starting".to_string()
                    } else {
                        pid.and_then(|id| injuries.get(&id).cloned()).unwrap_or_else(|| "unknown".to_string())
                    };
                    let ctx = FixtureCtx {
                        fixture_label: fixture_label.clone(),
                        fixture_id: fx.fixture_id,
                        team: team_name.clone(),
                        opponent: opp.clone(),
                        is_home,
                        availability,
                    };
                    candidates.extend(features::build_player_candidates_entry(
                        &entry,
                        fx.league_id,
                        &ctx,
                        &player_groups,
                        &in_form,
                        &consistency,
                        &baselines,
                    ));
                }
            }
        }

        // Team/match legs.
        if !team_groups.is_empty() {
            let mut home = fetch_team_stats(&state, fx.home_team_id, &fx.home_team, fx.league_id, fx.season).await.ok().flatten();
            let mut away = fetch_team_stats(&state, fx.away_team_id, &fx.away_team, fx.league_id, fx.season).await.ok().flatten();
            // Recent-form data (xG, corners, shots). Needed for xG-toggle or the
            // corner/shots markets. Opt-in / auto — extra requests, cached after.
            let need_form = selection.use_xg.unwrap_or(false)
                || team_groups.iter().any(|m| matches!(m.as_str(), "tcorners" | "tshots" | "toutbox" | "tinbox" | "toffsides"));
            if need_form {
                let apply = |t: &mut crate::features::TeamStats, f: &TeamForm| {
                    t.xg_for = f.xg_for;
                    t.xg_against = f.xg_against;
                    t.corners_for = f.corners_for;
                    t.corners_against = f.corners_against;
                    t.shots_for = f.shots_for;
                    t.shots_against = f.shots_against;
                    t.outbox_for = f.outbox_for;
                    t.inbox_for = f.inbox_for;
                    t.offsides_for = f.offsides_for;
                };
                if let Some(h) = home.as_mut() {
                    if let Some(f) = fetch_team_form(&state, fx.home_team_id).await {
                        apply(h, &f);
                    }
                }
                if let Some(a) = away.as_mut() {
                    if let Some(f) = fetch_team_form(&state, fx.away_team_id).await {
                        apply(a, &f);
                    }
                }
                if let (Some(h), Some(a)) = (home.as_ref(), away.as_ref()) {
                    if h.xg_for.is_some() && a.xg_for.is_some() {
                        live_notes.push(format!(
                            "{fixture_label}: real xG (recent form) — {} {:.2}xGF/{:.2}xGA, {} {:.2}/{:.2}.",
                            fx.home_team, h.xg_for.unwrap(), h.xg_against.unwrap(),
                            fx.away_team, a.xg_for.unwrap(), a.xg_against.unwrap()
                        ));
                    }
                    // Shot-location tactical signal (no market for it — context only):
                    // a high outside-box share vs a deep/low-block opponent shifts where
                    // chances come from.
                    for (t, name) in [(h, &fx.home_team), (a, &fx.away_team)] {
                        if let (Some(ob), Some(ib)) = (t.outbox_for, t.inbox_for) {
                            let total = ob + ib;
                            if total > 0.0 {
                                pred_notes.push(format!(
                                    "{name}: {:.0}% of shots from OUTSIDE the box ({:.1} out / {:.1} in per game) — weigh vs the opponent's block.",
                                    ob / total * 100.0, ob, ib
                                ));
                            }
                        }
                    }
                }
            }
            // Recent card model for the card markets (cached /fixtures/players).
            if team_groups.iter().any(|m| matches!(m.as_str(), "tcards" | "bothcards" | "mostcards")) {
                let hc = fetch_team_cards(&state, fx.home_team_id, fx.season).await;
                let ac = fetch_team_cards(&state, fx.away_team_id, fx.season).await;
                if let Some(h) = home.as_mut() {
                    h.cards_for = hc.0;
                    h.cards_against = hc.1;
                    h.both_card_rate = hc.2;
                    h.most_card_rate = hc.3;
                }
                if let Some(a) = away.as_mut() {
                    a.cards_for = ac.0;
                    a.cards_against = ac.1;
                    a.both_card_rate = ac.2;
                    a.most_card_rate = ac.3;
                }
            }
            if let (Some(h), Some(a)) = (home, away) {
                candidates.extend(features::build_team_candidates(&h, &a, &fixture_label, fx.fixture_id, &team_groups));
            }
        }

        // Referee-aware cards: nudge this fixture's card markets by the ref's
        // estimated strictness (flagged as a model estimate, never as measured).
        let card_picked = player_groups.iter().chain(team_groups.iter()).any(|m| matches!(m.as_str(), "cards" | "tcards" | "bothcards"));
        if card_picked {
            if let Some(r) = &fx.referee {
                if let Some(strict) = fetch_ref_strictness(&state, r).await {
                    let mult = (strict / 4.2).clamp(0.78, 1.3);
                    for c in candidates[fx_start..].iter_mut() {
                        if matches!(c.market_group.as_str(), "cards" | "tcards" | "bothcards") {
                            let m = if c.line.to_lowercase().contains("under") { 1.0 / mult } else { mult };
                            c.est_prob = round4((c.est_prob * m).clamp(0.02, 0.98));
                            c.support.push(format!("referee ~{strict:.1} cards/game (model estimate)"));
                        }
                    }
                }
            }
        }

        // Apply the calibration shrink to this fixture's fresh legs BEFORE odds
        // attach, so the model-fallback EV reflects the adjusted probability.
        if calib_on {
            for c in candidates[fx_start..].iter_mut() {
                c.est_prob = round4((0.5 + calib_lambda * (c.est_prob - 0.5)).clamp(0.01, 0.99));
            }
        }

        // Attach Pinnacle/Bet365/EV to this fixture's legs.
        features::attach_odds(&mut candidates, &fixture_odds, &fixture_label, &fx.home_team);
    }

    if calib_on {
        live_notes.push(format!(
            "Calibration shrink λ={calib_lambda:.2} applied to model probabilities (learned from {calib_n} settled legs)."
        ));
    }

    // Safety ceiling: drop legs more likely than the cap so the slate isn't all
    // chalk — pushes the model toward less-obvious picks.
    if let Some(cap) = selection.max_leg_prob {
        if cap < 0.999 {
            candidates.retain(|c| c.est_prob <= cap);
        }
    }

    // Per-leg odds sweet-spot: when set, keep only PRICED legs inside [min,max]
    // — drops chalk (e.g. 1.07) and lottery prices (e.g. 29x) before the model.
    let odds_lo = selection.min_odds.unwrap_or(1.0);
    let odds_hi = selection.max_odds.unwrap_or(1000.0);
    if odds_lo > 1.01 || odds_hi < 999.0 {
        // Filter PRICED legs to the band; keep unpriced ones (most player props
        // have no odds — a min-odds floor shouldn't silently delete them).
        candidates.retain(|c| c.book_odds.map_or(true, |o| o >= odds_lo && o <= odds_hi));
    }

    if candidates.is_empty() {
        return Err("No legs in range — widen the odds band or raise the safety ceiling.".to_string());
    }

    // Optional Grok precursor: X/news team-news digest (injuries, sentiment).
    let mut veto_removed = 0usize;
    let mut grok_error: Option<String> = None;
    let grok_digest: Option<String> = if selection.use_grok {
        let labels: Vec<String> = selection
            .fixtures
            .iter()
            .map(|f| format!("{} vs {}", f.home_team, f.away_team))
            .collect();
        match crate::grok::fetch_digest(&state, &labels, &af::today(), any_live, &selection.grok_categories).await {
            Ok(r) => {
                {
                    let conn = state.db.lock().map_err(|_| "db lock")?;
                    let _ = db::grok_usage_add(&conn, af::now_ts(), r.input, r.output, r.sources, r.cost_usd);
                    let label = labels.join(", ");
                    let _ = db::grok_log_add(&conn, af::now_ts(), &label, &r.digest);
                }
                // Hard rule: drop legs for players Grok flags as out/suspended.
                if selection.grok_veto {
                    let names = crate::grok::parse_unavailable(&r.digest);
                    if !names.is_empty() {
                        let before = candidates.len();
                        candidates.retain(|c| {
                            c.subject_kind != "player" || !name_flagged(&c.subject, &names)
                        });
                        veto_removed = before - candidates.len();
                    }
                }
                Some(r.digest)
            }
            Err(e) => {
                grok_error = Some(e);
                None
            }
        }
    } else {
        None
    };
    let grok_used = grok_digest.is_some();

    let (model, limit) = {
        let keys = state.keys.lock().map_err(|_| "keys lock")?;
        let m = selection.model.trim();
        let model = if !m.is_empty() && llm::is_allowed_model(m) {
            m.to_string()
        } else {
            keys.model.clone().unwrap_or_else(|| llm::DEFAULT_MODEL.to_string())
        };
        (model, keys.daily_limit.unwrap_or(db::DEFAULT_DAILY_LIMIT))
    };

    // "Most likely" mode ranks by pure probability and widens the pool so a far
    // broader set of plausible lines reaches the model (not just +EV ones).
    let strategy = selection
        .strategy
        .clone()
        .unwrap_or_else(|| if selection.most_likely { "likely".to_string() } else { "value".to_string() });
    let (mut pool_n, mut market_cap) = match strategy.as_str() {
        "likely" => (90usize, 18usize),
        "favorites" => (70, 14),
        "oracle" => (60, 8),  // Claude's read — selective, diverse across markets
        "power" => (64, 7),   // power stacker — generous-priced bankers, tight per-market
        "bankers" => (80, 16), // wide net of reliable recurring events
        "jackpot" => (90, 14), // lottery — wide pool of plausible longshots to stack big
        "predictor" => (110, 20), // deep single-match read — the whole market for one game
        _ => (50, 8),         // value
    };
    // Make sure the pool is big enough to actually BUILD the requested slate: with
    // a high ticket count and a leg-count floor, the model needs enough distinct
    // legs or it returns fewer tickets. Scale to demand (count × min-legs).
    let total_tickets =
        selection.ticket_count.unwrap_or(10) + selection.lucky_safe + selection.lucky_moderate + selection.lucky_risky;
    let min_legs_eff = selection.min_legs.unwrap_or(1).max(1) as usize;
    let demand = total_tickets as usize * min_legs_eff;
    pool_n = pool_n.max(demand + 24).min(150);
    if min_legs_eff >= 3 {
        market_cap = market_cap.max(16); // more legs per market → more distinct combos
    }
    // Tag candidates with their team's tactical style (visible at pick time).
    if !tactics_tags.is_empty() {
        for c in candidates.iter_mut() {
            if let Some(tag) = tactics_tags.get(&c.team) {
                c.flags.push(format!("style: {tag}"));
            }
        }
    }
    // Per-fixture Haiku plausibility pre-score (cached) — a real-world context
    // weight blended into ranking. One cheap call per fixture, never per player.
    if selection.use_plausibility.unwrap_or(true) {
        let (pin, pout) = attach_plausibility(&state, &mut candidates, "claude-haiku-4-5", true).await;
        let scored = candidates.iter().filter(|c| c.plausibility.is_some()).count();
        if scored > 0 {
            live_notes.push(format!(
                "Haiku plausibility pre-score blended into ranking ({scored} lines{}).",
                if pin + pout == 0 { ", cached" } else { "" }
            ));
        }
    }
    // Match Predictor: deterministic forecast from the FULL candidate set (before
    // the shortlist trims it). If the match is IN-PLAY, pull live state, fold a
    // live section into the forecast, and inject the live situation so the model
    // adjusts its suggestions to what's actually happening.
    let forecast = if predictor {
        let fx = &selection.fixtures[0];
        let fl = format!("{} vs {}", fx.home_team, fx.away_team);
        let mut fc = forecast_from_candidates(&candidates, &fl, &fx.home_team, &fx.away_team);
        let now = af::now_ts();
        // In the kickoff window if we have a time; if no time was supplied (e.g.
        // the one-tap predict from the Live screen) attempt the live pull anyway —
        // the snapshot's status decides whether to actually adjust.
        let live_window = fx
            .date_utc
            .as_deref()
            .and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok())
            .map(|ko| now >= ko.timestamp() - 300 && now <= ko.timestamp() + 3 * 3600)
            .unwrap_or(true);
        if live_window {
            let lf = crate::models::LiveFixture {
                fixture_id: fx.fixture_id,
                league_id: fx.league_id,
                league_name: String::new(),
                season: fx.season,
                home_team: fx.home_team.clone(),
                away_team: fx.away_team.clone(),
                home_team_id: fx.home_team_id,
                away_team_id: fx.away_team_id,
                status: String::new(),
                elapsed: 0,
                home_goals: 0,
                away_goals: 0,
                has_stats: false,
            };
            if let Ok(snap) = live_snapshot_inner(&state, lf).await {
                let f = snap.fixture.clone();
                // In-play if there's an elapsed minute and it isn't finished/not-started.
                let in_play = (f.elapsed > 0 || f.status == "HT")
                    && !matches!(f.status.as_str(), "FT" | "AET" | "PEN" | "NS" | "TBD" | "PST" | "CANC" | "ABD" | "AWD" | "WO" | "");
                if in_play {
                    fc.headline = format!("⚡ LIVE {}' · {} {}-{} {}", f.elapsed, f.home_team, f.home_goals, f.away_goals, f.away_team);
                    // Rebuild the goal-derived sections from the CURRENT score +
                    // remaining-time expectation, replacing the stale pre-match ones.
                    let remaining = (90 - f.elapsed).clamp(0, 90) as f64;
                    let frac = remaining / 90.0;
                    let h_ts = fetch_team_stats(&state, fx.home_team_id, &fx.home_team, fx.league_id, fx.season).await.ok().flatten();
                    let a_ts = fetch_team_stats(&state, fx.away_team_id, &fx.away_team, fx.league_id, fx.season).await.ok().flatten();
                    fc.sections.retain(|s| !matches!(s.title.as_str(), "Result" | "Goals" | "Most likely scores"));
                    let mut new_secs: Vec<crate::models::ForecastSection> = Vec::new();
                    // Live remaining-time estimates first.
                    let live_lines: Vec<crate::models::ForecastLine> = snap
                        .estimates
                        .iter()
                        .map(|e| crate::models::ForecastLine { label: e.label.clone(), pct: (e.prob * 100.0).round() })
                        .collect();
                    if !live_lines.is_empty() {
                        new_secs.push(crate::models::ForecastSection { title: format!("Live — remaining ~{}'", remaining as i64), lines: live_lines });
                    }
                    if let (Some(h), Some(a)) = (&h_ts, &a_ts) {
                        let lh = ((h.gf_avg + a.ga_avg) / 2.0).max(0.05) * frac;
                        let la = ((a.gf_avg + h.ga_avg) / 2.0).max(0.05) * frac;
                        new_secs.extend(live_goal_sections(&f.home_team, &f.away_team, f.home_goals, f.away_goals, lh, la));
                    }
                    new_secs.append(&mut fc.sections); // keep Cards & corners + Likely players
                    fc.sections = new_secs;
                    let stat_str = snap
                        .stats
                        .iter()
                        .map(|t| format!("{}: {}", t.team, t.stats.iter().map(|s| format!("{} {}", s.label, s.value)).collect::<Vec<_>>().join(", ")))
                        .collect::<Vec<_>>()
                        .join(" | ");
                    let ev_str = snap.events.iter().rev().take(8).map(|e| format!("{}' {} {}", e.minute, e.kind, e.player)).collect::<Vec<_>>().join("; ");
                    let est_str = snap.estimates.iter().map(|e| format!("{} {}%", e.label, (e.prob * 100.0).round() as i64)).collect::<Vec<_>>().join("; ");
                    pred_notes.push(format!(
                        "⚡ THIS MATCH IS LIVE: {} {}-{} {} at {}' ({}). The pre-match season-rate numbers in the table are NOW STALE — re-weigh EVERY suggestion for the CURRENT score, momentum and time remaining. Live stats: {}. Recent events: {}. Our live remaining-time estimates: {}. Favour bets that fit what is ACTUALLY happening (a side dominating shots to score next; a tight late game → lean unders; chasing team → more shots/corners/cards).",
                        f.home_team, f.home_goals, f.away_goals, f.away_team, f.elapsed, f.status,
                        if stat_str.is_empty() { "(none)".into() } else { stat_str },
                        if ev_str.is_empty() { "(none)".into() } else { ev_str },
                        if est_str.is_empty() { "(none)".into() } else { est_str },
                    ));
                }
            }
        }
        Some(fc)
    } else {
        None
    };
    // Simple mode: a forecast for EVERY selected match.
    let simple_forecasts: Vec<crate::models::MatchForecast> = if selection.simple.unwrap_or(false) {
        selection
            .fixtures
            .iter()
            .map(|fx| {
                let fl = format!("{} vs {}", fx.home_team, fx.away_team);
                forecast_from_candidates(&candidates, &fl, &fx.home_team, &fx.away_team)
            })
            .collect()
    } else {
        vec![]
    };
    let shortlist = features::shortlist(candidates, pool_n, &strategy, market_cap);
    let table = features::compact_table_json(&shortlist);
    let opts = llm::PromptOpts {
        count: selection.ticket_count.unwrap_or(10),
        types: &selection.ticket_types,
        variation: selection.variation,
        exclude: &selection.exclude,
        bias_builders: selection.bias_builders,
        grok_veto: selection.grok_veto,
        strategy: strategy.clone(),
        lucky_safe: selection.lucky_safe,
        lucky_moderate: selection.lucky_moderate,
        lucky_risky: selection.lucky_risky,
        min_legs: selection.min_legs.unwrap_or(1).clamp(1, 12),
        max_per_subject: selection.max_per_subject.unwrap_or(0).min(20),
    };
    // Pull in any browser-ingested pages matched to these fixtures — labeled
    // 3rd-party context for the model, and mark each item "used".
    if selection.use_ingest.unwrap_or(true) {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        if let Ok(items) = db::ingest_for_fixture(&conn) {
            for it in items.iter().filter(|i| i.status == "processed") {
                let fl = crate::odds::fold(&it.fixture_label.clone().unwrap_or_default());
                if fl.is_empty() {
                    continue;
                }
                let m = selection.fixtures.iter().find(|f| {
                    fl.contains(&crate::odds::fold(&f.home_team)) || fl.contains(&crate::odds::fold(&f.away_team))
                });
                if let Some(f) = m {
                    let detail = it
                        .extracted_json
                        .as_deref()
                        .and_then(|j| serde_json::from_str::<Value>(j).ok())
                        .map(|v| compact_ingest(&v))
                        .unwrap_or_default();
                    if !detail.is_empty() {
                        pred_notes.push(format!(
                            "{} vs {}: INGESTED 3rd-party data (from {}): {}",
                            f.home_team, f.away_team, it.url, detail
                        ));
                        let _ = db::ingest_mark_used(&conn, it.id);
                    }
                }
            }
        }
    }

    let hash = llm::input_hash(
        &table,
        &markets,
        selection.reasoning,
        &model,
        &selection.notes,
        &pred_notes,
        grok_digest.as_deref(),
        &opts,
    );

    let mut det_notes = vec![
        "Pinnacle = de-vigged true probability (sharp); Bet365 = the price to take. +EV = book_odds × pinnacle_prob − 1.".to_string(),
        "Player props and some team lines aren't priced in the feed — those legs are likelihood-only (no EV).".to_string(),
        "SGP/SGP+ combined odds are estimates (no correlated SGP pricing); correlated legs are usually cheaper in reality.".to_string(),
        "Base rates are season-derived; xG on scorer legs is a proxy.".to_string(),
    ];
    if grok_used {
        det_notes.push("Grok X/news digest used as soft context (injuries, team news, sentiment).".to_string());
    }
    if veto_removed > 0 {
        det_notes.push(format!(
            "Injury veto: removed {veto_removed} player candidate(s) Grok flagged as out/suspended."
        ));
    }
    for ln in &live_notes {
        det_notes.push(ln.clone());
    }
    if let Some(e) = &grok_error {
        det_notes.push(format!("Grok unavailable — {e}"));
    }

    let cached = {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        db::ai_get(&conn, &hash)?
    };

    let (mut result, in_tok, out_tok, from_cache): (BuildResult, i64, i64, bool) =
        if let Some(json) = cached {
            let mut r: BuildResult = serde_json::from_str(&json).map_err(|e| e.to_string())?;
            r.from_cache = true;
            (r, 0, 0, true)
        } else {
            let call = llm::call_model(
                &state,
                &model,
                &table,
                &markets,
                selection.reasoning,
                &selection.notes,
                &pred_notes,
                grok_digest.as_deref(),
                &opts,
            )
            .await?;
            let mut r = call.result;
            r.from_cache = false;
            r.grok_used = grok_used;
            r.grok_digest = grok_digest.clone();
            reground_tickets(&mut r, &shortlist, &selection.ticket_types, total_tickets as usize, selection.max_per_subject);
            let stored = serde_json::to_string(&r).map_err(|e| e.to_string())?;
            {
                let conn = state.db.lock().map_err(|_| "db lock")?;
                db::ai_put(&conn, &hash, &stored, &model, af::now_ts())?;
                db::usage_add(&conn, af::now_ts(), &model, call.input_tokens, call.output_tokens, "build")?;
                // Auto-save every fresh run so it's viewable later.
                let sel_json = serde_json::to_string(&markets).unwrap_or_default();
                let _ = db::save_ticket(&conn, af::now_ts(), &sel_json, &stored, &selection.notes);
            }
            (r, call.input_tokens, call.output_tokens, false)
        };

    det_notes.extend(result.data_quality_notes.drain(..));
    result.data_quality_notes = det_notes;
    result.context_notes = pred_notes;
    result.forecast = forecast;
    result.forecasts = simple_forecasts;

    // Paper-trading ledger: record each unique generated ticket by strategy +
    // grok flag, so we can later settle them all and see which approach wins.
    {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        let day = af::today();
        for t in &result.tickets {
            if t.legs.is_empty() {
                continue;
            }
            let mut sig: Vec<String> = t
                .legs
                .iter()
                .map(|l| format!("{}|{}|{}", l.market, l.selection, l.line.clone().unwrap_or_default()))
                .collect();
            sig.sort();
            if let Ok(tj) = serde_json::to_string(t) {
                let _ = db::gen_add(&conn, af::now_ts(), &day, &strategy, grok_used, &t.kind, &sig.join("##"), &tj, t.combined_odds);
            }
        }
    }

    let meter = {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        db::meter(&conn, &af::today(), limit)?
    };
    let usage = BuildUsage {
        model: model.clone(),
        input_tokens: in_tok,
        output_tokens: out_tok,
        cost_usd: llm::cost_usd(&model, in_tok, out_tok),
        from_cache,
    };

    Ok(BuildResponse { result, meter, usage })
}

// ---------- picks board (build-your-own) ----------

/// Return the ranked, data-backed candidate legs for the selected fixtures —
/// no model call. The user composes their own ticket from these.
#[tauri::command]
pub async fn get_picks(
    state: State<'_, AppState>,
    fixtures: Vec<FixtureInput>,
    markets: Vec<String>,
) -> Result<Vec<Candidate>, String> {
    if fixtures.is_empty() {
        return Err("Select at least one match first.".to_string());
    }
    let markets: Vec<String> = if markets.is_empty() {
        ALL_MARKETS.iter().map(|s| s.to_string()).collect()
    } else {
        markets
    };
    let books = {
        let keys = state.keys.lock().map_err(|_| "keys lock")?;
        keys.books.clone()
    };

    let mut candidates = gather_candidates(&state, &fixtures, &markets, &books).await;

    // Board sort: +EV first, then likelihood. Cap for a manageable board.
    candidates.sort_by(|a, b| {
        let ea = a.ev.unwrap_or(-9.0);
        let eb = b.ev.unwrap_or(-9.0);
        eb.partial_cmp(&ea)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.est_prob.partial_cmp(&a.est_prob).unwrap_or(std::cmp::Ordering::Equal))
    });
    candidates.truncate(150);
    Ok(candidates)
}

/// The Bankers board: the safest, most repeatable legs across the slate, ranked
/// by `banker_score` — high likelihood, recurring events, observed recency, sane
/// price, must-play. Deterministic, no model call. Anchor an acca with these.
#[tauri::command]
pub async fn get_bankers(
    state: State<'_, AppState>,
    fixtures: Vec<FixtureInput>,
    markets: Vec<String>,
) -> Result<Vec<Candidate>, String> {
    if fixtures.is_empty() {
        return Err("Select at least one match first.".to_string());
    }
    let markets: Vec<String> = if markets.is_empty() {
        ALL_MARKETS.iter().map(|s| s.to_string()).collect()
    } else {
        markets
    };
    let books = {
        let keys = state.keys.lock().map_err(|_| "keys lock")?;
        keys.books.clone()
    };

    let mut candidates = gather_candidates(&state, &fixtures, &markets, &books).await;
    let consensus = |x: &Candidate| x.pinnacle_prob.map(|p| 0.5 * p + 0.5 * x.est_prob).unwrap_or(x.est_prob);
    // Keep only genuinely likely, must-play legs — the bar for a "banker".
    candidates.retain(|x| consensus(x) >= 0.58 && !x.flags.iter().any(|f| f.contains("unlikely to feature")));
    candidates.sort_by(|a, b| {
        features::banker_score(b)
            .partial_cmp(&features::banker_score(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.truncate(120);
    Ok(candidates)
}

/// Build team + player candidates (with odds/EV) for the selected fixtures and
/// markets. Shared by the picks board and the accumulator ladder.
async fn gather_candidates(
    state: &AppState,
    fixtures: &[FixtureInput],
    markets: &[String],
    books: &[String],
) -> Vec<Candidate> {
    let player_groups: Vec<String> = markets.iter().filter(|m| is_player_market(m)).cloned().collect();
    let team_groups: Vec<String> = markets.iter().filter(|m| !is_player_market(m)).cloned().collect();
    let mut candidates: Vec<Candidate> = Vec::new();
    for fx in fixtures {
        let fixture_label = format!("{} vs {}", fx.home_team, fx.away_team);
        let injuries = fetch_injury_map(state, fx.fixture_id).await.unwrap_or_default();
        let fixture_odds = af::cached_get(state, "/odds", vec![("fixture", fx.fixture_id.to_string())], af::TTL_ODDS)
            .await
            .ok()
            .map(|j| crate::odds::parse_fixture_odds(&j, books))
            .unwrap_or_default();

        if !player_groups.is_empty() {
            let in_form = fetch_inform(state, fx.league_id, fx.season).await;
            for (team_id, team_name, is_home, opp) in [
                (fx.home_team_id, fx.home_team.clone(), true, fx.away_team.clone()),
                (fx.away_team_id, fx.away_team.clone(), false, fx.home_team.clone()),
            ] {
                let consistency = fetch_consistency(state, team_id, fx.season).await;
                let entries = fetch_team_players(state, team_id, fx.season).await.unwrap_or_default();
                let baselines = features::squad_baselines(&entries, fx.league_id);
                for entry in top_players(entries, 24) {
                    let availability = entry
                        .get("player")
                        .and_then(|p| p.get("id"))
                        .and_then(|v| v.as_i64())
                        .and_then(|pid| injuries.get(&pid).cloned())
                        .unwrap_or_else(|| "unknown".to_string());
                    let ctx = FixtureCtx {
                        fixture_label: fixture_label.clone(),
                        fixture_id: fx.fixture_id,
                        team: team_name.clone(),
                        opponent: opp.clone(),
                        is_home,
                        availability,
                    };
                    candidates.extend(features::build_player_candidates_entry(&entry, fx.league_id, &ctx, &player_groups, &in_form, &consistency, &baselines));
                }
            }
        }
        if !team_groups.is_empty() {
            let mut home = fetch_team_stats(state, fx.home_team_id, &fx.home_team, fx.league_id, fx.season).await.ok().flatten();
            let mut away = fetch_team_stats(state, fx.away_team_id, &fx.away_team, fx.league_id, fx.season).await.ok().flatten();
            if team_groups.iter().any(|m| matches!(m.as_str(), "tcorners" | "tshots" | "toutbox" | "tinbox" | "toffsides")) {
                let apply = |t: &mut crate::features::TeamStats, f: &TeamForm| {
                    t.corners_for = f.corners_for;
                    t.corners_against = f.corners_against;
                    t.shots_for = f.shots_for;
                    t.shots_against = f.shots_against;
                    t.outbox_for = f.outbox_for;
                    t.inbox_for = f.inbox_for;
                    t.offsides_for = f.offsides_for;
                };
                if let Some(h) = home.as_mut() {
                    if let Some(f) = fetch_team_form(state, fx.home_team_id).await {
                        apply(h, &f);
                    }
                }
                if let Some(a) = away.as_mut() {
                    if let Some(f) = fetch_team_form(state, fx.away_team_id).await {
                        apply(a, &f);
                    }
                }
            }
            if team_groups.iter().any(|m| matches!(m.as_str(), "tcards" | "bothcards" | "mostcards")) {
                let hc = fetch_team_cards(state, fx.home_team_id, fx.season).await;
                let ac = fetch_team_cards(state, fx.away_team_id, fx.season).await;
                if let Some(h) = home.as_mut() {
                    h.cards_for = hc.0;
                    h.cards_against = hc.1;
                    h.both_card_rate = hc.2;
                    h.most_card_rate = hc.3;
                }
                if let Some(a) = away.as_mut() {
                    a.cards_for = ac.0;
                    a.cards_against = ac.1;
                    a.both_card_rate = ac.2;
                    a.most_card_rate = ac.3;
                }
            }
            if let (Some(h), Some(a)) = (home, away) {
                candidates.extend(features::build_team_candidates(&h, &a, &fixture_label, fx.fixture_id, &team_groups));
            }
        }
        features::attach_odds(&mut candidates, &fixture_odds, &fixture_label, &fx.home_team);
    }
    // Attach cached tactical style tags (no model call — only shows if a prior
    // build with tactics-on computed them).
    let mut tagmap: HashMap<String, Option<String>> = HashMap::new();
    for c in candidates.iter_mut() {
        let tag = if let Some(t) = tagmap.get(&c.team) {
            t.clone()
        } else {
            let t = cached_tactics_tag(state, &c.team).await;
            tagmap.insert(c.team.clone(), t.clone());
            t
        };
        if let Some(t) = tag {
            c.flags.push(format!("style: {t}"));
        }
    }
    candidates
}

fn ladder_conf(p: f64) -> String {
    if p > 0.7 { "Very High" } else if p > 0.5 { "High" } else if p > 0.3 { "Medium" } else { "Low" }.to_string()
}

fn make_ladder_ticket(cands: &[Candidate], title: &str) -> Ticket {
    let mut legs: Vec<TicketLeg> = cands
        .iter()
        .map(|c| TicketLeg {
            r#match: c.fixture.clone(),
            fixture_id: c.fixture_id,
            market: c.market.clone(),
            selection: c.subject.clone(),
            team: team_badge_of(c),
            line: Some(c.line.clone()),
            est_prob: Some(c.est_prob),
            pinnacle_prob: c.pinnacle_prob,
            book_odds: c.book_odds,
            book: c.book.clone(),
            ev: c.ev,
            ev_source: c.ev_source.clone(),
        })
        .collect();
    legs.sort_by(|a, b| a.r#match.cmp(&b.r#match)); // group same-fixture legs
    let prob: f64 = cands.iter().map(|c| c.est_prob).product();
    let priced: Vec<f64> = cands.iter().filter_map(|c| c.book_odds).collect();
    let odds = if priced.len() == cands.len() && !priced.is_empty() {
        Some(round2(priced.iter().product()))
    } else {
        None
    };
    let mut per_fix: HashMap<&String, usize> = HashMap::new();
    for c in cands.iter() {
        *per_fix.entry(&c.fixture).or_insert(0) += 1;
    }
    let max_per_fix = per_fix.values().copied().max().unwrap_or(0);
    let kind = if cands.len() <= 1 {
        "Single"
    } else if per_fix.len() <= 1 {
        "SGP"
    } else if max_per_fix >= 2 {
        "SGP+"
    } else {
        "Acca"
    };
    Ticket {
        kind: kind.to_string(),
        title: title.to_string(),
        confidence: ladder_conf(prob),
        legs,
        combined_prob: Some(round4(prob)),
        combined_odds: odds,
        combined_ev: None,
        flags: vec!["ladder".to_string(), "estimated parlay price".to_string()],
        why: None,
    }
}

/// Within a (match, market-family) keep the line that best fits the threshold:
/// among lines at/above `min_prob`, the one with the MOST value (lowest prob);
/// if none clear it, the one closest to it.
fn ladder_prefer(new: &Candidate, cur: &Candidate, min_prob: f64) -> bool {
    let (n_ok, c_ok) = (new.est_prob >= min_prob, cur.est_prob >= min_prob);
    match (n_ok, c_ok) {
        (true, true) => new.est_prob < cur.est_prob,
        (true, false) => true,
        (false, true) => false,
        (false, false) => new.est_prob > cur.est_prob,
    }
}

/// Goal/result markets that constrain the final scoreline — used to reject
/// logically impossible ladder tickets.
fn is_goal_market(g: &str) -> bool {
    matches!(
        g,
        "ou25" | "tgoals" | "btts" | "win" | "dc" | "exactscore" | "goalsrange" | "h1goals" | "h2goals" | "ahandicap"
    )
}

/// Is candidate `c` consistent with a final score of (h home, a away)? Returns
/// true for non-goal markets (they don't constrain the score).
fn goal_ok(c: &Candidate, home_team: &str, h: i32, a: i32) -> bool {
    if !is_goal_market(&c.market_group) {
        return true;
    }
    let is_home = c.subject == home_team || c.team == home_team;
    let line = c.line.to_lowercase();
    // Floor of the half-line, e.g. "over 1.5" → 1.
    let x: i32 = line
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .parse::<f64>()
        .map(|v| v as i32)
        .unwrap_or(0);
    let over = line.contains("over");
    match c.market_group.as_str() {
        "ou25" => if over { h + a >= x + 1 } else { h + a <= x },
        "tgoals" => {
            let g = if is_home { h } else { a };
            if over { g >= x + 1 } else { g <= x }
        }
        "btts" => if line.contains("no") { h == 0 || a == 0 } else { h >= 1 && a >= 1 },
        "win" => if is_home { h > a } else { a > h },
        "dc" => {
            // Named-team double chance → that team doesn't lose. (homeaway is rare; treat leniently.)
            let (g, o) = if is_home { (h, a) } else { (a, h) };
            g >= o
        }
        "ahandicap" => if line.contains("to win") { if is_home { h > a } else { a > h } } else { true },
        "exactscore" => {
            let p: Vec<i32> = c.line.split('-').filter_map(|s| s.trim().parse().ok()).collect();
            p.len() == 2 && h == p[0] && a == p[1]
        }
        "goalsrange" => {
            let n: Vec<i32> = c.line.split(|ch: char| !ch.is_ascii_digit()).filter_map(|s| s.parse().ok()).collect();
            n.len() >= 2 && (h + a) >= n[0] && (h + a) <= n[1]
        }
        // Half goals bound the TOTAL from below on the over side only (1H/2H ≤ total).
        "h1goals" | "h2goals" => !over || h + a >= x + 1,
        _ => true,
    }
}

/// Can the goal/result legs of one fixture all be true at once? Checks a small
/// scoreline grid — false means the combination is impossible.
fn goals_satisfiable(legs: &[Candidate], home_team: &str) -> bool {
    let gr: Vec<&Candidate> = legs.iter().filter(|c| is_goal_market(&c.market_group)).collect();
    if gr.len() < 2 {
        return true;
    }
    for h in 0..=8 {
        for a in 0..=8 {
            if gr.iter().all(|c| goal_ok(c, home_team, h, a)) {
                return true;
            }
        }
    }
    false
}

/// Deterministic accumulator ladder: an all-match acca of the selected markets
/// plus `count` nested subsets, with one non-conflicting line per match/market
/// chosen against a probability threshold. No model call.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn build_ladder(
    state: State<'_, AppState>,
    fixtures: Vec<FixtureInput>,
    markets: Vec<String>,
    count: Option<u32>,
    min_prob: Option<f64>,
    scope: Option<String>,
    max_legs: Option<u32>,
    min_hit: Option<f64>,
    max_per_subject: Option<u32>,
    ou_side: Option<String>,
    min_legs: Option<u32>,
    exclude_sigs: Option<Vec<String>>,
    exclude_subjects: Option<Vec<String>>,
    seed_subjects: Option<Vec<String>>,
    variation: Option<u32>,
    min_odds: Option<f64>,
    max_odds: Option<f64>,
) -> Result<BuildResult, String> {
    if fixtures.is_empty() {
        return Err("Select at least one match first.".to_string());
    }
    let ou_side = ou_side.unwrap_or_else(|| "auto".to_string());
    let scope = scope.unwrap_or_else(|| "team".to_string()); // team | props | mixed
    let count = count.unwrap_or(5).clamp(1, 20) as usize;
    let min_prob = min_prob.unwrap_or(0.55).clamp(0.05, 0.97);
    let max_legs = max_legs.unwrap_or(8).clamp(2, 20) as usize;
    let min_hit = min_hit.unwrap_or(0.05).clamp(0.005, 0.9);
    let max_per_subject = max_per_subject.unwrap_or(2).clamp(1, 20) as usize;
    let min_legs = (min_legs.unwrap_or(2).clamp(2, 20) as usize).min(max_legs);
    // "Add more" support: skip ticket signatures already shown, drop voided
    // subjects entirely, optionally continue the diversity pool, vary the combos.
    let exclude_sigs: HashSet<String> = exclude_sigs.unwrap_or_default().into_iter().collect();
    let excl_subj: HashSet<String> =
        exclude_subjects.unwrap_or_default().into_iter().map(|s| crate::odds::fold(&s)).collect();
    let seed_subjects = seed_subjects.unwrap_or_default();
    let variation = variation.unwrap_or(0) as usize;

    let mut markets: Vec<String> = if markets.is_empty() {
        ALL_MARKETS.iter().map(|s| s.to_string()).collect()
    } else {
        markets
    };
    let had_player = markets.iter().any(|m| is_player_market(m));
    let had_team = markets.iter().any(|m| !is_player_market(m));
    match scope.as_str() {
        "team" => markets.retain(|m| !is_player_market(m)),
        "props" => markets.retain(|m| is_player_market(m)),
        _ => {} // mixed → keep both
    }
    if markets.is_empty() {
        let reason = match scope.as_str() {
            "team" => {
                if had_player {
                    "The ladder scope is 'Teams/Match', but you only selected PLAYER-prop markets. Switch the ladder scope to 'Props' or 'Mixed', or add a team market (Match Result, Goals O/U, Team Corners…)."
                } else {
                    "No team/match markets selected. Pick at least one (Match Result, Goals O/U, BTTS, Team Corners…)."
                }
            }
            "props" => {
                if had_team {
                    "The ladder scope is 'Props', but you only selected TEAM/match markets. Switch the ladder scope to 'Teams/Match' or 'Mixed', or add a player prop (Anytime Scorer, Shots on Target…)."
                } else {
                    "No player-prop markets selected. Pick at least one (Anytime Scorer, Shots on Target, Tackles…)."
                }
            }
            _ => "No markets selected — pick some markets above first.",
        };
        return Err(reason.to_string());
    }
    let books = {
        let keys = state.keys.lock().map_err(|_| "keys lock")?;
        keys.books.clone()
    };
    let mut candidates = gather_candidates(&state, &fixtures, &markets, &books).await;
    // Cache-only plausibility (no model call — the ladder stays deterministic).
    // If the user pre-scored in the background, every line gets a 1-5 weight that
    // tilts which lines lead each ticket.
    let _ = attach_plausibility(&state, &mut candidates, "claude-haiku-4-5", false).await;

    // Isolate Over or Under on the O/U families (goals, corners, shots, team
    // goals) when the user asks — otherwise both sides compete on probability.
    if ou_side == "over" || ou_side == "under" {
        let want_over = ou_side == "over";
        candidates.retain(|c| {
            let is_ou = matches!(c.market_group.as_str(), "ou25" | "tcorners" | "tshots" | "tgoals");
            !is_ou || c.line.to_lowercase().starts_with(if want_over { "over" } else { "under" })
        });
    }

    // POOL: one best line per PLAYER (so a player can't appear as both scorer
    // and SOT — those are nested/correlated), and one per (match, team-market
    // family) so we never put Over 1.5 AND Over 2.5 on the same ticket.
    let mut player_best: HashMap<String, Candidate> = HashMap::new();
    let mut team_best: HashMap<String, Candidate> = HashMap::new();
    for c in candidates {
        if c.subject_kind == "player" {
            let k = c.subject.to_lowercase();
            match player_best.get(&k) {
                Some(e) if !ladder_prefer(&c, e, min_prob) => {}
                _ => {
                    player_best.insert(k, c);
                }
            }
        } else {
            // Keep distinct lines per team (so SA's and Canada's team-goals are
            // BOTH available, and alt goal-lines / scorelines survive) — far more
            // pool variety. The in-ticket guards below stop conflicts.
            let k = format!("{}||{}||{}||{}", c.fixture, c.market_group, c.subject, c.line);
            match team_best.get(&k) {
                Some(e) if !ladder_prefer(&c, e, min_prob) => {}
                _ => {
                    team_best.insert(k, c);
                }
            }
        }
    }
    // Rank legs by probability, nudged by the cached plausibility weight (+0.08 at
    // 5, −0.08 at 1) so more realistic lines lead each ticket.
    let lscore = |c: &Candidate| c.est_prob + c.plausibility.map(|p| (p as f64 - 3.0) * 0.04).unwrap_or(0.0);
    let cmp_desc = |a: &Candidate, b: &Candidate| lscore(b).partial_cmp(&lscore(a)).unwrap_or(std::cmp::Ordering::Equal);
    let mut player_legs: Vec<Candidate> = player_best.into_values().collect();
    let mut team_legs: Vec<Candidate> = team_best.into_values().collect();
    player_legs.sort_by(cmp_desc);
    team_legs.sort_by(cmp_desc);

    // In "mixed" scope, INTERLEAVE players and teams (round-robin by rank) so a
    // ticket gets a balance instead of being flooded by high-prob player props.
    let legs: Vec<Candidate> = if scope == "mixed" && !player_legs.is_empty() && !team_legs.is_empty() {
        let (mut pi, mut ti, mut out) = (0usize, 0usize, Vec::new());
        while pi < player_legs.len() || ti < team_legs.len() {
            if pi < player_legs.len() {
                out.push(player_legs[pi].clone());
                pi += 1;
            }
            if ti < team_legs.len() {
                out.push(team_legs[ti].clone());
                ti += 1;
            }
        }
        out
    } else {
        let mut v = player_legs;
        v.extend(team_legs);
        v.sort_by(cmp_desc);
        v
    };
    // Drop any subjects the user voided (e.g. a player ruled out) so they can't
    // reappear in newly-added tickets. Also apply the per-leg odds sweet-spot.
    let odds_lo = min_odds.unwrap_or(1.0);
    let odds_hi = max_odds.unwrap_or(1000.0);
    let band_active = odds_lo > 1.01 || odds_hi < 999.0;
    let mut legs: Vec<Candidate> = legs
        .into_iter()
        .filter(|c| !excl_subj.contains(&crate::odds::fold(&c.subject)))
        .filter(|c| !band_active || c.book_odds.map_or(true, |o| o >= odds_lo && o <= odds_hi))
        .collect();
    if legs.is_empty() {
        return Err("No usable lines for the selected markets and matches.".to_string());
    }
    // "Add more" variation: rotate the (prob-sorted) pool so a different set of
    // legs leads each band → genuinely new combinations, not the same ladder.
    if variation > 0 {
        let off = (variation.wrapping_mul(3)) % legs.len();
        legs.rotate_left(off);
    }

    // fixture label → home team name, for the scoreline contradiction guard.
    let home_by_fix: HashMap<String, String> = fixtures
        .iter()
        .map(|f| (format!("{} vs {}", f.home_team, f.away_team), f.home_team.clone()))
        .collect();

    // Diversity key: the entity that shouldn't appear in too many tickets — a
    // player, a named team, or (for match-level BTTS/O-U) the fixture. Applied to
    // players AND teams so one side can't sit on every ticket.
    let dkey = |c: &Candidate| -> String {
        let s = crate::odds::fold(&c.subject);
        if s == "both teams" || s == "match" {
            format!("m:{}", c.fixture)
        } else {
            format!("e:{s}")
        }
    };
    let band = |p: f64| -> &'static str {
        if p >= 0.6 { "safe" } else if p >= 0.35 { "moderate" } else if p >= 0.12 { "risky" } else { "long" }
    };

    // Build `count` tickets at geometrically-spaced hit-chance targets (safe →
    // risky). Each ticket: ≤ max_legs, no repeated subject, and no player in more
    // than `max_per_subject` tickets total — so one star can't sink the slate.
    let hi = 0.78f64.max(min_hit);
    let mut subj_used: HashMap<String, usize> = HashMap::new();
    // Continue the diversity pool across an "add more" (unless the user reset it):
    // pre-charge subjects already used by the existing tickets.
    for s in &seed_subjects {
        *subj_used.entry(format!("e:{}", crate::odds::fold(s))).or_insert(0) += 1;
    }
    // Don't reproduce tickets already on screen.
    let mut seen_sigs: HashSet<String> = exclude_sigs.clone();
    let mut tickets: Vec<Ticket> = Vec::new();
    let pool_len = legs.len();
    let mut cap = max_per_subject;
    let mut attempt = 0usize;
    let max_attempts = count * 8 + 40; // retry budget so a thin slot can't lose a ticket
    while tickets.len() < count && attempt < max_attempts {
        // Geometric hit-target by how many we've built so far (safe → risky). With
        // a high min-legs the target only gates EXTRA legs; min-legs is always met.
        let idx = tickets.len().min(count.saturating_sub(1));
        let target = if count <= 1 {
            min_hit
        } else {
            hi * (min_hit / hi).powf(idx as f64 / (count - 1) as f64)
        };
        // Vary the starting point each attempt → explore different combinations.
        let start = attempt.wrapping_mul(3) % pool_len;
        let mut chosen: Vec<Candidate> = Vec::new();
        let mut prod = 1.0f64;
        let mut in_ticket: HashSet<String> = HashSet::new();
        for j in 0..pool_len {
            if chosen.len() >= max_legs {
                break;
            }
            let c = &legs[(start + j) % pool_len];
            let fkey = format!("{}|{}", c.fixture, c.market_group);
            let k = dkey(c);
            if in_ticket.contains(&fkey)
                || in_ticket.contains(&k)
                || *subj_used.get(&k).unwrap_or(&0) >= cap
            {
                continue;
            }
            // Contradiction guard: the goal/result legs of this fixture + c must be
            // jointly satisfiable by SOME scoreline — no impossible tickets.
            if is_goal_market(&c.market_group) {
                let home = home_by_fix.get(&c.fixture).map(|s| s.as_str()).unwrap_or("");
                let mut same: Vec<Candidate> =
                    chosen.iter().filter(|x| x.fixture == c.fixture).cloned().collect();
                same.push(c.clone());
                if !goals_satisfiable(&same, home) {
                    continue;
                }
            }
            // Fill to min_legs regardless of the target; the target only stops us
            // from adding MORE legs once the floor is met.
            if chosen.len() >= min_legs && prod * c.est_prob < target {
                break;
            }
            prod *= c.est_prob;
            chosen.push(c.clone());
            in_ticket.insert(fkey);
            in_ticket.insert(k);
        }
        attempt += 1;
        if chosen.len() < min_legs {
            // Stuck — the per-subject cap is starving us; relax it so the count
            // still gets met (the count the user asked for wins over diversity).
            if attempt % count.max(1) == 0 && cap < 8 {
                cap += 1;
            }
            continue;
        }
        let mut sig: Vec<String> = chosen.iter().map(|c| format!("{}|{}|{}", c.market, c.subject, c.line)).collect();
        sig.sort();
        if !seen_sigs.insert(sig.join("##")) {
            continue;
        }
        for c in &chosen {
            *subj_used.entry(dkey(c)).or_insert(0) += 1;
        }
        let title = format!("Ladder · {} legs · ~{}% ({})", chosen.len(), (prod * 100.0).round() as i64, band(prod));
        tickets.push(make_ladder_ticket(&chosen, &title));
    }
    tickets.sort_by_key(|t| t.legs.len());

    // Record in the ledger under the "ladder" strategy.
    {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        let day = af::today();
        for t in &tickets {
            let mut sig: Vec<String> = t
                .legs
                .iter()
                .map(|l| format!("{}|{}|{}", l.market, l.selection, l.line.clone().unwrap_or_default()))
                .collect();
            sig.sort();
            if let Ok(tj) = serde_json::to_string(t) {
                let _ = db::gen_add(&conn, af::now_ts(), &day, "ladder", false, &t.kind, &sig.join("##"), &tj, t.combined_odds);
            }
        }
    }

    let mut notes = vec![format!(
        "Accumulator ladder — {} candidate lines (one per match/market, ≥{}% preferred); {} tickets, {}-{} legs, ≥{}% hit chance. Deterministic, no model call.",
        legs.len(),
        (min_prob * 100.0).round() as i64,
        tickets.len(),
        min_legs,
        max_legs,
        (min_hit * 100.0).round() as i64
    )];
    if tickets.len() < count {
        notes.push(format!(
            "Only {} of {} tickets — the pool ({} lines) can't form more distinct {}-leg combos. Add fixtures/markets, lower the min legs, widen the odds band, or switch the markets scope to 'mixed'/'props'.",
            tickets.len(), count, legs.len(), min_legs
        ));
    }
    Ok(BuildResult {
        tickets,
        forecast: None,
        forecasts: vec![],
        data_quality_notes: notes,
        context_notes: vec![],
        from_cache: false,
        grok_used: false,
        grok_digest: None,
    })
}

/// Evaluate user-built tickets with a (usually cheaper) model — analysis + risks.
#[tauri::command]
pub async fn evaluate_tickets(
    state: State<'_, AppState>,
    tickets: Vec<serde_json::Value>,
    model: Option<String>,
    leagues: Option<HashMap<i64, String>>,
) -> Result<Vec<TicketEval>, String> {
    if tickets.is_empty() {
        return Ok(vec![]);
    }
    let model = model
        .filter(|m| llm::is_allowed_analysis_model(m))
        .unwrap_or_else(|| "claude-haiku-4-5".to_string());
    let leagues = leagues.unwrap_or_default();

    let mut rows = Vec::new();
    let mut n = 0usize;
    for (i, t) in tickets.iter().enumerate() {
        let tk: Ticket = serde_json::from_value(t.clone()).map_err(|e| e.to_string())?;
        let legs: Vec<serde_json::Value> = tk
            .legs
            .iter()
            .map(|l| {
                // Tell the model which COMPETITION this is (so it doesn't assume a
                // friendly/qualifier) — looked up per leg by fixture id.
                let comp = leagues.get(&l.fixture_id).cloned().unwrap_or_default();
                serde_json::json!({
                    "match": l.r#match, "competition": comp, "sel": l.selection, "market": l.market, "line": l.line,
                    "est": l.est_prob, "pin": l.pinnacle_prob, "odds": l.book_odds, "ev": l.ev
                })
            })
            .collect();
        rows.push(serde_json::json!({
            "i": i + 1, "type": tk.kind, "legs": legs,
            "combined_odds": tk.combined_odds, "combined_prob": tk.combined_prob
        }));
        n += 1;
    }
    let compact = serde_json::to_string(&rows).unwrap_or_default();
    let (mut evals, gin, gout) = llm::evaluate(&state, &model, &compact).await?;
    {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        db::usage_add(&conn, af::now_ts(), &model, gin, gout, "eval")?;
    }
    while evals.len() < n {
        evals.push(TicketEval {
            analysis: "(no analysis returned)".to_string(),
            leg_notes: vec![],
            risks: vec![],
            recommendations: vec![],
            verdict: String::new(),
        });
    }
    evals.truncate(n);
    Ok(evals)
}

// ---------- saved tickets ----------

#[tauri::command]
pub fn save_ticket(
    state: State<AppState>,
    selection_json: String,
    result_json: String,
    notes: String,
) -> Result<i64, String> {
    let conn = state.db.lock().map_err(|_| "db lock")?;
    db::save_ticket(&conn, af::now_ts(), &selection_json, &result_json, &notes)
}

#[tauri::command]
pub fn list_tickets(state: State<AppState>) -> Result<Vec<SavedTicket>, String> {
    let conn = state.db.lock().map_err(|_| "db lock")?;
    db::list_tickets(&conn)
}

#[tauri::command]
pub fn list_grok_log(state: State<AppState>) -> Result<Vec<GrokLogEntry>, String> {
    let conn = state.db.lock().map_err(|_| "db lock")?;
    let rows = db::grok_log_list(&conn)?;
    Ok(rows
        .into_iter()
        .map(|(id, created_at, matches, digest)| GrokLogEntry {
            id,
            created_at,
            matches,
            digest,
        })
        .collect())
}

// ---------- cost breakdown ----------

fn cost_from(rows: Vec<(String, i64, i64)>) -> (f64, i64) {
    let mut cost = 0.0;
    let mut tokens = 0;
    for (m, i, o) in rows {
        cost += llm::cost_usd(&m, i, o);
        tokens += i + o;
    }
    ((cost * 10000.0).round() / 10000.0, tokens)
}

#[tauri::command]
pub fn usage_breakdown(state: State<AppState>) -> Result<UsageBreakdown, String> {
    // Calendar-based windows (UTC): today = since 00:00 today, week = since
    // Monday, month = since the 1st — not a rolling 24h/7d.
    use chrono::{Datelike, TimeZone, Utc};
    let now_dt = Utc::now();
    let day_start = Utc
        .with_ymd_and_hms(now_dt.year(), now_dt.month(), now_dt.day(), 0, 0, 0)
        .single()
        .map(|d| d.timestamp())
        .unwrap_or(0);
    let week_start = day_start - now_dt.weekday().num_days_from_monday() as i64 * 86_400;
    let month_start = Utc
        .with_ymd_and_hms(now_dt.year(), now_dt.month(), 1, 0, 0, 0)
        .single()
        .map(|d| d.timestamp())
        .unwrap_or(0);
    let conn = state.db.lock().map_err(|_| "db lock")?;
    let (today, today_tokens) = cost_from(db::usage_since(&conn, day_start)?);
    let (week, week_tokens) = cost_from(db::usage_since(&conn, week_start)?);
    let (month, _) = cost_from(db::usage_since(&conn, month_start)?);
    let (lifetime, lifetime_tokens) = cost_from(db::usage_by_model(&conn)?);

    Ok(UsageBreakdown {
        today,
        week,
        month,
        lifetime,
        today_tokens,
        week_tokens,
        lifetime_tokens,
        grok_today: db::grok_cost_since(&conn, day_start)?,
        grok_week: db::grok_cost_since(&conn, week_start)?,
        grok_month: db::grok_cost_since(&conn, month_start)?,
        grok_lifetime: db::grok_cost_since(&conn, 0)?,
    })
}

// ---------- bankroll + bet tracking ----------

fn bankroll_view(conn: &rusqlite::Connection) -> Result<BankrollView, String> {
    let bankroll: f64 = db::setting_get(conn, "bankroll")?
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let rows = db::list_placed(conn)?;
    let mut staked_open = 0.0;
    let mut pnl = 0.0;
    let mut open = 0i64;
    let mut settled = 0i64;
    for r in &rows {
        if r.settled {
            settled += 1;
            pnl += r.returns - r.stake;
        } else {
            open += 1;
            staked_open += r.stake;
        }
    }
    Ok(BankrollView {
        bankroll,
        staked_open,
        pnl: (pnl * 100.0).round() / 100.0,
        current: ((bankroll + pnl - staked_open) * 100.0).round() / 100.0,
        open_count: open,
        settled_count: settled,
    })
}

#[tauri::command]
pub fn get_bankroll(state: State<AppState>) -> Result<BankrollView, String> {
    let conn = state.db.lock().map_err(|_| "db lock")?;
    bankroll_view(&conn)
}

#[tauri::command]
pub fn set_bankroll(state: State<AppState>, amount: f64) -> Result<BankrollView, String> {
    let conn = state.db.lock().map_err(|_| "db lock")?;
    db::setting_set(&conn, "bankroll", &amount.to_string())?;
    bankroll_view(&conn)
}

fn row_to_bet(r: &db::PlacedRow) -> Result<PlacedBet, String> {
    let ticket: Ticket = serde_json::from_str(&r.ticket_json).map_err(|e| e.to_string())?;
    let leg_results: Vec<LegResult> =
        serde_json::from_str(&r.leg_results_json).unwrap_or_default();
    Ok(PlacedBet {
        id: r.id,
        created_at: r.created_at,
        day: r.day.clone(),
        ticket,
        stake: r.stake,
        status: r.status.clone(),
        returns: r.returns,
        leg_results,
        settled: r.settled,
        grok_used: r.grok_used,
        strategy: r.strategy.clone(),
    })
}

#[tauri::command]
pub fn place_bet(
    state: State<AppState>,
    ticket: serde_json::Value,
    stake: f64,
    odds: Option<f64>,
    grok_used: Option<bool>,
    strategy: Option<String>,
) -> Result<i64, String> {
    // Inject the actual odds the user took (if given) so P&L is accurate.
    let mut t: Ticket = serde_json::from_value(ticket).map_err(|e| e.to_string())?;
    if let Some(o) = odds {
        if o > 0.0 {
            t.combined_odds = Some(o);
        }
    }
    let ticket_json = serde_json::to_string(&t).map_err(|e| e.to_string())?;
    let conn = state.db.lock().map_err(|_| "db lock")?;
    db::place_bet(
        &conn,
        af::now_ts(),
        &af::today(),
        &ticket_json,
        stake,
        grok_used.unwrap_or(false),
        strategy.as_deref().unwrap_or("value"),
    )
}

#[tauri::command]
pub fn list_bets(state: State<AppState>) -> Result<Vec<PlacedBet>, String> {
    let conn = state.db.lock().map_err(|_| "db lock")?;
    let rows = db::list_placed(&conn)?;
    rows.iter().map(row_to_bet).collect()
}

/// Minimum graded legs before the calibration shrink is trusted/applied.
const CALIB_MIN_N: i64 = 30;

/// Pair every settled leg's predicted prob with its actual outcome and measure
/// how well-calibrated our `est_prob` is.
/// (predicted, outcome 0/1) pairs from settled PLACED bets.
fn pairs_from_bets(bets: &[PlacedBet]) -> Vec<(f64, f64)> {
    let mut pairs = Vec::new();
    for b in bets {
        if !b.settled {
            continue;
        }
        for (leg, res) in b.ticket.legs.iter().zip(b.leg_results.iter()) {
            if let (Some(p), Some(won)) = (leg.est_prob, res.won) {
                if p > 0.0 && p < 1.0 {
                    pairs.push((p, if won { 1.0 } else { 0.0 }));
                }
            }
        }
    }
    pairs
}

/// (predicted, outcome 0/1) pairs from the settled GENERATED ledger — far more
/// data than you'd ever place, so calibration learns much faster.
fn pairs_from_generated(conn: &rusqlite::Connection) -> Vec<(f64, f64)> {
    let mut pairs = Vec::new();
    for (tj, lrj) in db::gen_settled(conn).unwrap_or_default() {
        let legs = serde_json::from_str::<Ticket>(&tj).map(|t| t.legs).unwrap_or_default();
        let results: Vec<crate::models::LegResult> = serde_json::from_str(&lrj).unwrap_or_default();
        for (leg, res) in legs.iter().zip(results.iter()) {
            if let (Some(p), Some(won)) = (leg.est_prob, res.won) {
                if p > 0.0 && p < 1.0 {
                    pairs.push((p, if won { 1.0 } else { 0.0 }));
                }
            }
        }
    }
    pairs
}

fn calibration_from_pairs(pairs: Vec<(f64, f64)>) -> CalibrationReport {
    let n = pairs.len() as i64;

    // 5 reliability bins.
    let edges = [0.0, 0.2, 0.4, 0.6, 0.8, 1.0001];
    let mut bins = Vec::new();
    for w in edges.windows(2) {
        let (lo, hi) = (w[0], w[1]);
        let inb: Vec<&(f64, f64)> = pairs.iter().filter(|(p, _)| *p >= lo && *p < hi).collect();
        if inb.is_empty() {
            continue;
        }
        let cnt = inb.len() as f64;
        bins.push(CalBin {
            lo,
            hi: hi.min(1.0),
            predicted_avg: inb.iter().map(|(p, _)| p).sum::<f64>() / cnt,
            actual_rate: inb.iter().map(|(_, o)| o).sum::<f64>() / cnt,
            n: inb.len() as i64,
        });
    }

    // Slope through origin of (outcome−0.5) on (pred−0.5): the raw reliability.
    let (mut num, mut den) = (0.0, 0.0);
    for (p, o) in &pairs {
        let x = p - 0.5;
        num += x * (o - 0.5);
        den += x * x;
    }
    let raw_slope = if den > 1e-9 { (num / den).clamp(0.3, 1.2) } else { 1.0 };

    // Sample-size-aware (empirical-Bayes) shrinkage of the slope toward 1.0
    // (no adjustment): trust the measured slope more as evidence accumulates,
    // so a handful of unlucky legs can't yank everyone's probabilities around.
    // w = n / (n + K); at n=30 w≈0.20, at n=500 w≈0.81.
    const CALIB_K: f64 = 120.0;
    let w = n as f64 / (n as f64 + CALIB_K);
    let lambda = 1.0 + w * (raw_slope - 1.0);
    let applied = n >= CALIB_MIN_N;

    let verdict = if n < CALIB_MIN_N {
        format!("Need more settled legs to assess calibration ({n}/{CALIB_MIN_N}).")
    } else if raw_slope < 0.9 {
        format!(
            "Overconfident — shrinking edges ~{}% toward 50/50 (weighted {}% to {n} legs of evidence).",
            ((1.0 - lambda) * 100.0).round(),
            (w * 100.0).round()
        )
    } else if raw_slope > 1.1 {
        "Underconfident — your real edge is a touch bigger than estimated.".to_string()
    } else {
        "Well calibrated — no material adjustment needed.".to_string()
    };

    CalibrationReport { bins, lambda, n, verdict, applied }
}

#[tauri::command]
pub fn calibration(state: State<AppState>) -> Result<CalibrationReport, String> {
    let conn = state.db.lock().map_err(|_| "db lock")?;
    let rows = db::list_placed(&conn)?;
    let bets: Vec<PlacedBet> = rows.iter().filter_map(|r| row_to_bet(r).ok()).collect();
    let mut pairs = pairs_from_bets(&bets);
    pairs.extend(pairs_from_generated(&conn)); // learn from the paper-trade too
    Ok(calibration_from_pairs(pairs))
}

/// The shrink factor to apply in builds (1.0 = none) plus the graded-leg count.
fn calibration_shrink(state: &AppState) -> (f64, i64) {
    let report = {
        let conn = match state.db.lock() {
            Ok(c) => c,
            Err(_) => return (1.0, 0),
        };
        let rows = db::list_placed(&conn).unwrap_or_default();
        let bets: Vec<PlacedBet> = rows.iter().filter_map(|r| row_to_bet(r).ok()).collect();
        let mut pairs = pairs_from_bets(&bets);
        pairs.extend(pairs_from_generated(&conn));
        calibration_from_pairs(pairs)
    };
    if report.applied {
        (report.lambda, report.n)
    } else {
        (1.0, report.n)
    }
}

// ---------- generated-tickets ledger ----------

fn build_gen_report(state: &AppState) -> Result<Vec<GenReportRow>, String> {
    let conn = state.db.lock().map_err(|_| "db lock")?;
    let rows = db::gen_report(&conn)?;
    Ok(rows
        .into_iter()
        .map(|(strategy, grok_used, total, settled, won, priced_n, ret_sum)| {
            let hit_rate = if settled > 0 { won as f64 / settled as f64 } else { 0.0 };
            let roi = if priced_n > 0 {
                Some(((ret_sum - priced_n as f64) / priced_n as f64 * 1000.0).round() / 1000.0)
            } else {
                None
            };
            GenReportRow { strategy, grok_used, total, settled, won, hit_rate, roi }
        })
        .collect())
}

/// Grade EVERY generated ticket whose matches have finished, then return the
/// per-strategy report. A ticket settles only once all its legs are gradeable.
#[tauri::command]
pub async fn settle_generated(state: State<'_, AppState>) -> Result<Vec<GenReportRow>, String> {
    let rows = {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        db::gen_unsettled(&conn)?
    };
    for row in rows {
        let t: Ticket = match serde_json::from_str(&row.ticket_json) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let results = settle::grade_legs(&state, &t.legs).await;
        if results.is_empty() || results.iter().any(|r| r.won.is_none()) {
            continue; // not all legs gradeable yet
        }
        let won = results.iter().all(|r| r.won == Some(true));
        let lr = serde_json::to_string(&results).unwrap_or_else(|_| "[]".to_string());
        let conn = state.db.lock().map_err(|_| "db lock")?;
        let _ = db::gen_mark_settled(&conn, row.id, won, &lr);
    }
    build_gen_report(&state)
}

#[tauri::command]
pub fn generated_report(state: State<AppState>) -> Result<Vec<GenReportRow>, String> {
    build_gen_report(&state)
}

/// Same report but grouped by ticket KIND (Single / SGP / SGP+).
#[tauri::command]
pub fn generated_report_by_kind(state: State<AppState>) -> Result<Vec<GenReportRow>, String> {
    let conn = state.db.lock().map_err(|_| "db lock")?;
    let rows = db::gen_report_by_kind(&conn)?;
    Ok(rows
        .into_iter()
        .map(|(kind, total, settled, won, priced_n, ret_sum)| {
            let hit_rate = if settled > 0 { won as f64 / settled as f64 } else { 0.0 };
            let roi = if priced_n > 0 {
                Some(((ret_sum - priced_n as f64) / priced_n as f64 * 1000.0).round() / 1000.0)
            } else {
                None
            };
            // Reuse the row shape: strategy holds the kind label.
            GenReportRow { strategy: kind, grok_used: false, total, settled, won, hit_rate, roi }
        })
        .collect())
}

/// Per-market (per-pick) hit-rate vs the model's predicted rate, from every
/// settled GENERATED leg — this is where biases show up (e.g. "team corners over
/// predicted 45% but lands 30%" → the model is over-rating that market).
#[tauri::command]
pub fn generated_report_by_market(state: State<AppState>) -> Result<Vec<MarketReportRow>, String> {
    let conn = state.db.lock().map_err(|_| "db lock")?;
    // market -> (settled, won, pred_sum, margin_sum, margin_n, near_miss_losses)
    let mut agg: HashMap<String, (i64, i64, f64, f64, i64, i64)> = HashMap::new();
    for (tj, lrj) in db::gen_settled(&conn)? {
        let legs = serde_json::from_str::<Ticket>(&tj).map(|t| t.legs).unwrap_or_default();
        let results: Vec<crate::models::LegResult> = serde_json::from_str(&lrj).unwrap_or_default();
        for (leg, res) in legs.iter().zip(results.iter()) {
            if let Some(won) = res.won {
                let e = agg.entry(leg.market.clone()).or_insert((0, 0, 0.0, 0.0, 0, 0));
                e.0 += 1;
                if won {
                    e.1 += 1;
                }
                e.2 += leg.est_prob.unwrap_or(0.0);
                if let Some(m) = res.margin {
                    e.3 += m;
                    e.4 += 1;
                    if !won && m > -1.0 {
                        e.5 += 1; // a near-miss loss (within 1 of the line)
                    }
                }
            }
        }
    }
    let mut out: Vec<MarketReportRow> = agg
        .into_iter()
        .map(|(market, (settled, won, psum, msum, mn, near))| MarketReportRow {
            market,
            settled,
            won,
            hit_rate: if settled > 0 { round4(won as f64 / settled as f64) } else { 0.0 },
            predicted: if settled > 0 { round4(psum / settled as f64) } else { 0.0 },
            avg_margin: if mn > 0 { Some((msum / mn as f64 * 100.0).round() / 100.0) } else { None },
            near_misses: near,
        })
        .collect();
    out.sort_by(|a, b| b.settled.cmp(&a.settled));
    Ok(out)
}

/// Export all meaningful app state (bets, generated ledger, saved picks, stats,
/// settings) as a portable JSON backup string.
#[tauri::command]
pub fn export_data(state: State<AppState>) -> Result<String, String> {
    let conn = state.db.lock().map_err(|_| "db lock")?;
    db::export_all(&conn)
}

/// Replace current state with a previously exported backup. Returns rows loaded.
#[tauri::command]
pub fn import_data(state: State<AppState>, json: String) -> Result<usize, String> {
    let conn = state.db.lock().map_err(|_| "db lock")?;
    db::import_all(&conn, &json)
}

/// Reset to a fresh state — wipe every bet, generated ticket, saved pick, stat and
/// cache (so calibration/learning restarts). API keys and settings are kept.
#[tauri::command]
pub fn reset_data(state: State<AppState>) -> Result<(), String> {
    let conn = state.db.lock().map_err(|_| "db lock")?;
    db::reset_all(&conn)
}

/// Token spend grouped by model AND purpose (build / eval / plausibility / ingest
/// / tactics) — so the ledger shows what each model contributed to the data.
#[tauri::command]
pub fn usage_by_purpose(state: State<AppState>) -> Result<Vec<ModelPurposeRow>, String> {
    let conn = state.db.lock().map_err(|_| "db lock")?;
    let mut out: Vec<ModelPurposeRow> = db::usage_by_purpose(&conn)?
        .into_iter()
        .map(|(model, purpose, i, o)| ModelPurposeRow {
            cost_usd: (llm::cost_usd(&model, i, o) * 10000.0).round() / 10000.0,
            model,
            purpose,
            input_tokens: i,
            output_tokens: o,
        })
        .collect();
    out.sort_by(|a, b| b.cost_usd.partial_cmp(&a.cost_usd).unwrap_or(std::cmp::Ordering::Equal));
    Ok(out)
}

/// Write the bundled browser extension to the user's Downloads so they can load it
/// unpacked. Returns the folder path.
#[tauri::command]
pub fn export_extension(app: tauri::AppHandle) -> Result<String, String> {
    use tauri::Manager;
    let dir = app
        .path()
        .download_dir()
        .or_else(|_| app.path().app_data_dir())
        .map_err(|e| format!("no folder: {e}"))?
        .join("powabetz-extension");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let files: [(&str, &[u8]); 5] = [
        ("manifest.json", include_bytes!("../../extension/manifest.json")),
        ("background.js", include_bytes!("../../extension/background.js")),
        ("popup.html", include_bytes!("../../extension/popup.html")),
        ("popup.js", include_bytes!("../../extension/popup.js")),
        ("icon.png", include_bytes!("../../extension/icon.png")),
    ];
    for (name, bytes) in files {
        std::fs::write(dir.join(name), bytes).map_err(|e| e.to_string())?;
    }
    // Reveal the folder so it's tangible (it IS the extension — load it unpacked).
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener).arg(&dir).spawn();
    Ok(dir.to_string_lossy().to_string())
}

// ---------- in-play / live ----------

const TTL_LIVE_SHORT: i64 = 15; // live data: refresh after 15s, reuse within

fn pois_ge(k: u32, l: f64) -> f64 {
    let mut term = (-l).exp();
    let mut cdf = term;
    for i in 1..k {
        term *= l / i as f64;
        cdf += term;
    }
    (1.0 - cdf).clamp(0.0, 1.0)
}

/// First in-play odd whose market contains ALL keywords and whose selection
/// contains `sel` (all lowercased) — for matching an estimate to a live price.
fn find_odd<'a>(odds: &'a [LiveOdd], market_kw: &[&str], sel: &str) -> Option<&'a LiveOdd> {
    odds.iter().find(|o| {
        let ml = o.market.to_lowercase();
        let sl = o.selection.to_lowercase();
        market_kw.iter().all(|k| ml.contains(k)) && sl.contains(sel)
    })
}

/// (edge vs the matched price, "selection @ odds") for displaying value on an estimate.
fn edge_of(p: f64, o: Option<&LiveOdd>) -> (Option<f64>, Option<String>) {
    match o {
        Some(o) => (Some(((p - o.implied) * 10000.0).round() / 10000.0), Some(format!("{} @ {:.2}", o.selection, o.odds))),
        None => (None, None),
    }
}

/// All matches in play right now (HT first — that's where the value is). On demand.
#[tauri::command]
pub async fn live_fixtures(state: State<'_, AppState>) -> Result<Vec<LiveFixture>, String> {
    let j = af::cached_get(&state, "/fixtures", vec![("live", "all".to_string())], TTL_LIVE_SHORT).await?;
    let mut out = Vec::new();
    for f in response_array(&j) {
        let fx = f.get("fixture");
        let stt = fx.and_then(|x| x.get("status"));
        out.push(LiveFixture {
            fixture_id: fx.and_then(|x| x.get("id")).and_then(|v| v.as_i64()).unwrap_or(0),
            league_id: f.get("league").and_then(|l| l.get("id")).and_then(|v| v.as_i64()).unwrap_or(0),
            league_name: f.get("league").and_then(|l| l.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string(),
            season: f.get("league").and_then(|l| l.get("season")).and_then(|v| v.as_i64()).unwrap_or(0),
            home_team: f.get("teams").and_then(|t| t.get("home")).and_then(|h| h.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string(),
            away_team: f.get("teams").and_then(|t| t.get("away")).and_then(|h| h.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string(),
            home_team_id: f.get("teams").and_then(|t| t.get("home")).and_then(|h| h.get("id")).and_then(|v| v.as_i64()).unwrap_or(0),
            away_team_id: f.get("teams").and_then(|t| t.get("away")).and_then(|h| h.get("id")).and_then(|v| v.as_i64()).unwrap_or(0),
            status: stt.and_then(|s| s.get("short")).and_then(|v| v.as_str()).unwrap_or("").to_string(),
            elapsed: stt.and_then(|s| s.get("elapsed")).and_then(|v| v.as_i64()).unwrap_or(0),
            home_goals: f.get("goals").and_then(|g| g.get("home")).and_then(|v| v.as_i64()).unwrap_or(0),
            away_goals: f.get("goals").and_then(|g| g.get("away")).and_then(|v| v.as_i64()).unwrap_or(0),
            has_stats: false,
        });
    }
    // HT first (most actionable), then later games first.
    out.sort_by(|a, b| (a.status != "HT").cmp(&(b.status != "HT")).then(b.elapsed.cmp(&a.elapsed)));
    Ok(out)
}

/// A current-state snapshot for one live match: stats, events, our remaining-time
/// estimates, and the in-play odds — so you can find value at half-time.
#[tauri::command]
pub async fn live_snapshot(state: State<'_, AppState>, fixture: LiveFixture) -> Result<LiveSnapshot, String> {
    live_snapshot_inner(&state, fixture).await
}

async fn live_snapshot_inner(state: &AppState, fixture: LiveFixture) -> Result<LiveSnapshot, String> {
    let fid = fixture.fixture_id;
    // Refresh the live score/minute.
    let (mut status, mut elapsed, mut hg, mut ag) = (fixture.status.clone(), fixture.elapsed, fixture.home_goals, fixture.away_goals);
    if let Ok(fj) = af::cached_get(&state, "/fixtures", vec![("id", fid.to_string())], TTL_LIVE_SHORT).await {
        if let Some(f) = response_array(&fj).into_iter().next() {
            let stt = f.get("fixture").and_then(|x| x.get("status"));
            status = stt.and_then(|s| s.get("short")).and_then(|v| v.as_str()).unwrap_or(&status).to_string();
            elapsed = stt.and_then(|s| s.get("elapsed")).and_then(|v| v.as_i64()).unwrap_or(elapsed);
            hg = f.get("goals").and_then(|g| g.get("home")).and_then(|v| v.as_i64()).unwrap_or(hg);
            ag = f.get("goals").and_then(|g| g.get("away")).and_then(|v| v.as_i64()).unwrap_or(ag);
        }
    }

    // Live in-match stats (big leagues only — patchy elsewhere).
    let want = ["Shots on Goal", "Total Shots", "Ball Possession", "Corner Kicks", "Fouls", "Yellow Cards", "Offsides", "Total passes"];
    let mut stats: Vec<LiveTeamStat> = Vec::new();
    let mut corners_total = 0.0;
    let (mut home_sot, mut away_sot): (Option<f64>, Option<f64>) = (None, None);
    if let Ok(sj) = af::cached_get(&state, "/fixtures/statistics", vec![("fixture", fid.to_string())], TTL_LIVE_SHORT).await {
        for t in response_array(&sj) {
            let tname = t.get("team").and_then(|x| x.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let is_home = tname == fixture.home_team;
            let mut items = Vec::new();
            for w in want {
                if let Some(v) = t.get("statistics").and_then(|a| a.as_array()).and_then(|arr| arr.iter().find(|s| s.get("type").and_then(|x| x.as_str()) == Some(w))).and_then(|s| s.get("value")) {
                    let val = v.as_str().map(|s| s.to_string()).or_else(|| v.as_f64().map(|f| format!("{}", f as i64))).unwrap_or_default();
                    if !val.is_empty() && val != "null" {
                        items.push(LiveStatKV { label: w.to_string(), value: val.clone() });
                        let num = val.trim_end_matches('%').parse::<f64>().ok();
                        if w == "Corner Kicks" {
                            corners_total += num.unwrap_or(0.0);
                        }
                        if w == "Shots on Goal" {
                            if is_home { home_sot = num } else { away_sot = num }
                        }
                    }
                }
            }
            if !items.is_empty() {
                stats.push(LiveTeamStat { team: tname, stats: items });
            }
        }
    }
    let has_stats = !stats.is_empty();

    // Events (goals + scorers, subs, cards).
    let mut events = Vec::new();
    if let Ok(ej) = af::cached_get(&state, "/fixtures/events", vec![("fixture", fid.to_string())], TTL_LIVE_SHORT).await {
        for e in response_array(&ej) {
            events.push(LiveEvent {
                minute: e.get("time").and_then(|t| t.get("elapsed")).and_then(|v| v.as_i64()).unwrap_or(0),
                team: e.get("team").and_then(|t| t.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string(),
                kind: e.get("type").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                player: e.get("player").and_then(|p| p.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string(),
                detail: e.get("detail").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            });
        }
    }

    // In-play odds for goal/2nd-half/corner markets (parse first so estimates can
    // be value-flagged against them).
    let mut odds = Vec::new();
    if let Ok(oj) = af::cached_get(&state, "/odds/live", vec![("fixture", fid.to_string())], TTL_LIVE_SHORT).await {
        if let Some(r) = response_array(&oj).into_iter().next() {
            if let Some(arr) = r.get("odds").and_then(|x| x.as_array()) {
                for o in arr {
                    let name = o.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let nl = name.to_lowercase();
                    if !(nl.contains("goal") || nl.contains("score") || nl.contains("corner") || nl.contains("over/under")) {
                        continue;
                    }
                    if let Some(vals) = o.get("values").and_then(|x| x.as_array()) {
                        for v in vals.iter().take(8) {
                            let raw = v.get("value").and_then(|x| x.as_str()).unwrap_or("");
                            // The line (e.g. corners "9.5") lives in `handicap`; fold it in.
                            let hcap = v.get("handicap").and_then(|x| x.as_str()).filter(|s| !s.is_empty() && *s != "null");
                            let sel = match hcap {
                                Some(h) if !raw.contains(h) => format!("{raw} {h}"),
                                _ => raw.to_string(),
                            };
                            let odd = v.get("odd").and_then(|x| x.as_str()).and_then(|s| s.parse::<f64>().ok()).or_else(|| v.get("odd").and_then(|x| x.as_f64())).unwrap_or(0.0);
                            if odd > 1.0 {
                                odds.push(LiveOdd { market: name.to_string(), selection: sel, odds: odd, implied: round4(1.0 / odd) });
                            }
                        }
                    }
                }
            }
        }
    }
    let cur_total = (hg + ag) as f64;

    // Remaining-time estimates: our pre-match rates scaled by time left, sharpened
    // by live shot momentum where stats exist.
    let mut estimates = Vec::new();
    let remaining = (90 - elapsed).clamp(0, 90) as f64;
    let frac = remaining / 90.0;
    let home = fetch_team_stats(&state, fixture.home_team_id, &fixture.home_team, fixture.league_id, fixture.season).await.ok().flatten();
    let away = fetch_team_stats(&state, fixture.away_team_id, &fixture.away_team, fixture.league_id, fixture.season).await.ok().flatten();
    if let (Some(h), Some(a)) = (&home, &away) {
        // Blend the model rate with a live shots-on-target pace (≈0.3 conversion).
        let mom = |pre: f64, sot: Option<f64>| -> f64 {
            match sot {
                Some(s) if elapsed > 12 => {
                    let pace = (s * 0.30) / elapsed as f64 * remaining;
                    0.5 * pre + 0.5 * pace
                }
                _ => pre,
            }
        };
        let used_mom = has_stats && (home_sot.is_some() || away_sot.is_some()) && elapsed > 12;
        let lh = mom(((h.gf_avg + a.ga_avg) / 2.0).max(0.05) * frac, home_sot);
        let la = mom(((a.gf_avg + h.ga_avg) / 2.0).max(0.05) * frac, away_sot);
        let basis = if used_mom { "model + live shot momentum" } else { "model (rate × time left)" };

        // Any further goal → the in-play "Over (current total + 0.5)" price.
        let p_any = round4(1.0 - (-(lh + la)).exp());
        let goal_sel = format!("over {:.1}", cur_total + 0.5);
        let (e, b) = edge_of(p_any, find_odd(&odds, &["goal"], &goal_sel));
        estimates.push(LiveEstimate { label: format!("Any goal in the remaining ~{}'", remaining as i64), prob: p_any, basis: basis.into(), edge: e, book: b });

        // Each team to score from here → "{team} ... score a goal" / over 0.5.
        let p_home = round4(1.0 - (-lh).exp());
        let home_odd = find_odd(&odds, &["home", "score"], "yes").or_else(|| find_odd(&odds, &["home", "goal"], "over 0.5"));
        let (eh, bh) = edge_of(p_home, home_odd);
        estimates.push(LiveEstimate { label: format!("{} to score from here", fixture.home_team), prob: p_home, basis: basis.into(), edge: eh, book: bh });

        let p_away = round4(1.0 - (-la).exp());
        let away_odd = find_odd(&odds, &["away", "score"], "yes").or_else(|| find_odd(&odds, &["away", "goal"], "over 0.5"));
        let (ea, ba) = edge_of(p_away, away_odd);
        estimates.push(LiveEstimate { label: format!("{} to score from here", fixture.away_team), prob: p_away, basis: basis.into(), edge: ea, book: ba });
    }
    // Corner pace (live-driven): extrapolate current rate over remaining time.
    if corners_total > 0.0 && elapsed > 5 {
        let add = corners_total * (remaining / elapsed as f64);
        let next = corners_total + 2.5;
        let p_corner = round4(pois_ge(3, add));
        let corner_sel = format!("over {:.1}", next);
        let (ec, bc) = edge_of(p_corner, find_odd(&odds, &["corner"], &corner_sel));
        estimates.push(LiveEstimate {
            label: format!("Over {:.1} total corners (now {})", next, corners_total as i64),
            prob: p_corner,
            basis: format!("pace: ~{:.1} more expected", add),
            edge: ec,
            book: bc,
        });
    }

    let note = if has_stats {
        "Live stats available. Estimates use our pre-match rates scaled to time remaining; corners use live pace.".to_string()
    } else {
        "No live in-match stats for this match (common outside top leagues) — goal estimates still apply from the score, minute and our rates.".to_string()
    };

    Ok(LiveSnapshot {
        fixture: LiveFixture { status, elapsed, home_goals: hg, away_goals: ag, has_stats, ..fixture },
        stats,
        events,
        estimates,
        odds,
        note,
    })
}

/// Build an IN-PLAY ticket: take the live snapshot (current stats + our
/// remaining-time estimates + the live odds), fold in any ingested page notes
/// for these teams, and make ONE model call to assemble a coherent ticket from
/// that menu. The model only SELECTS and EXPLAINS — every probability is ours
/// (our estimate) or the book's (de-vigged implied); it never invents a number.
/// Cached by a hash of the live state + menu + model, so a refresh is free.
#[tauri::command]
pub async fn live_ticket(state: State<'_, AppState>, fixture: LiveFixture, model: String) -> Result<LiveTicket, String> {
    if !llm::is_allowed_analysis_model(&model) {
        return Err(format!("model {model} is not allowed for analysis"));
    }
    let snap = live_snapshot_inner(&state, fixture.clone()).await?;
    let f = snap.fixture.clone();

    // Numbered menu the model must pick from (index-aligned to `menu`).
    let mut menu: Vec<LiveLeg> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    for e in &snap.estimates {
        let odds = e.book.as_ref().and_then(|b| b.rsplit('@').next()).and_then(|s| s.trim().parse::<f64>().ok());
        lines.push(format!(
            "[{}] {} — our {}%{}",
            menu.len(), e.label, (e.prob * 100.0).round() as i64,
            e.book.as_ref().map(|b| format!("  | book: {b}")).unwrap_or_default()
        ));
        menu.push(LiveLeg { label: e.label.clone(), prob: e.prob, odds, source: "model".into(), why: String::new() });
    }
    for o in snap.odds.iter().take(16) {
        lines.push(format!(
            "[{}] {} — {} @ {:.2} ({}% implied)",
            menu.len(), o.market, o.selection, o.odds, (o.implied * 100.0).round() as i64
        ));
        menu.push(LiveLeg { label: format!("{} — {}", o.market, o.selection), prob: o.implied, odds: Some(o.odds), source: "book".into(), why: String::new() });
    }

    // Live PLAYER props — it's a numbers game: who's actually shooting/attempting
    // right now is most likely to do more. Pace-extrapolate each player's live
    // shots over the time remaining.
    let elapsed = snap.fixture.elapsed.max(1);
    let frac = ((90 - snap.fixture.elapsed).clamp(0, 90) as f64) / elapsed as f64;
    if let Ok(pj) = af::cached_get(&state, "/fixtures/players", vec![("fixture", fixture.fixture_id.to_string())], TTL_LIVE_SHORT).await {
        let mut players: Vec<(String, String, f64, f64, f64)> = Vec::new(); // name, team, mins, shots, sot
        for team in response_array(&pj) {
            let tname = team.get("team").and_then(|t| t.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string();
            if let Some(arr) = team.get("players").and_then(|x| x.as_array()) {
                for p in arr {
                    let name = p.get("player").and_then(|x| x.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let st = p.get("statistics").and_then(|a| a.as_array()).and_then(|a| a.first());
                    let mins = st.and_then(|s| s.get("games")).and_then(|g| g.get("minutes")).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let shots = st.and_then(|s| s.get("shots")).and_then(|g| g.get("total")).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let sot = st.and_then(|s| s.get("shots")).and_then(|g| g.get("on")).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    if mins >= 1.0 && !name.is_empty() {
                        players.push((name, tname.clone(), mins, shots, sot));
                    }
                }
            }
        }
        // Prioritise the live shooters, then high-minute regulars.
        players.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal).then(b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal)));
        for (name, _team, _mins, shots, sot) in players.into_iter().take(8) {
            // Shots: P(≥1 more) over remaining time, pace from shots-so-far.
            let more_shots = shots * frac;
            let p_shot = 1.0 - (-more_shots).exp();
            if p_shot > 0.15 {
                let next = (shots as i64) + 1;
                lines.push(format!("[{}] {} — {}+ shots (has {} so far) — our {}%", menu.len(), name, next, shots as i64, (p_shot * 100.0).round() as i64));
                menu.push(LiveLeg { label: format!("{} {}+ shots", name, next), prob: round4(p_shot), odds: None, source: "model".into(), why: String::new() });
            }
            // To score from here, from SOT pace × ~0.3 conversion.
            let p_score = 1.0 - (-(sot * frac * 0.30)).exp();
            if p_score > 0.08 {
                lines.push(format!("[{}] {} — to score from here ({} SOT so far) — our {}%", menu.len(), name, sot as i64, (p_score * 100.0).round() as i64));
                menu.push(LiveLeg { label: format!("{} to score from here", name), prob: round4(p_score), odds: None, source: "model".into(), why: String::new() });
            }
        }
    }

    if menu.is_empty() {
        return Err("no live markets or estimates to build from yet".into());
    }

    let stat_str = snap.stats.iter().map(|t| {
        format!("{}: {}", t.team, t.stats.iter().map(|s| format!("{} {}", s.label, s.value)).collect::<Vec<_>>().join(", "))
    }).collect::<Vec<_>>().join(" | ");
    let ev_str = snap.events.iter().rev().take(10).map(|e| format!("{}' {} {} ({})", e.minute, e.kind, e.player, e.team)).collect::<Vec<_>>().join("; ");

    let ingest_notes = {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        db::ingest_for_fixture(&conn).ok().map(|rows| {
            rows.into_iter().filter(|r| {
                let t = format!("{} {} {}", r.title, r.note, r.content).to_lowercase();
                t.contains(&f.home_team.to_lowercase()) || t.contains(&f.away_team.to_lowercase())
            }).take(4).map(|r| {
                let body = r.note.trim();
                let body = if body.is_empty() { r.content.chars().take(400).collect::<String>() } else { body.to_string() };
                format!("- {}: {}", r.title, body)
            }).collect::<Vec<_>>().join("\n")
        }).unwrap_or_default()
    };

    let menu_block = lines.join("\n");
    let ingest_block = if ingest_notes.is_empty() { "(none)".to_string() } else { ingest_notes };
    let system = "You are a sharp in-play football trader. From the supplied MENU you surface the BEST INDIVIDUAL bets (SINGLES) for the current game state — a POOL the user cherry-picks from, NOT one combined ticket. RULES: pick only indices that exist; never invent or alter a probability (the menu % is fixed and authoritative); the picks may point different ways or even mildly conflict — that's fine, they are separate singles, not a parlay; avoid near-duplicates (don't pick both '2+ shots' and 'to score' for the same player unless both are strong). Lean on who is ACTUALLY attempting/shooting (it's a numbers game) and the live momentum. Output strict JSON only, no prose.";
    let user = format!(
        "LIVE MATCH: {} {}-{} {} ({}', {} | {})\n\nLIVE STATS: {}\nEVENTS: {}\n\nINGESTED PAGE NOTES (your own clipped pages — soft context, don't trust blindly):\n{}\n\nMENU (pick by index — the ONLY picks you may use; the % is fixed, never change it):\n{}\n\nSurface the 5-8 BEST SINGLES for THIS game state right now — a pool to build from. Favour live shooters likely to do more, value vs the live odds, and any quiet threat the numbers say is due. Each is a STANDALONE single; do NOT try to make them combine. Output STRICT JSON only: {{\"legs\":[{{\"i\":<index>,\"why\":\"<6-12 words>\"}}],\"rationale\":\"<1 sentence on how the game is unfolding>\",\"confidence\":\"low|medium|high\"}}",
        f.home_team, f.home_goals, f.away_goals, f.away_team, f.elapsed, f.status, f.league_name,
        if stat_str.is_empty() { "(none)" } else { &stat_str },
        if ev_str.is_empty() { "(none)" } else { &ev_str },
        ingest_block, menu_block
    );

    // Cache by live state + menu + model (token-budget rule: one call, cached).
    let mut h = Sha256::new();
    h.update(f.fixture_id.to_le_bytes());
    h.update(f.elapsed.to_le_bytes());
    h.update(f.home_goals.to_le_bytes());
    h.update(f.away_goals.to_le_bytes());
    h.update(menu_block.as_bytes());
    h.update(ingest_block.as_bytes());
    h.update(model.as_bytes());
    let key = format!("live-ticket:{:x}", h.finalize());

    let mut cached = true;
    let raw = {
        let hit = { let conn = state.db.lock().map_err(|_| "db lock")?; db::ai_get(&conn, &key).ok().flatten() };
        match hit {
            Some(v) => v,
            None => {
                cached = false;
                let (txt, gin, gout) = llm::chat_call(&state, &model, system, &user, 600).await?;
                let conn = state.db.lock().map_err(|_| "db lock")?;
                let _ = db::usage_add(&conn, af::now_ts(), &model, gin, gout, "live");
                let _ = db::ai_put(&conn, &key, &txt, &model, af::now_ts());
                txt
            }
        }
    };

    // Parse the model's JSON selection defensively; reconstruct legs from the menu.
    let json_str = raw.find('{').and_then(|s| raw.rfind('}').map(|e| &raw[s..=e])).unwrap_or("{}");
    let parsed: serde_json::Value = serde_json::from_str(json_str).unwrap_or_else(|_| serde_json::json!({}));
    let mut legs: Vec<LiveLeg> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Some(arr) = parsed.get("legs").and_then(|x| x.as_array()) {
        for item in arr {
            let i = item.get("i").and_then(|x| x.as_i64()).unwrap_or(-1);
            if i < 0 || i as usize >= menu.len() || !seen.insert(i) {
                continue;
            }
            let mut leg = menu[i as usize].clone();
            leg.why = item.get("why").and_then(|x| x.as_str()).unwrap_or("").to_string();
            legs.push(leg);
        }
    }
    if legs.is_empty() {
        return Err("model returned no usable legs — try refresh".into());
    }

    let combined_prob = round4(legs.iter().map(|l| l.prob).product::<f64>());
    let combined_odds = if legs.iter().all(|l| l.odds.is_some()) {
        Some(round4(legs.iter().map(|l| l.odds.unwrap()).product::<f64>()))
    } else {
        None
    };
    let rationale = parsed.get("rationale").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let confidence = parsed.get("confidence").and_then(|x| x.as_str()).unwrap_or("medium").to_string();

    Ok(LiveTicket {
        fixture: snap.fixture,
        legs,
        combined_prob,
        combined_odds,
        rationale,
        confidence,
        model,
        cached,
        note: "Combined % assumes leg independence — correlated same-game legs skew it; treat as a guide. Every probability is ours (estimate) or de-vigged book implied, never model-invented.".into(),
    })
}

/// Correlation-aware price for a same-game (or mixed) parlay. Returns the joint
/// probability WITH leg correlation by Monte Carlo, next to the naive
/// independent product, so the UI can show how much the legs help each other.
/// Marginals are our model probabilities (or sharp where that's all a leg has) —
/// the sim only models co-occurrence, it never changes a leg's probability.
#[tauri::command]
pub fn price_sgp(legs: Vec<TicketLeg>) -> Result<crate::montecarlo::SgpPrice, String> {
    let sim_legs: Vec<crate::montecarlo::SimLeg> = legs
        .iter()
        .map(|l| {
            let line = l.line.clone().unwrap_or_default();
            let prob = l.est_prob.or(l.pinnacle_prob).unwrap_or(0.5).clamp(0.001, 0.999);
            crate::montecarlo::SimLeg {
                fixture_id: l.fixture_id,
                subject: crate::odds::fold(&l.selection),
                theme: crate::montecarlo::theme_of(&l.market, &line, &l.selection),
                prob,
                scoreline: crate::montecarlo::is_scoreline_market(&l.market),
            }
        })
        .collect();
    Ok(crate::montecarlo::sgp_probability(&sim_legs, 20_000))
}

// ---------- browser-extension ingest ----------

/// Short one-line digest of an ingested page's extracted JSON for the prompt.
fn compact_ingest(v: &Value) -> String {
    let summary = v.get("summary").and_then(|s| s.as_str()).unwrap_or("");
    let data: Vec<String> = v
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| {
                    let l = e.get("label").and_then(|x| x.as_str())?;
                    let val = e.get("value").and_then(|x| x.as_str())?;
                    Some(format!("{l}={val}"))
                })
                .take(8)
                .collect()
        })
        .unwrap_or_default();
    let mut out = summary.to_string();
    if !data.is_empty() {
        if !out.is_empty() {
            out.push_str(" | ");
        }
        out.push_str(&data.join("; "));
    }
    out.chars().take(400).collect()
}

fn to_ingest_item(r: &db::IngestRow) -> IngestItem {
    let v = r
        .extracted_json
        .as_deref()
        .and_then(|j| serde_json::from_str::<Value>(j).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let summary = v.get("summary").and_then(|s| s.as_str()).unwrap_or_default().to_string();
    let fixture_date = v
        .get("date")
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let data: Vec<IngestKV> = v
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| {
                    Some(IngestKV {
                        label: e.get("label").and_then(|x| x.as_str())?.to_string(),
                        value: e.get("value").and_then(|x| x.as_str())?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    IngestItem {
        id: r.id,
        created_at: r.created_at,
        url: r.url.clone(),
        title: r.title.clone(),
        note: r.note.clone(),
        status: r.status.clone(),
        fixture_label: r.fixture_label.clone(),
        fixture_date,
        summary,
        data,
        model: r.model.clone(),
        used: r.used,
    }
}

/// Local ingest endpoint info — shown in Settings so you can paste the URL + token
/// into the browser extension.
#[tauri::command]
pub fn ingest_info(state: State<AppState>) -> Result<IngestInfo, String> {
    let (enabled, port, token) = {
        let keys = state.keys.lock().map_err(|_| "keys lock")?;
        (
            keys.ingest_enabled.unwrap_or(true),
            keys.ingest_port.unwrap_or(8765),
            keys.ingest_token.clone().unwrap_or_default(),
        )
    };
    let new_count = {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        db::ingest_list(&conn)?.iter().filter(|r| r.status == "new").count() as i64
    };
    Ok(IngestInfo { enabled, port, token, new_count })
}

#[tauri::command]
pub fn list_ingested(state: State<AppState>) -> Result<Vec<IngestItem>, String> {
    let conn = state.db.lock().map_err(|_| "db lock")?;
    Ok(db::ingest_list(&conn)?.iter().map(to_ingest_item).collect())
}

#[tauri::command]
pub fn delete_ingested(state: State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(|_| "db lock")?;
    db::ingest_delete(&conn, id)
}

#[tauri::command]
pub fn update_ingest_note(state: State<AppState>, id: i64, note: String) -> Result<(), String> {
    let conn = state.db.lock().map_err(|_| "db lock")?;
    db::ingest_set_note(&conn, id, &note)
}

/// Run Haiku over an ingested page: structure it + tag the fixture it's about.
#[tauri::command]
pub async fn process_ingested(
    state: State<'_, AppState>,
    id: i64,
    model: Option<String>,
) -> Result<IngestItem, String> {
    let row = {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        db::ingest_get(&conn, id)?.ok_or_else(|| "not found".to_string())?
    };
    let model = model
        .filter(|m| llm::is_allowed_analysis_model(m))
        .unwrap_or_else(|| "claude-haiku-4-5".to_string());
    let model = model.as_str();
    let (json, gin, gout) = llm::extract_ingest(&state, model, &row.content, &row.note).await?;
    let v: Value = serde_json::from_str(&json).unwrap_or_else(|_| serde_json::json!({}));
    let home = v.get("home").and_then(|x| x.as_str()).unwrap_or("").trim();
    let away = v.get("away").and_then(|x| x.as_str()).unwrap_or("").trim();
    let label = if !home.is_empty() && !away.is_empty() { format!("{home} vs {away}") } else { String::new() };
    {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        let _ = db::usage_add(&conn, af::now_ts(), model, gin, gout, "ingest");
        db::ingest_set_processed(&conn, id, &label, None, &json, model)?;
    }
    let conn = state.db.lock().map_err(|_| "db lock")?;
    let updated = db::ingest_get(&conn, id)?.ok_or_else(|| "not found".to_string())?;
    Ok(to_ingest_item(&updated))
}

#[tauri::command]
pub fn delete_bet(state: State<AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(|_| "db lock")?;
    db::delete_placed(&conn, id)
}

/// Grade an open bet against results. Status: won / lost / partial / open.
#[tauri::command]
pub async fn settle_bet(state: State<'_, AppState>, id: i64) -> Result<PlacedBet, String> {
    let row = {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        db::get_placed(&conn, id)?.ok_or_else(|| "bet not found".to_string())?
    };
    let ticket: Ticket = serde_json::from_str(&row.ticket_json).map_err(|e| e.to_string())?;

    let leg_results = settle::grade_legs(&state, &ticket.legs).await;
    let graded = leg_results.iter().filter(|r| r.won.is_some()).count();
    let total = leg_results.len();
    let any_lost = leg_results.iter().any(|r| r.won == Some(false));
    let any_won = leg_results.iter().any(|r| r.won == Some(true));

    let (status, settled, returns) = if total == 0 {
        ("open".to_string(), false, 0.0)
    } else if graded < total {
        ("open".to_string(), false, 0.0) // not all matches finished yet
    } else if any_lost {
        let s = if any_won { "partial" } else { "lost" };
        (s.to_string(), true, 0.0)
    } else {
        // all legs won
        let payout = match ticket.combined_odds {
            Some(o) if o > 0.0 => row.stake * o,
            _ => row.stake, // unknown odds → break-even placeholder
        };
        ("won".to_string(), true, (payout * 100.0).round() / 100.0)
    };

    let lr_json = serde_json::to_string(&leg_results).map_err(|e| e.to_string())?;
    {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        db::update_settlement(&conn, id, &status, returns, &lr_json, settled)?;
        let r = db::get_placed(&conn, id)?.ok_or_else(|| "bet not found".to_string())?;
        row_to_bet(&r)
    }
}

/// Settle every open bet; returns the full updated list.
#[tauri::command]
pub async fn settle_all(state: State<'_, AppState>) -> Result<Vec<PlacedBet>, String> {
    let open_ids: Vec<i64> = {
        let conn = state.db.lock().map_err(|_| "db lock")?;
        db::list_placed(&conn)?
            .into_iter()
            .filter(|r| !r.settled)
            .map(|r| r.id)
            .collect()
    };
    for id in open_ids {
        let _ = settle_bet(state.clone(), id).await;
    }
    list_bets(state)
}

// ---------- data inspector (cache-only — never hits the network) ----------

/// Read a cached API body without touching the network or the request meter.
fn peek(state: &AppState, endpoint: &str, params: Vec<(&str, String)>) -> Option<Value> {
    let key = af::cache_key(endpoint, &params);
    let now = af::now_ts();
    let conn = state.db.lock().ok()?;
    let payload = db::cache_get(&conn, &key, now).ok()??;
    serde_json::from_str(&payload).ok()
}

fn squad_lite(json: &Value, injuries: &HashMap<i64, String>) -> Vec<PlayerLite> {
    let mut out = Vec::new();
    if let Some(entry) = response_array(json).first() {
        if let Some(arr) = entry.get("players").and_then(|p| p.as_array()) {
            for p in arr {
                if let Some(pid) = p.get("id").and_then(|v| v.as_i64()) {
                    out.push(PlayerLite {
                        player_id: pid,
                        name: p.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown").to_string(),
                        position: p.get("position").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        availability: injuries.get(&pid).cloned().unwrap_or_else(|| "unknown".to_string()),
                    });
                }
            }
        }
    }
    out
}

/// Build the team/squad inspector tree for the selected fixtures. Squads and
/// injuries are fetched cache-first (normally already cached from the Players
/// step → 0 requests); team stats stay cache-only and load on demand below.
#[tauri::command]
pub async fn inspect_fixtures(
    state: State<'_, AppState>,
    fixtures: Vec<FixtureInput>,
) -> Result<Vec<InspectFixture>, String> {
    let mut out = Vec::new();
    for fx in &fixtures {
        let injuries = fetch_injury_map(&state, fx.fixture_id).await.unwrap_or_default();

        let mut teams = Vec::new();
        for (team_id, name) in [
            (fx.home_team_id, fx.home_team.clone()),
            (fx.away_team_id, fx.away_team.clone()),
        ] {
            let squad = af::cached_get(
                &state,
                "/players/squads",
                vec![("team", team_id.to_string())],
                af::TTL_SQUADS,
            )
            .await
            .ok();
            let loaded = squad.is_some();
            let players = squad.as_ref().map(|j| squad_lite(j, &injuries)).unwrap_or_default();

            let stats = peek(
                &state,
                "/teams/statistics",
                vec![
                    ("team", team_id.to_string()),
                    ("league", fx.league_id.to_string()),
                    ("season", fx.season.to_string()),
                ],
            )
            .and_then(|j| features::parse_team_stats(&j, &name))
            .map(|t| features::team_stats_view(&t));

            teams.push(InspectTeam {
                team_id,
                team_name: name,
                loaded,
                stats,
                players,
            });
        }

        out.push(InspectFixture {
            fixture_id: fx.fixture_id,
            league_id: fx.league_id,
            season: fx.season,
            fixture_label: format!("{} vs {}", fx.home_team, fx.away_team),
            teams,
        });
    }
    Ok(out)
}

/// A single player's season stats + per-90 rates (the engine's inputs).
/// Cache-first fetch, so tapping any player loads their data (then caches it).
#[tauri::command]
pub async fn inspect_player(
    state: State<'_, AppState>,
    player_id: i64,
    league_id: i64,
    season: i64,
) -> Result<Option<PlayerInspect>, String> {
    let json = af::cached_get(
        &state,
        "/players",
        vec![
            ("id", player_id.to_string()),
            ("league", league_id.to_string()),
            ("season", season.to_string()),
        ],
        af::TTL_PLAYERS,
    )
    .await?;
    Ok(features::parse_player_inspect(&json, league_id))
}

/// Load one team's season stats on demand (cache-first).
#[tauri::command]
pub async fn inspect_team_stats(
    state: State<'_, AppState>,
    team_id: i64,
    league_id: i64,
    season: i64,
) -> Result<Option<TeamStatsView>, String> {
    let json = af::cached_get(
        &state,
        "/teams/statistics",
        vec![
            ("team", team_id.to_string()),
            ("league", league_id.to_string()),
            ("season", season.to_string()),
        ],
        af::TTL_TEAMS,
    )
    .await?;
    Ok(features::parse_team_stats(&json, "").map(|t| features::team_stats_view(&t)))
}
