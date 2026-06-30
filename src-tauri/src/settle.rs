//! Bet settlement: fetch finished-match results + player stats and grade each
//! leg of a ticket. Highlights what hit even when the whole ticket misses.

use std::collections::HashMap;

use serde_json::Value;

use crate::apifootball::{self as af, response_array};
use crate::models::{LegResult, TicketLeg};
use crate::AppState;

const TTL_RESULT: i64 = 7 * 24 * 3600;

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

fn parse_finished(j: &Value) -> bool {
    response_array(j)
        .first()
        .and_then(|e| e.get("fixture"))
        .and_then(|f| f.get("status"))
        .and_then(|s| s.get("short"))
        .and_then(|v| v.as_str())
        .map(|s| matches!(s, "FT" | "AET" | "PEN"))
        .unwrap_or(false)
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
        .map(|dt| af::now_ts() >= dt.timestamp() + 150 * 60)
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
    if !parse_finished(&fj) && expected_ended(&fj) {
        if let Ok(fresh) =
            af::fetch_fresh(state, "/fixtures", vec![("id", fixture_id.to_string())], TTL_RECHECK).await
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

    let goals = e.get("goals");
    let home_goals = goals.and_then(|g| g.get("home")).map(num).unwrap_or(0.0);
    let away_goals = goals.and_then(|g| g.get("away")).map(num).unwrap_or(0.0);
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
    let players_params = vec![("fixture", fixture_id.to_string())];
    let pj_res = if forced {
        af::fetch_fresh(state, "/fixtures/players", players_params, TTL_RECHECK).await
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
        af::fetch_fresh(state, "/fixtures/statistics", stats_params, TTL_RECHECK).await
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
        if let Ok(ej) = af::cached_get(state, "/fixtures/events", vec![("fixture", fixture_id.to_string())], TTL_RESULT).await {
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
    LegResult { won: Some(b), detail, margin: None }
}
/// O/U-style result: `over` = the bet direction, `actual`/`thr` the value & line.
/// Records the signed gap to the line (positive = cleared it, negative = missed).
fn won_ou(over: bool, actual: f64, thr: f64, detail: String) -> LegResult {
    let hit = if over { actual > thr } else { actual < thr };
    let margin = if over { actual - thr } else { thr - actual };
    LegResult { won: Some(hit), detail, margin: Some((margin * 100.0).round() / 100.0) }
}
fn ungraded(detail: &str) -> LegResult {
    LegResult { won: None, detail: detail.to_string(), margin: None }
}

fn lookup_player<'a>(players: &'a HashMap<String, PlayerLine>, name: &str) -> Option<&'a PlayerLine> {
    let n = crate::odds::fold(name);
    if let Some(p) = players.get(&n) {
        return Some(p);
    }
    let last = n.rsplit(' ').next().unwrap_or(&n);
    players.iter().find(|(k, _)| k.contains(&n) || k.contains(last)).map(|(_, v)| v)
}

fn grade_leg(leg: &TicketLeg, r: &FixtureResult) -> LegResult {
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
            let is_home = leg.selection.eq_ignore_ascii_case(&r.home_name);
            let (gf, ga) = if is_home { (r.home_goals, r.away_goals) } else { (r.away_goals, r.home_goals) };
            won(gf > ga, format!("FT {}-{}", r.home_goals as i64, r.away_goals as i64))
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
                Some(p) => p,
                None => return ungraded("no player stats"),
            };
            let k = threshold(&line);
            match market {
                "Anytime Scorer" => won(p.goals >= 1.0, format!("{} goals", p.goals as i64)),
                "Anytime Assist" => won(p.assists >= 1.0, format!("{} assists", p.assists as i64)),
                "Shots on Target" => won(p.sot >= k, format!("{} on target", p.sot as i64)),
                "Player Shots" => won(p.shots >= k, format!("{} shots", p.shots as i64)),
                "Tackles" => won(p.tackles >= k, format!("{} tackles", p.tackles as i64)),
                "Fouls Committed" => won(p.fouls_committed >= k, format!("{} fouls", p.fouls_committed as i64)),
                "Fouls Drawn" => won(p.fouls_drawn >= k, format!("{} drawn", p.fouls_drawn as i64)),
                "To Be Carded" => won(p.cards >= 1.0, format!("{} cards", p.cards as i64)),
                "Passes Completed" => won(p.passes >= k, format!("{} passes", p.passes as i64)),
                "Goalkeeper Saves" => won(p.saves >= k, format!("{} saves", p.saves as i64)),
                _ => ungraded("unsupported"),
            }
        }
        _ => ungraded("market not auto-graded"),
    }
}

/// Grade every leg of a ticket. Fetches each distinct fixture's result once.
pub async fn grade_legs(state: &AppState, legs: &[TicketLeg]) -> Vec<LegResult> {
    let mut results_by_fixture: HashMap<i64, Option<FixtureResult>> = HashMap::new();
    for leg in legs {
        if leg.fixture_id != 0 && !results_by_fixture.contains_key(&leg.fixture_id) {
            let r = fetch_result(state, leg.fixture_id).await;
            results_by_fixture.insert(leg.fixture_id, r);
        }
    }
    legs.iter()
        .map(|leg| match results_by_fixture.get(&leg.fixture_id).and_then(|o| o.as_ref()) {
            Some(r) => grade_leg(leg, r),
            None => ungraded("no result data"),
        })
        .collect()
}
