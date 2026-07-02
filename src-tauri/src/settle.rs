//! Bet settlement: fetch finished-match results + player stats and grade each
//! leg of a ticket. Highlights what hit even when the whole ticket misses.

use std::collections::HashMap;

use serde_json::Value;

use crate::apifootball::{self as af, response_array};
use crate::models::{LegResult, TicketLeg};
use crate::AppState;

pub const TTL_RESULT: i64 = 7 * 24 * 3600;

#[derive(Default)]
struct PlayerLine {
    goals: f64,
    assists: f64,
    sot: f64,
    shots: f64,
    tackles: f64,
    fouls_committed: f64,
    fouls_drawn: f64,
    cards: f64,
    passes: f64,
    saves: f64,
}

struct FixtureResult {
    finished: bool,
    /// Match will never finish (postponed/cancelled/abandoned/awarded/walkover)
    /// → every leg is void, and we must stop re-fetching it.
    terminal_void: bool,
    /// Match went to extra time (AET/PEN). We grade on the 90-minute score, but
    /// player stats from the API cover the whole match — flag that honestly.
    extra_time: bool,
    first_scorer_team: Option<String>,
    home_name: String,
    away_name: String,
    home_goals: f64,
    away_goals: f64,
    ht_home: Option<f64>,
    ht_away: Option<f64>,
    home_corners: Option<f64>,
    away_corners: Option<f64>,
    home_shots: Option<f64>,
    away_shots: Option<f64>,
    home_outbox: Option<f64>,
    away_outbox: Option<f64>,
    home_inbox: Option<f64>,
    away_inbox: Option<f64>,
    home_offsides: Option<f64>,
    away_offsides: Option<f64>,
    home_cards: f64,
    away_cards: f64,
    players: HashMap<String, PlayerLine>,
}

fn num(v: &Value) -> f64 {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
        .unwrap_or(0.0)
}
fn opt_num(v: Option<&Value>) -> Option<f64> {
    v.and_then(|x| x.as_f64().or_else(|| x.as_i64().map(|i| i as f64)))
}

/// Short re-check window for a fixture that isn't finished yet, so a non-final
/// status can never poison the long result cache.
const TTL_RECHECK: i64 = 300;

fn parse_status(j: &Value) -> String {
    response_array(j)
        .first()
        .and_then(|e| e.get("fixture"))
        .and_then(|f| f.get("status"))
        .and_then(|s| s.get("short"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn parse_finished(j: &Value) -> bool {
    matches!(parse_status(j).as_str(), "FT" | "AET" | "PEN")
}

/// Statuses that mean the match will NEVER reach full time — the bet is void at
/// the book and re-fetching forever would just burn budget.
fn is_terminal_void(status: &str) -> bool {
    matches!(status, "PST" | "CANC" | "ABD" | "AWD" | "WO")
}

/// Is the match expected to have ENDED (kickoff + ~2.5h passed)? Used to avoid
/// force-refreshing fixtures that are still upcoming or in play.
fn expected_ended(j: &Value) -> bool {
    response_array(j)
        .first()
        .and_then(|e| e.get("fixture"))
        .and_then(|f| f.get("date"))
        .and_then(|v| v.as_str())
        .and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok())
        // ~100 min after kickoff a match is normally at/near full time. Force a
        // fresh result check from here (a still-live game just returns not-finished
        // and retries later) — 150 min was too late and left ended games stuck.
        .map(|dt| af::now_ts() >= dt.timestamp() + 100 * 60)
        .unwrap_or(true) // no date → don't block
}

async fn fetch_result(state: &AppState, fixture_id: i64) -> Option<FixtureResult> {
    // Cache-first, but if the cached row isn't finished (it may have been cached
    // while the match was still live/not-started), force a fresh pull so we see
    // the final result instead of a stale "not finished".
    let mut fj = af::cached_get(
        state,
        "/fixtures",
        vec![("id", fixture_id.to_string())],
        TTL_RESULT,
    )
    .await
    .ok()?;
    let mut forced = false;
    if !parse_finished(&fj) && !is_terminal_void(&parse_status(&fj)) && expected_ended(&fj) {
        // Priority: reading a finished result must not be blocked by the daily
        // build budget (that's what leaves ended matches stuck "pending").
        if let Ok(fresh) =
            af::fetch_live(state, "/fixtures", vec![("id", fixture_id.to_string())], TTL_RECHECK).await
        {
            fj = fresh;
            forced = true;
        }
    }
    let entry = response_array(&fj);
    let e = entry.first()?;

    let status = e
        .get("fixture")
        .and_then(|f| f.get("status"))
        .and_then(|s| s.get("short"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let finished = matches!(status, "FT" | "AET" | "PEN");
    let extra_time = matches!(status, "AET" | "PEN");

    // Postponed/cancelled/abandoned: return a void marker immediately — no
    // player/stats/events fetches (nothing to grade, and it would loop forever).
    if is_terminal_void(status) {
        let teams = e.get("teams");
        let name = |side: &str| {
            teams
                .and_then(|t| t.get(side))
                .and_then(|h| h.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        return Some(FixtureResult {
            finished: false,
            terminal_void: true,
            extra_time: false,
            first_scorer_team: None,
            home_name: name("home"),
            away_name: name("away"),
            home_goals: 0.0,
            away_goals: 0.0,
            ht_home: None,
            ht_away: None,
            home_corners: None,
            away_corners: None,
            home_shots: None,
            away_shots: None,
            home_outbox: None,
            away_outbox: None,
            home_inbox: None,
            away_inbox: None,
            home_offsides: None,
            away_offsides: None,
            home_cards: 0.0,
            away_cards: 0.0,
            players: HashMap::new(),
        });
    }

    // 90-MINUTE score: books settle standard markets on full time, but `goals`
    // is the final aggregate INCLUDING extra time for AET/PEN. Prefer
    // score.fulltime; fall back to goals for leagues that omit it.
    let ft = e.get("score").and_then(|s| s.get("fulltime"));
    let goals = e.get("goals");
    let goal_of = |side: &str| {
        opt_num(ft.and_then(|f| f.get(side)))
            .or_else(|| opt_num(goals.and_then(|g| g.get(side))))
            .unwrap_or(0.0)
    };
    let home_goals = goal_of("home");
    let away_goals = goal_of("away");
    let ht = e.get("score").and_then(|s| s.get("halftime"));
    let ht_home = opt_num(ht.and_then(|h| h.get("home")));
    let ht_away = opt_num(ht.and_then(|h| h.get("away")));

    let teams = e.get("teams");
    let home_name = teams
        .and_then(|t| t.get("home"))
        .and_then(|h| h.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let away_name = teams
        .and_then(|t| t.get("away"))
        .and_then(|a| a.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Player stats for the fixture — pull fresh too if we just forced the result
    // (a match that only finished after an earlier settle would have stale stats).
    // If we just forced the result and the match IS finished, the fresh player
    // stats are final — cache them for the full week, not the 300s recheck.
    let forced_ttl = if finished { TTL_RESULT } else { TTL_RECHECK };
    let players_params = vec![("fixture", fixture_id.to_string())];
    let pj_res = if forced {
        af::fetch_live(state, "/fixtures/players", players_params, forced_ttl).await
    } else {
        af::cached_get(state, "/fixtures/players", players_params, TTL_RESULT).await
    };
    let mut players = HashMap::new();
    let mut cards_by_team: HashMap<i64, f64> = HashMap::new();
    if let Ok(pj) = pj_res {
        for team in response_array(&pj) {
            let tid = team.get("team").and_then(|t| t.get("id")).and_then(|v| v.as_i64()).unwrap_or(-1);
            if let Some(arr) = team.get("players").and_then(|p| p.as_array()) {
                for p in arr {
                    let name = crate::odds::fold(
                        p.get("player").and_then(|x| x.get("name")).and_then(|v| v.as_str()).unwrap_or(""),
                    );
                    let s = p
                        .get("statistics")
                        .and_then(|x| x.as_array())
                        .and_then(|a| a.first());
                    if name.is_empty() {
                        continue;
                    }
                    let g = |a: &str, b: &str| {
                        s.and_then(|st| st.get(a)).and_then(|x| x.get(b)).map(num).unwrap_or(0.0)
                    };
                    let cards = g("cards", "yellow") + g("cards", "red");
                    *cards_by_team.entry(tid).or_insert(0.0) += cards;
                    players.insert(
                        name,
                        PlayerLine {
                            goals: g("goals", "total"),
                            assists: g("goals", "assists"),
                            sot: g("shots", "on"),
                            shots: g("shots", "total"),
                            tackles: g("tackles", "total"),
                            fouls_committed: g("fouls", "committed"),
                            fouls_drawn: g("fouls", "drawn"),
                            cards,
                            passes: g("passes", "total"),
                            saves: g("goals", "saves"),
                        },
                    );
                }
            }
        }
    }

    // Team corners / shots from fixture statistics (covered leagues).
    let home_id = teams.and_then(|t| t.get("home")).and_then(|h| h.get("id")).and_then(|v| v.as_i64());
    let away_id = teams.and_then(|t| t.get("away")).and_then(|h| h.get("id")).and_then(|v| v.as_i64());
    let home_cards = home_id.and_then(|id| cards_by_team.get(&id).copied()).unwrap_or(0.0);
    let away_cards = away_id.and_then(|id| cards_by_team.get(&id).copied()).unwrap_or(0.0);
    let (mut home_corners, mut away_corners, mut home_shots, mut away_shots) = (None, None, None, None);
    let (mut home_outbox, mut away_outbox, mut home_inbox, mut away_inbox) = (None, None, None, None);
    let (mut home_offsides, mut away_offsides) = (None, None);
    let stats_params = vec![("fixture", fixture_id.to_string())];
    let sj_res = if forced {
        af::fetch_live(state, "/fixtures/statistics", stats_params, forced_ttl).await
    } else {
        af::cached_get(state, "/fixtures/statistics", stats_params, TTL_RESULT).await
    };
    if let Ok(sj) = sj_res {
        let stat = |team: &Value, ty: &str| -> Option<f64> {
            team.get("statistics")
                .and_then(|s| s.as_array())
                .and_then(|arr| arr.iter().find(|s| s.get("type").and_then(|t| t.as_str()) == Some(ty)))
                .and_then(|s| s.get("value"))
                .and_then(|v| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()).or_else(|| v.as_f64()))
        };
        for team in response_array(&sj) {
            let is_home = team.get("team").and_then(|t| t.get("id")).and_then(|v| v.as_i64()) == home_id;
            if is_home {
                home_corners = stat(&team, "Corner Kicks");
                home_shots = stat(&team, "Total Shots");
                home_outbox = stat(&team, "Shots outsidebox");
                home_inbox = stat(&team, "Shots insidebox");
                home_offsides = stat(&team, "Offsides");
            } else {
                away_corners = stat(&team, "Corner Kicks");
                away_shots = stat(&team, "Total Shots");
                away_outbox = stat(&team, "Shots outsidebox");
                away_inbox = stat(&team, "Shots insidebox");
                away_offsides = stat(&team, "Offsides");
            }
        }
    }

    // First team to score — the team of the earliest real Goal event (skip
    // own-goal? no: an own goal still counts as a goal for the team it credits;
    // skip missed/cancelled penalties). None if 0-0 or events unavailable.
    let mut first_scorer_team: Option<String> = None;
    if home_goals + away_goals > 0.0 {
        // Finished-match events are immutable: cache-first (a week), but still
        // budget-exempt so a maxed build budget can't stall settlement. This used
        // to be fetch_live (always-network) — every settle pass burned a fresh
        // request per fixture with goals.
        if let Ok(ej) = af::fetch_priority(state, "/fixtures/events", vec![("fixture", fixture_id.to_string())], TTL_RESULT).await {
            let mut best: Option<(i64, String)> = None;
            for ev in response_array(&ej) {
                let kind = ev.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let detail = ev.get("detail").and_then(|v| v.as_str()).unwrap_or("");
                if kind != "Goal" || detail.contains("Missed") || detail.contains("Cancelled") {
                    continue;
                }
                let minute = ev.get("time").and_then(|t| t.get("elapsed")).and_then(|v| v.as_i64()).unwrap_or(999);
                let team = ev.get("team").and_then(|t| t.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                if best.as_ref().map_or(true, |(m, _)| minute < *m) {
                    best = Some((minute, team));
                }
            }
            first_scorer_team = best.map(|(_, t)| t);
        }
    }

    Some(FixtureResult {
        finished,
        terminal_void: false,
        extra_time,
        first_scorer_team,
        home_name,
        away_name,
        home_goals,
        away_goals,
        ht_home,
        ht_away,
        home_corners,
        away_corners,
        home_shots,
        away_shots,
        home_outbox,
        away_outbox,
        home_inbox,
        away_inbox,
        home_offsides,
        away_offsides,
        home_cards,
        away_cards,
        players,
    })
}

/// Extract the leading integer threshold from a line like "2+ tackles".
fn threshold(line: &str) -> f64 {
    line.split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(1.0)
}

fn won(b: bool, detail: String) -> LegResult {
    LegResult { won: Some(b), detail, margin: None, void: false }
}
/// O/U-style result: `over` = the bet direction, `actual`/`thr` the value & line.
/// Records the signed gap to the line (positive = cleared it, negative = missed).
fn won_ou(over: bool, actual: f64, thr: f64, detail: String) -> LegResult {
    let hit = if over { actual > thr } else { actual < thr };
    let margin = if over { actual - thr } else { thr - actual };
    LegResult { won: Some(hit), detail, margin: Some((margin * 100.0).round() / 100.0), void: false }
}
fn ungraded(detail: &str) -> LegResult {
    LegResult { won: None, detail: detail.to_string(), margin: None, void: false }
}
/// Book-refunded leg: doesn't count toward win/lose; an all-void ticket pushes.
fn voided(detail: &str) -> LegResult {
    LegResult { won: None, detail: detail.to_string(), margin: None, void: true }
}

/// UNAMBIGUOUS player lookup. Exact folded-name match first; then a containment
/// match, then a last-name match — but a fuzzy match only counts if EXACTLY ONE
/// player fits. Two Silvas in a match must never grade each other's props (the
/// old first-match-wins over a HashMap picked one at random).
enum Lookup<'a> {
    Found(&'a PlayerLine),
    /// Multiple players fit the name — refuse to guess (leave ungraded).
    Ambiguous,
    /// Nobody fits — the player did not feature (void at the book).
    Absent,
}
fn lookup_player<'a>(players: &'a HashMap<String, PlayerLine>, name: &str) -> Lookup<'a> {
    let n = crate::odds::fold(name);
    if let Some(p) = players.get(&n) {
        return Lookup::Found(p);
    }
    let contains: Vec<&PlayerLine> = players
        .iter()
        .filter(|(k, _)| k.contains(&n) || n.contains(k.as_str()))
        .map(|(_, v)| v)
        .collect();
    if contains.len() == 1 {
        return Lookup::Found(contains[0]);
    }
    if !contains.is_empty() {
        return Lookup::Ambiguous;
    }
    let last = n.rsplit(' ').next().unwrap_or(&n);
    if last.len() >= 4 {
        let by_last: Vec<&PlayerLine> = players
            .iter()
            .filter(|(k, _)| k.rsplit(' ').next() == Some(last))
            .map(|(_, v)| v)
            .collect();
        match by_last.len() {
            1 => return Lookup::Found(by_last[0]),
            0 => {}
            _ => return Lookup::Ambiguous,
        }
    }
    Lookup::Absent
}

fn grade_leg(leg: &TicketLeg, r: &FixtureResult) -> LegResult {
    if r.terminal_void {
        return voided("match postponed/cancelled — void");
    }
    if !r.finished {
        return ungraded("not finished");
    }
    let line = leg.line.clone().unwrap_or_default();
    let total = r.home_goals + r.away_goals;
    let market = leg.market.as_str();

    // Goals in a single half (uses the half-time score).
    if (market.contains("1st Half") || market.contains("2nd Half")) && market.ends_with("Goals") {
        let (hh, ha) = match (r.ht_home, r.ht_away) {
            (Some(h), Some(a)) => (h, a),
            _ => return ungraded("no halftime score"),
        };
        let half_goals = if market.contains("1st Half") {
            hh + ha
        } else {
            (r.home_goals - hh) + (r.away_goals - ha)
        };
        let thr: f64 = line.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect::<String>().parse().unwrap_or(0.5);
        let half = if market.contains("1st Half") { "1H" } else { "2H" };
        return won_ou(line.to_lowercase().contains("over"), half_goals, thr, format!("{half} {} goals", half_goals as i64));
    }

    // Any goals O/U line (1.5 / 2.5 / 3.5 / …), not just 2.5.
    if (market.starts_with("Over ") || market.starts_with("Under ")) && market.ends_with("Goals") {
        let thr: f64 = market
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '.')
            .collect::<String>()
            .parse()
            .unwrap_or(2.5);
        return won_ou(market.starts_with("Over"), total, thr, format!("FT total {}", total as i64));
    }

    match market {
        "BTTS" => won(
            r.home_goals > 0.0 && r.away_goals > 0.0,
            format!("FT {}-{}", r.home_goals as i64, r.away_goals as i64),
        ),
        "Both Teams Carded" => won(
            r.home_cards >= 1.0 && r.away_cards >= 1.0,
            format!("cards {}-{}", r.home_cards as i64, r.away_cards as i64),
        ),
        "Most Cards" => {
            let sel = leg.selection.to_lowercase();
            let is_home = r.home_name.to_lowercase().contains(&sel) || sel.contains(&r.home_name.to_lowercase());
            let (mine, theirs) = if is_home { (r.home_cards, r.away_cards) } else { (r.away_cards, r.home_cards) };
            won(mine > theirs, format!("cards {}-{}", r.home_cards as i64, r.away_cards as i64))
        }
        "Most Corners" | "Most Shots" => {
            let sel = leg.selection.to_lowercase();
            let hl = r.home_name.to_lowercase();
            let is_home = hl.contains(&sel) || sel.contains(&hl);
            let (h, a) = if market == "Most Corners" { (r.home_corners, r.away_corners) } else { (r.home_shots, r.away_shots) };
            match (h, a) {
                (Some(hv), Some(av)) => {
                    let (mine, theirs) = if is_home { (hv, av) } else { (av, hv) };
                    won(mine > theirs, format!("{} {}-{}", if market == "Most Corners" { "corners" } else { "shots" }, hv as i64, av as i64))
                }
                _ => ungraded("no team stats"),
            }
        }
        "Team Total Cards" => {
            let sel = leg.selection.to_lowercase();
            let is_home = r.home_name.to_lowercase().contains(&sel) || sel.contains(&r.home_name.to_lowercase());
            let val = if is_home { r.home_cards } else { r.away_cards };
            let thr: f64 = line.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect::<String>().parse().unwrap_or(1.5);
            won_ou(line.to_lowercase().contains("over"), val, thr, format!("{} {} cards", leg.selection, val as i64))
        }
        "Correct Score" => {
            let p: Vec<i64> = line.split('-').filter_map(|s| s.trim().parse().ok()).collect();
            if p.len() == 2 {
                won(
                    r.home_goals as i64 == p[0] && r.away_goals as i64 == p[1],
                    format!("FT {}-{}", r.home_goals as i64, r.away_goals as i64),
                )
            } else {
                ungraded("bad score line")
            }
        }
        "Goals Range" => {
            let nums: Vec<i64> = line
                .split(|c: char| !c.is_ascii_digit())
                .filter_map(|s| s.parse().ok())
                .collect();
            if nums.len() >= 2 {
                let t = total as i64;
                won(t >= nums[0] && t <= nums[1], format!("FT total {t}"))
            } else {
                ungraded("bad range")
            }
        }
        "Team Total Goals" => {
            let sel = leg.selection.to_lowercase();
            let hl = r.home_name.to_lowercase();
            let is_home = hl.contains(&sel) || sel.contains(&hl);
            let tg = if is_home { r.home_goals } else { r.away_goals };
            let thr: f64 = line
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == '.')
                .collect::<String>()
                .parse()
                .unwrap_or(1.5);
            won_ou(line.to_lowercase().contains("over"), tg, thr, format!("{} {}", leg.selection, tg as i64))
        }
        "Team Corners" | "Team Shots" | "Team Shots Outside Box" | "Team Shots Inside Box" | "Team Offsides" => {
            let sel = leg.selection.to_lowercase();
            let hl = r.home_name.to_lowercase();
            let is_home = hl.contains(&sel) || sel.contains(&hl);
            let (h, a) = match market {
                "Team Corners" => (r.home_corners, r.away_corners),
                "Team Shots" => (r.home_shots, r.away_shots),
                "Team Shots Outside Box" => (r.home_outbox, r.away_outbox),
                "Team Offsides" => (r.home_offsides, r.away_offsides),
                _ => (r.home_inbox, r.away_inbox),
            };
            let val = if is_home { h } else { a };
            match val {
                Some(v) => {
                    let thr: f64 = line.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect::<String>().parse().unwrap_or(0.5);
                    won_ou(line.to_lowercase().contains("over"), v, thr, format!("{} {}", leg.selection, v as i64))
                }
                None => ungraded("no team stats"),
            }
        }
        "Asian Handicap" => {
            // Line carries the signed handicap, e.g. "-1.5 (win by 2+)" → -1.5.
            // The named team covers iff (its margin) + handicap > 0. Works for any
            // half-line (-0.5/+0.5/-1.5/+1.5…) and is backward-compatible.
            let is_home = leg.selection.eq_ignore_ascii_case(&r.home_name);
            let (gf, ga) = if is_home {
                (r.home_goals, r.away_goals)
            } else {
                (r.away_goals, r.home_goals)
            };
            let hcap: f64 = line
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+')
                .collect::<String>()
                .parse()
                .unwrap_or(0.0);
            won((gf - ga) as f64 + hcap > 0.0, format!("FT {}-{}", r.home_goals as i64, r.away_goals as i64))
        }
        "Win 1st Half" => match (r.ht_home, r.ht_away) {
            (Some(h), Some(a)) => {
                let is_home = leg.selection.eq_ignore_ascii_case(&r.home_name);
                let (gf, ga) = if is_home { (h, a) } else { (a, h) };
                won(gf > ga, format!("HT {}-{}", h as i64, a as i64))
            }
            _ => ungraded("no half-time score"),
        },
        "Match Result" => {
            let ft = format!("FT {}-{}", r.home_goals as i64, r.away_goals as i64);
            let sel = leg.selection.to_lowercase();
            if sel == "draw" || sel == "x" || sel == "the draw" {
                return won(r.home_goals == r.away_goals, ft);
            }
            let is_home = leg.selection.eq_ignore_ascii_case(&r.home_name);
            let (gf, ga) = if is_home { (r.home_goals, r.away_goals) } else { (r.away_goals, r.home_goals) };
            won(gf > ga, ft)
        }
        "First Team to Score" => {
            let total = (r.home_goals + r.away_goals) as i64;
            let sel = leg.selection.to_lowercase();
            if sel.contains("no goal") || sel == "neither" {
                won(total == 0, format!("FT {}-{}", r.home_goals as i64, r.away_goals as i64))
            } else if total == 0 {
                won(false, "0-0, no scorer".to_string())
            } else {
                match &r.first_scorer_team {
                    Some(t) => won(t.eq_ignore_ascii_case(&leg.selection), format!("first: {t}")),
                    None => ungraded("no goal timeline"),
                }
            }
        }
        "Double Chance" => {
            let team = leg.selection.to_lowercase().replace(" or draw", "");
            let team = team.trim();
            let hl = r.home_name.to_lowercase();
            let is_home = hl.contains(team) || team.contains(&hl);
            let (gf, ga) = if is_home { (r.home_goals, r.away_goals) } else { (r.away_goals, r.home_goals) };
            won(gf >= ga, format!("FT {}-{}", r.home_goals as i64, r.away_goals as i64))
        }
        "Win 2nd Half" => match (r.ht_home, r.ht_away) {
            (Some(h), Some(a)) => {
                let sh = r.home_goals - h;
                let sa = r.away_goals - a;
                let is_home = leg.selection.eq_ignore_ascii_case(&r.home_name);
                let (gf, ga) = if is_home { (sh, sa) } else { (sa, sh) };
                won(gf > ga, format!("2H {}-{}", sh as i64, sa as i64))
            }
            _ => ungraded("no half-time score"),
        },
        // Player markets
        "Anytime Scorer" | "Anytime Assist" | "Shots on Target" | "Player Shots" | "Tackles"
        | "Fouls Committed" | "Fouls Drawn" | "To Be Carded" | "Passes Completed" | "Goalkeeper Saves" => {
            let p = match lookup_player(&r.players, &leg.selection) {
                Lookup::Found(p) => p,
                Lookup::Ambiguous => return ungraded("ambiguous player name — grade manually"),
                Lookup::Absent => {
                    // We HAVE the match's player stats but this player isn't in
                    // them → they never featured → books void the prop. An empty
                    // map means the league has no stats coverage → ungradeable.
                    return if r.players.is_empty() {
                        ungraded("no player stats")
                    } else {
                        voided("did not feature — void")
                    };
                }
            };
            let k = threshold(&line);
            // API player stats cover the WHOLE match; for AET games books settle
            // props on 90 min — we can't split, so grade but say so (honest-data).
            let et = if r.extra_time { " (incl. ET)" } else { "" };
            match market {
                "Anytime Scorer" => won(p.goals >= 1.0, format!("{} goals{et}", p.goals as i64)),
                "Anytime Assist" => won(p.assists >= 1.0, format!("{} assists{et}", p.assists as i64)),
                "Shots on Target" => won(p.sot >= k, format!("{} on target{et}", p.sot as i64)),
                "Player Shots" => won(p.shots >= k, format!("{} shots{et}", p.shots as i64)),
                "Tackles" => won(p.tackles >= k, format!("{} tackles{et}", p.tackles as i64)),
                "Fouls Committed" => won(p.fouls_committed >= k, format!("{} fouls{et}", p.fouls_committed as i64)),
                "Fouls Drawn" => won(p.fouls_drawn >= k, format!("{} drawn{et}", p.fouls_drawn as i64)),
                "To Be Carded" => won(p.cards >= 1.0, format!("{} cards{et}", p.cards as i64)),
                "Passes Completed" => won(p.passes >= k, format!("{} passes{et}", p.passes as i64)),
                "Goalkeeper Saves" => won(p.saves >= k, format!("{} saves{et}", p.saves as i64)),
                _ => ungraded("unsupported"),
            }
        }
        _ => ungraded("market not auto-graded"),
    }
}

/// Per-run fixture-result cache. Settling N tickets that share a fixture must
/// cost ONE result fetch, not N — pass one of these across the whole run.
#[derive(Default)]
pub struct ResultCache(HashMap<i64, Option<FixtureResult>>);

impl ResultCache {
    /// Home team of a fixture already graded this run (for CLV odds matching).
    pub fn home_of(&self, fixture_id: i64) -> Option<String> {
        self.0
            .get(&fixture_id)
            .and_then(|o| o.as_ref())
            .map(|r| r.home_name.clone())
            .filter(|s| !s.is_empty())
    }
}

/// Grade every leg of a ticket, reusing (and filling) the shared `cache`.
pub async fn grade_legs_cached(
    state: &AppState,
    legs: &[TicketLeg],
    cache: &mut ResultCache,
) -> Vec<LegResult> {
    for leg in legs {
        if leg.fixture_id != 0 && !cache.0.contains_key(&leg.fixture_id) {
            let r = fetch_result(state, leg.fixture_id).await;
            cache.0.insert(leg.fixture_id, r);
        }
    }
    legs.iter()
        .map(|leg| match cache.0.get(&leg.fixture_id).and_then(|o| o.as_ref()) {
            Some(r) => grade_leg(leg, r),
            None => ungraded("no result data"),
        })
        .collect()
}

