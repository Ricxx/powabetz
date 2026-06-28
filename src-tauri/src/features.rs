//! Deterministic feature engine. ALL arithmetic happens here — the model never
//! computes or invents a number.
//!
//! Core principle (per addendum): for EVERY market, compare the subject's
//! underlying rate against the line and emit `est_prob` = P(line hits). The
//! scoring-drought guard is just the scorer-specific case of this rule. Legs are
//! ranked purely by likelihood; markets are treated equally (no xG bias).
//!
//! Form basis: season aggregates from `/players` and `/teams/statistics` (see
//! decision.md D2). Proxied/missing values are always flagged (honest-data rule).

use serde_json::{json, Value};

use crate::apifootball::response_array;
use crate::models::Candidate;

// ---------- small math ----------

fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}
fn clampp(p: f64) -> f64 {
    p.clamp(0.01, 0.99)
}

/// P(X >= 1) for Poisson(l).
fn pois_ge1(l: f64) -> f64 {
    1.0 - (-l).exp()
}
/// P(X >= 2) for Poisson(l).
fn pois_ge2(l: f64) -> f64 {
    1.0 - (-l).exp() * (1.0 + l)
}
/// P(X = k) for Poisson(l), small k.
fn pois_pmf(k: u32, l: f64) -> f64 {
    let mut term = (-l).exp();
    for i in 1..=k {
        term *= l / i as f64;
    }
    term
}
/// P(X <= k) for Poisson(l), small k.
fn pois_cdf(k: u32, l: f64) -> f64 {
    let mut term = (-l).exp();
    let mut sum = term;
    for i in 1..=k {
        term *= l / i as f64;
        sum += term;
    }
    sum
}
fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}
/// Standard normal CDF via erf approximation (Abramowitz & Stegun 7.1.26).
fn norm_cdf(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.2316419 * x.abs());
    let d = 0.3989423 * (-x * x / 2.0).exp();
    let p = d
        * t
        * (0.3193815 + t * (-0.3565638 + t * (1.781478 + t * (-1.821256 + t * 1.330274))));
    if x >= 0.0 {
        1.0 - p
    } else {
        p
    }
}

fn num(v: &Value) -> f64 {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
        .unwrap_or(0.0)
}

// ---------- player season parsing ----------

#[derive(Default)]
struct PlayerSeason {
    name: String,
    position: String,
    height_cm: f64,
    apps: f64,
    minutes: f64,
    goals: f64,
    assists: f64,
    shots: f64,
    sot: f64,
    tackles: f64,
    fouls_for: f64,
    fouls_drawn: f64,
    cards: f64,
    passes: f64,
    saves: f64,
}

fn parse_player_season(json: &Value, league_id: i64) -> Option<PlayerSeason> {
    parse_season_entry(response_array(json).first()?, league_id)
}

/// Parse one `/players` response entry ({player, statistics}) into season totals.
fn parse_season_entry(entry: &Value, league_id: i64) -> Option<PlayerSeason> {
    let mut s = PlayerSeason {
        name: entry
            .get("player")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("Unknown")
            .to_string(),
        height_cm: entry
            .get("player")
            .and_then(|p| p.get("height"))
            .and_then(|h| h.as_str())
            .map(|h| h.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0.0))
            .unwrap_or(0.0),
        ..Default::default()
    };
    let stats = entry.get("statistics").and_then(|s| s.as_array())?;

    let mut matched = false;
    for row in stats {
        let lid = row
            .get("league")
            .and_then(|l| l.get("id"))
            .and_then(|i| i.as_i64())
            .unwrap_or(-1);
        if lid == league_id {
            matched = true;
            accumulate_player(&mut s, row);
        }
    }
    if !matched {
        for row in stats {
            accumulate_player(&mut s, row);
        }
    }
    Some(s)
}

fn accumulate_player(s: &mut PlayerSeason, row: &Value) {
    let g = |a: &str, b: &str| row.get(a).and_then(|x| x.get(b)).map(num).unwrap_or(0.0);
    s.apps += g("games", "appearences");
    s.minutes += g("games", "minutes");
    if s.position.is_empty() {
        s.position = row
            .get("games")
            .and_then(|x| x.get("position"))
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_string();
    }
    s.goals += g("goals", "total");
    s.assists += g("goals", "assists");
    s.shots += g("shots", "total");
    s.sot += g("shots", "on");
    s.tackles += g("tackles", "total");
    s.fouls_for += g("fouls", "committed");
    s.fouls_drawn += g("fouls", "drawn");
    s.cards += g("cards", "yellow") + g("cards", "yellowred") + g("cards", "red");
    s.passes += g("passes", "total");
    s.saves += g("goals", "saves");
}

/// Parse a cached `/players` body into the inspector view (raw season totals +
/// per-90 rates — exactly the engine's inputs, for the user to review).
pub fn parse_player_inspect(json: &Value, league_id: i64) -> Option<crate::models::PlayerInspect> {
    let s = parse_player_season(json, league_id)?;
    let nineties = if s.minutes > 0.0 { s.minutes / 90.0 } else { 0.0 };
    let p90 = |t: f64| if nineties > 0.0 { r2(t / nineties) } else { 0.0 };
    Some(crate::models::PlayerInspect {
        name: s.name.clone(),
        position: s.position.clone(),
        apps: s.apps,
        minutes: s.minutes,
        goals: s.goals,
        shots: s.shots,
        sot: s.sot,
        tackles: s.tackles,
        fouls_committed: s.fouls_for,
        fouls_drawn: s.fouls_drawn,
        cards: s.cards,
        passes: s.passes,
        per90: crate::models::PlayerRates {
            goals: p90(s.goals),
            sot: p90(s.sot),
            shots: p90(s.shots),
            tackles: p90(s.tackles),
            fouls: p90(s.fouls_for),
            cards: p90(s.cards),
            passes: p90(s.passes),
        },
    })
}

/// Convert internal TeamStats into the serialisable inspector view.
pub fn team_stats_view(t: &TeamStats) -> crate::models::TeamStatsView {
    crate::models::TeamStatsView {
        played: t.played,
        gf_avg: r2(t.gf_avg),
        ga_avg: r2(t.ga_avg),
        ppg: r2(t.ppg),
        first_half_share: r2(t.first_half_share),
        fts_rate: r2(t.fts_rate),
    }
}

pub struct FixtureCtx {
    pub fixture_label: String,
    pub fixture_id: i64,
    pub team: String,
    pub opponent: String,
    pub is_home: bool,
    pub availability: String,
}

// ---------- player candidate generation ----------

/// Build candidates from a single `/players` entry. `groups` are the selected
/// player-market toggle keys.
pub fn build_player_candidates_entry(
    entry: &Value,
    league_id: i64,
    ctx: &FixtureCtx,
    groups: &[String],
    in_form: &std::collections::HashSet<String>,
) -> Vec<Candidate> {
    let s = match parse_season_entry(entry, league_id) {
        Some(s) => s,
        None => return Vec::new(),
    };
    let hot = in_form.contains(&crate::odds::fold(&s.name));
    let mut out = Vec::new();

    let gp = s.apps.max(1.0);
    let exp_min = (s.minutes / gp).clamp(0.0, 90.0);
    if s.minutes <= 0.0 {
        return out; // no minutes this season → nothing to rate on
    }
    let nineties = s.minutes / 90.0;
    let per90 = |total: f64| if nineties > 0.0 { total / nineties } else { 0.0 };

    let goals_p90 = per90(s.goals);
    let shots_p90 = per90(s.shots);
    let sot_p90 = per90(s.sot);
    let tackles_p90 = per90(s.tackles);
    let fouls_p90 = per90(s.fouls_for);
    let fdrawn_p90 = per90(s.fouls_drawn);
    let cards_p90 = per90(s.cards);
    let passes_p90 = per90(s.passes);
    let assists_p90 = per90(s.assists);
    let saves_p90 = per90(s.saves);
    let min_scale = exp_min / 90.0;
    let is_keeper = s.position.to_lowercase().contains("goalkeeper") || s.position.eq_ignore_ascii_case("g");

    // Availability: injured/suspended unlikely to feature → sink + flag.
    let avail_mult = match ctx.availability.as_str() {
        "injured" | "suspended" => 0.15,
        "doubtful" => 0.85,
        _ => 1.0,
    };
    let avail_flag = |flags: &mut Vec<String>| match ctx.availability.as_str() {
        "injured" | "suspended" => flags.push(format!("{} — unlikely to feature", ctx.availability)),
        "doubtful" => flags.push("doubtful — minutes at risk".to_string()),
        "unknown" => flags.push("availability unknown".to_string()),
        _ => {}
    };

    // Defensive-workload proxy (rises for the away/likely-lower-possession side).
    let workload = if ctx.is_home { 0.95 } else { 1.10 };

    let base = |market: &str, group: &str, line: &str, rate: f64, est: f64, support: Vec<String>, mut flags: Vec<String>| {
        avail_flag(&mut flags);
        Candidate {
            subject: s.name.clone(),
            subject_kind: "player".to_string(),
            team: ctx.team.clone(),
            opponent: ctx.opponent.clone(),
            fixture: ctx.fixture_label.clone(),
            fixture_id: ctx.fixture_id,
            market: market.to_string(),
            market_group: group.to_string(),
            line: line.to_string(),
            base_rate: r2(rate),
            est_prob: r2(clampp(est * avail_mult)),
            pinnacle_prob: None,
            book_odds: None,
            book: None,
            ev: None,
            ev_source: None,
            form_state: None,
            xg_source: None,
            support,
            flags,
            plausibility: None,
        }
    };

    for g in groups {
        match g.as_str() {
            "scorer" => {
                // xG only enters here, as one optional input.
                let proxy_xg_p90 = sot_p90 * 0.30 + (shots_p90 - sot_p90).max(0.0) * 0.05;
                let lambda = goals_p90.max(0.18 * sot_p90) * min_scale;
                let mut est = pois_ge1(lambda);
                let form = classify(goals_p90, proxy_xg_p90, sot_p90);
                let mut flags = vec!["xG is estimated (proxy)".to_string()];
                match form.as_str() {
                    "cold_falling_off" => {
                        est *= 0.6;
                        flags.push("cold: chances dried up — not 'due'".to_string());
                    }
                    "due_regression" => flags.push("due: healthy chances despite drought".to_string()),
                    _ => {}
                }
                let mut sup = vec![
                    format!("goals/90 {:.2}", goals_p90),
                    format!("sot/90 {:.2}", sot_p90),
                    format!("proxy_xg/90 {:.2}", proxy_xg_p90),
                    format!("exp_min {:.0}", exp_min),
                ];
                // Honest context only (no probability change): tall players carry an
                // aerial/set-piece edge the model can weigh.
                if s.height_cm >= 188.0 {
                    sup.push(format!("{:.0}cm — aerial threat on set pieces", s.height_cm));
                } else if s.height_cm > 0.0 {
                    sup.push(format!("{:.0}cm", s.height_cm));
                }
                if !s.position.is_empty() {
                    sup.push(s.position.clone());
                }
                if hot {
                    flags.push("in-form: among the league's top scorers/assisters this season".to_string());
                }
                let mut c = base("Anytime Scorer", "scorer", "1+ goal", goals_p90, est, sup, flags);
                c.form_state = Some(form);
                c.xg_source = Some("proxy".to_string());
                out.push(c);
            }
            "sot" => {
                let lambda = sot_p90 * min_scale;
                let (line, est) = if pois_ge2(lambda) >= 0.5 {
                    ("2+ shots on target", pois_ge2(lambda))
                } else {
                    ("1+ shot on target", pois_ge1(lambda))
                };
                out.push(base(
                    "Shots on Target",
                    "sot",
                    line,
                    sot_p90,
                    est,
                    vec![format!("sot/90 {:.2}", sot_p90), format!("exp_min {:.0}", exp_min)],
                    vec![],
                ));
            }
            "tackles" => {
                let lambda = tackles_p90 * workload * min_scale;
                let (line, est) = if pois_ge2(lambda) >= 0.45 {
                    ("2+ tackles", pois_ge2(lambda))
                } else {
                    ("1+ tackle", pois_ge1(lambda))
                };
                out.push(base(
                    "Tackles",
                    "tackles",
                    line,
                    tackles_p90,
                    est,
                    vec![
                        format!("tackles/90 {:.2}", tackles_p90),
                        format!("workload x{:.2} ({})", workload, if ctx.is_home { "home" } else { "away" }),
                    ],
                    vec!["workload proxied from home/away (no odds)".to_string()],
                ));
            }
            "fouls" => {
                let lf = fouls_p90 * workload * min_scale;
                let (cline, cest) = if pois_ge2(lf) >= 0.4 {
                    ("2+ fouls committed", pois_ge2(lf))
                } else {
                    ("1+ foul committed", pois_ge1(lf))
                };
                out.push(base(
                    "Fouls Committed",
                    "fouls",
                    cline,
                    fouls_p90,
                    cest,
                    vec![format!("fouls/90 {:.2}", fouls_p90)],
                    vec!["workload proxied from home/away".to_string()],
                ));
                let ld = fdrawn_p90 * min_scale;
                out.push(base(
                    "Fouls Drawn",
                    "fouls",
                    "1+ foul drawn",
                    fdrawn_p90,
                    pois_ge1(ld),
                    vec![format!("fouls_drawn/90 {:.2}", fdrawn_p90)],
                    vec![],
                ));
            }
            "cards" => {
                let lambda = cards_p90 * workload * min_scale;
                out.push(base(
                    "To Be Carded",
                    "cards",
                    "1+ card",
                    cards_p90,
                    pois_ge1(lambda),
                    vec![format!("cards/90 {:.2}", cards_p90)],
                    vec!["card rate is noisy season-wide".to_string()],
                ));
            }
            "passes" => {
                let expected = passes_p90 * min_scale;
                if expected < 8.0 {
                    continue; // not a passing role — skip rather than offer a junk line
                }
                let line_val = ((expected - 0.6 * expected.sqrt()) / 5.0).floor() * 5.0;
                let line_val = line_val.max(10.0);
                let z = (expected - line_val + 0.5) / expected.max(1.0).sqrt();
                out.push(base(
                    "Passes Completed",
                    "passes",
                    &format!("{:.0}+ passes", line_val),
                    passes_p90,
                    norm_cdf(z),
                    vec![format!("passes/90 {:.0}", passes_p90), format!("expected {:.0}", expected)],
                    vec![],
                ));
            }
            "saves" => {
                // Goalkeepers only — anything else has no save data.
                if !is_keeper || saves_p90 < 0.5 {
                    continue;
                }
                let lambda = saves_p90 * min_scale;
                let (line, est) = if pois_ge2(lambda * 1.5) >= 0.5 {
                    ("3+ saves", 1.0 - pois_cdf(2, lambda))
                } else {
                    ("2+ saves", 1.0 - pois_cdf(1, lambda))
                };
                out.push(base(
                    "Goalkeeper Saves",
                    "saves",
                    line,
                    saves_p90,
                    est,
                    vec![format!("saves/90 {:.1}", saves_p90), format!("exp_min {:.0}", exp_min)],
                    vec![],
                ));
            }
            "assists" => {
                let lambda = assists_p90 * min_scale;
                if assists_p90 < 0.05 {
                    continue;
                }
                let af = if hot {
                    vec!["in-form: among the league's top scorers/assisters this season".to_string()]
                } else {
                    vec![]
                };
                out.push(base(
                    "Anytime Assist",
                    "assists",
                    "1+ assist",
                    assists_p90,
                    pois_ge1(lambda),
                    vec![format!("assists/90 {:.2}", assists_p90)],
                    af,
                ));
            }
            "pshots" => {
                let lambda = shots_p90 * min_scale;
                let (line, est) = if pois_ge2(lambda) >= 0.5 {
                    ("2+ shots", pois_ge2(lambda))
                } else {
                    ("1+ shot", pois_ge1(lambda))
                };
                out.push(base(
                    "Player Shots",
                    "pshots",
                    line,
                    shots_p90,
                    est,
                    vec![format!("shots/90 {:.2}", shots_p90), format!("exp_min {:.0}", exp_min)],
                    vec![],
                ));
            }
            _ => {}
        }
    }
    out
}

/// Scorer drought guard (the scorer-specific case of underlying-rate-vs-line).
fn classify(goals_p90: f64, proxy_xg_p90: f64, sot_p90: f64) -> String {
    let low_goals = goals_p90 < 0.3;
    let healthy = sot_p90 >= 0.7;
    let underperforming = proxy_xg_p90 - goals_p90 >= 0.15;
    if low_goals && underperforming && healthy {
        return "due_regression".to_string();
    }
    if low_goals && (proxy_xg_p90 < 0.2 || sot_p90 < 0.4) {
        return "cold_falling_off".to_string();
    }
    if goals_p90 >= 0.5 && proxy_xg_p90 >= 0.3 {
        return "hot_in_form".to_string();
    }
    "neutral".to_string()
}

// ---------- team statistics parsing + candidates ----------

#[derive(Default, Clone)]
#[allow(dead_code)] // played/fts_rate are parsed for future markets, not yet ranked on
pub struct TeamStats {
    pub name: String,
    pub played: f64,
    pub gf_avg: f64,
    pub ga_avg: f64,
    pub fts_rate: f64, // failed to score
    pub ppg: f64,
    pub first_half_share: f64, // share of goals scored in 1st half
    /// Real xG for/against per match from recent fixtures (None = unavailable,
    /// fall back to goal averages).
    pub xg_for: Option<f64>,
    pub xg_against: Option<f64>,
    /// Recent-form averages for corner/shots markets (None = unavailable).
    pub corners_for: Option<f64>,
    pub corners_against: Option<f64>,
    pub shots_for: Option<f64>,
    pub shots_against: Option<f64>,
    pub outbox_for: Option<f64>,
    pub inbox_for: Option<f64>,
    pub offsides_for: Option<f64>,
}

pub fn parse_team_stats(json: &Value, name: &str) -> Option<TeamStats> {
    let r = json.get("response")?;
    let played = r
        .get("fixtures")
        .and_then(|f| f.get("played"))
        .and_then(|p| p.get("total"))
        .map(num)
        .unwrap_or(0.0);
    if played <= 0.0 {
        return None;
    }
    let gf_avg = r
        .get("goals")
        .and_then(|g| g.get("for"))
        .and_then(|f| f.get("average"))
        .and_then(|a| a.get("total"))
        .map(num)
        .unwrap_or(0.0);
    let ga_avg = r
        .get("goals")
        .and_then(|g| g.get("against"))
        .and_then(|f| f.get("average"))
        .and_then(|a| a.get("total"))
        .map(num)
        .unwrap_or(0.0);
    let fts = r
        .get("failed_to_score")
        .and_then(|f| f.get("total"))
        .map(num)
        .unwrap_or(0.0);
    let wins = r.get("fixtures").and_then(|f| f.get("wins")).and_then(|w| w.get("total")).map(num).unwrap_or(0.0);
    let draws = r.get("fixtures").and_then(|f| f.get("draws")).and_then(|w| w.get("total")).map(num).unwrap_or(0.0);

    // First-half goal share from the minute buckets.
    let minute = r.get("goals").and_then(|g| g.get("for")).and_then(|f| f.get("minute"));
    let bucket = |key: &str| {
        minute
            .and_then(|m| m.get(key))
            .and_then(|b| b.get("total"))
            .map(num)
            .unwrap_or(0.0)
    };
    let first = bucket("0-15") + bucket("16-30") + bucket("31-45");
    let second = bucket("46-60") + bucket("61-75") + bucket("76-90");
    let total_halves = first + second;
    let first_half_share = if total_halves > 0.0 { first / total_halves } else { 0.5 };

    Some(TeamStats {
        name: name.to_string(),
        played,
        gf_avg,
        ga_avg,
        fts_rate: fts / played,
        ppg: (wins * 3.0 + draws) / played,
        first_half_share,
        xg_for: None,
        xg_against: None,
        corners_for: None,
        offsides_for: None,
        corners_against: None,
        shots_for: None,
        shots_against: None,
        outbox_for: None,
        inbox_for: None,
    })
}

/// Team/match-line candidates. `groups` are the selected team-market keys.
pub fn build_team_candidates(
    home: &TeamStats,
    away: &TeamStats,
    fixture_label: &str,
    fixture_id: i64,
    groups: &[String],
) -> Vec<Candidate> {
    let mut out = Vec::new();
    // Use real xG (recent fixtures) where both teams have it — averaging a team's
    // attack xG with the opponent's defensive xG-conceded — else season goal rates.
    let h_for = home.xg_for.unwrap_or(home.gf_avg);
    let h_against = home.xg_against.unwrap_or(home.ga_avg);
    let a_for = away.xg_for.unwrap_or(away.gf_avg);
    let a_against = away.xg_against.unwrap_or(away.ga_avg);
    let lambda_home = ((h_for + a_against) / 2.0).max(0.05);
    let lambda_away = ((a_for + h_against) / 2.0).max(0.05);
    let xg_used = home.xg_for.is_some() && away.xg_for.is_some();
    let crude = if xg_used {
        "team line: real xG (recent form)".to_string()
    } else {
        "team line: crude season-rate proxy".to_string()
    };

    let mk = |subject: &str, market: &str, group: &str, line: &str, rate: f64, est: f64, support: Vec<String>| Candidate {
        subject: subject.to_string(),
        subject_kind: "team".to_string(),
        team: subject.to_string(),
        opponent: String::new(),
        fixture: fixture_label.to_string(),
        fixture_id,
        market: market.to_string(),
        market_group: group.to_string(),
        line: line.to_string(),
        base_rate: r2(rate),
        est_prob: r2(clampp(est)),
        pinnacle_prob: None,
        book_odds: None,
        book: None,
        ev: None,
        ev_source: None,
        form_state: None,
        xg_source: None,
        support,
        flags: vec![crude.clone()],
        plausibility: None,
    };

    for g in groups {
        match g.as_str() {
            "btts" => {
                let p_home = pois_ge1(lambda_home);
                let p_away = pois_ge1(lambda_away);
                out.push(mk(
                    "Both Teams",
                    "BTTS",
                    "btts",
                    "Yes",
                    (p_home * p_away) * 100.0,
                    p_home * p_away,
                    vec![
                        format!("xg_home {:.2}", lambda_home),
                        format!("xg_away {:.2}", lambda_away),
                    ],
                ));
            }
            "ou25" => {
                let total = lambda_home + lambda_away;
                // Both sides of each goal line so the user can isolate over OR under.
                for thresh in [1u32, 2, 3] {
                    let line_val = thresh as f64 + 0.5;
                    let over = 1.0 - pois_cdf(thresh, total);
                    let sup = vec![format!("exp_goals {:.2}", total)];
                    out.push(mk("Match", &format!("Over {line_val:.1} Goals"), "ou25", &format!("Over {line_val:.1}"), total, over, sup.clone()));
                    out.push(mk("Match", &format!("Under {line_val:.1} Goals"), "ou25", &format!("Under {line_val:.1}"), total, 1.0 - over, sup));
                }
            }
            "h1goals" | "h2goals" => {
                // Goals in a single half. 1H ≈ ~45% of the match (per-team share).
                let first = g == "h1goals";
                let sh = |s: f64| if first { s } else { 1.0 - s };
                let lam = lambda_home * sh(home.first_half_share) + lambda_away * sh(away.first_half_share);
                let half = if first { "1st Half" } else { "2nd Half" };
                let sup = vec![format!("{half} exp_goals {lam:.2}")];
                for thresh in [0u32, 1] {
                    let line_val = thresh as f64 + 0.5;
                    let over = 1.0 - pois_cdf(thresh, lam);
                    out.push(mk("Match", &format!("{half} Over {line_val:.1} Goals"), g, &format!("Over {line_val:.1}"), lam, over, sup.clone()));
                    out.push(mk("Match", &format!("{half} Under {line_val:.1} Goals"), g, &format!("Under {line_val:.1}"), lam, 1.0 - over, sup.clone()));
                }
            }
            "exactscore" => {
                // Most-likely correct scores from the independent-Poisson grid.
                let mut scores: Vec<((u32, u32), f64)> = Vec::new();
                for h in 0u32..=5 {
                    for a in 0u32..=5 {
                        scores.push(((h, a), pois_pmf(h, lambda_home) * pois_pmf(a, lambda_away)));
                    }
                }
                scores.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
                for ((h, a), p) in scores.into_iter().take(6) {
                    let line = format!("{h}-{a}");
                    out.push(mk(
                        "Match",
                        "Correct Score",
                        "exactscore",
                        &line,
                        p,
                        clampp(p),
                        vec![format!("xg {lambda_home:.2}-{lambda_away:.2}")],
                    ));
                }
            }
            "goalsrange" => {
                let total = lambda_home + lambda_away;
                let sup = vec![format!("exp_goals {total:.2}")];
                // (lo, hi) inclusive total-goals bands.
                for (lo, hi) in [(0u32, 1u32), (2, 3), (4, 6)] {
                    let p_hi = pois_cdf(hi, total);
                    let p_lo = if lo == 0 { 0.0 } else { pois_cdf(lo - 1, total) };
                    let p = (p_hi - p_lo).clamp(0.0, 1.0);
                    out.push(mk("Match", "Goals Range", "goalsrange", &format!("{lo}-{hi} goals"), total, clampp(p), sup.clone()));
                }
            }
            "tcorners" => {
                for (team, cf) in [(&home.name, home.corners_for), (&away.name, away.corners_for)] {
                    if let Some(lam) = cf {
                        for line in [2.5_f64, 3.5, 4.5, 5.5] {
                            let thr = line.floor() as u32;
                            let over = 1.0 - pois_cdf(thr, lam);
                            let sup = vec![format!("corners/g {lam:.1}")];
                            out.push(mk(team, "Team Corners", "tcorners", &format!("Over {line:.1}"), lam, over, sup.clone()));
                            out.push(mk(team, "Team Corners", "tcorners", &format!("Under {line:.1}"), lam, 1.0 - over, sup));
                        }
                    }
                }
            }
            "toffsides" => {
                for (team, of) in [(&home.name, home.offsides_for), (&away.name, away.offsides_for)] {
                    if let Some(lam) = of {
                        for line in [0.5_f64, 1.5, 2.5] {
                            let thr = line.floor() as u32;
                            let over = 1.0 - pois_cdf(thr, lam);
                            let sup = vec![format!("offsides/g {lam:.1}")];
                            out.push(mk(team, "Team Offsides", "toffsides", &format!("Over {line:.1}"), lam, over, sup.clone()));
                            out.push(mk(team, "Team Offsides", "toffsides", &format!("Under {line:.1}"), lam, 1.0 - over, sup));
                        }
                    }
                }
            }
            "tshots" => {
                for (team, sf) in [(&home.name, home.shots_for), (&away.name, away.shots_for)] {
                    if let Some(lam) = sf {
                        let base = lam.round();
                        for off in [-2.5_f64, -0.5, 1.5] {
                            let line = (base + off).max(2.5);
                            let thr = line.floor() as u32;
                            let over = 1.0 - pois_cdf(thr, lam);
                            let sup = vec![format!("shots/g {lam:.1}")];
                            out.push(mk(team, "Team Shots", "tshots", &format!("Over {line:.1}"), lam, over, sup.clone()));
                            out.push(mk(team, "Team Shots", "tshots", &format!("Under {line:.1}"), lam, 1.0 - over, sup));
                        }
                    }
                }
            }
            "toutbox" => {
                for (team, sf) in [(&home.name, home.outbox_for), (&away.name, away.outbox_for)] {
                    if let Some(lam) = sf {
                        let base = lam.round();
                        for off in [-1.5_f64, 0.5, 2.5] {
                            let line = (base + off).max(0.5);
                            let thr = line.floor() as u32;
                            let over = 1.0 - pois_cdf(thr, lam);
                            let (l, est) = if over >= 0.5 { (format!("Over {line:.1}"), over) } else { (format!("Under {line:.1}"), 1.0 - over) };
                            out.push(mk(team, "Team Shots Outside Box", "toutbox", &l, lam, est, vec![format!("out-box/g {lam:.1}")]));
                        }
                    }
                }
            }
            "tinbox" => {
                for (team, sf) in [(&home.name, home.inbox_for), (&away.name, away.inbox_for)] {
                    if let Some(lam) = sf {
                        let base = lam.round();
                        for off in [-2.5_f64, -0.5, 1.5] {
                            let line = (base + off).max(0.5);
                            let thr = line.floor() as u32;
                            let over = 1.0 - pois_cdf(thr, lam);
                            let (l, est) = if over >= 0.5 { (format!("Over {line:.1}"), over) } else { (format!("Under {line:.1}"), 1.0 - over) };
                            out.push(mk(team, "Team Shots Inside Box", "tinbox", &l, lam, est, vec![format!("in-box/g {lam:.1}")]));
                        }
                    }
                }
            }
            "tgoals" => {
                // Each team's own total goals O/U (e.g. "Germany Over 1.5").
                for (team, lam) in [(&home.name, lambda_home), (&away.name, lambda_away)] {
                    for thresh in [0u32, 1, 2] {
                        let line_val = thresh as f64 + 0.5;
                        let over = 1.0 - pois_cdf(thresh, lam);
                        let sup = vec![format!("exp {:.2}", lam)];
                        out.push(mk(team, "Team Total Goals", "tgoals", &format!("Over {line_val:.1}"), lam, over, sup.clone()));
                        out.push(mk(team, "Team Total Goals", "tgoals", &format!("Under {line_val:.1}"), lam, 1.0 - over, sup));
                    }
                }
            }
            "win" => {
                let diff = (home.ppg - away.ppg) + 0.3;
                let raw_home = sigmoid(0.9 * diff);
                let p_draw = 0.27;
                let p_home = raw_home * (1.0 - p_draw);
                let p_away = (1.0 - raw_home) * (1.0 - p_draw);
                let sup = vec![format!("ppg {:.2} vs {:.2}", home.ppg, away.ppg)];
                out.push(mk(&home.name, "Match Result", "win", "to win", p_home * 100.0, p_home, sup.clone()));
                out.push(mk(&away.name, "Match Result", "win", "to win", p_away * 100.0, p_away, sup));
            }
            "dc" => {
                let diff = (home.ppg - away.ppg) + 0.3;
                let raw_home = sigmoid(0.9 * diff);
                let p_draw = 0.27;
                let p_home = raw_home * (1.0 - p_draw);
                let p_away = (1.0 - raw_home) * (1.0 - p_draw);
                let sup = vec![format!("ppg {:.2} vs {:.2}", home.ppg, away.ppg)];
                out.push(mk(
                    &format!("{} or draw", home.name),
                    "Double Chance",
                    "dc",
                    "draw no bet-ish",
                    (p_home + p_draw) * 100.0,
                    p_home + p_draw,
                    sup.clone(),
                ));
                out.push(mk(
                    &format!("{} or draw", away.name),
                    "Double Chance",
                    "dc",
                    "draw no bet-ish",
                    (p_away + p_draw) * 100.0,
                    p_away + p_draw,
                    sup,
                ));
            }
            "half1" => {
                let lh = lambda_home * home.first_half_share;
                let la = lambda_away * away.first_half_share;
                let p_home = sigmoid(1.3 * (lh - la));
                let (team, line, est) = if p_home >= 0.5 {
                    (home.name.as_str(), "to win 1st half", p_home)
                } else {
                    (away.name.as_str(), "to win 1st half", 1.0 - p_home)
                };
                out.push(mk(team, "Win 1st Half", "half1", line, est * 100.0, est, vec![format!("1H xg {:.2} vs {:.2}", lh, la)]));
            }
            "half2" => {
                let lh = lambda_home * (1.0 - home.first_half_share);
                let la = lambda_away * (1.0 - away.first_half_share);
                let p_home = sigmoid(1.3 * (lh - la));
                let (team, line, est) = if p_home >= 0.5 {
                    (home.name.as_str(), "to win 2nd half", p_home)
                } else {
                    (away.name.as_str(), "to win 2nd half", 1.0 - p_home)
                };
                out.push(mk(team, "Win 2nd Half", "half2", line, est * 100.0, est, vec![format!("2H xg {:.2} vs {:.2}", lh, la)]));
            }
            "ahandicap" => {
                let diff = (home.ppg - away.ppg) + 0.3; // home advantage
                let p_home = sigmoid(0.9 * diff);
                let (team, line, est) = if p_home >= 0.5 {
                    (home.name.as_str(), "-0.5 (to win)", p_home)
                } else {
                    (away.name.as_str(), "+0.5 (draw/win)", 1.0 - p_home + 0.10)
                };
                out.push(mk(team, "Asian Handicap", "ahandicap", line, est * 100.0, clampp(est), vec![format!("ppg {:.2} vs {:.2}", home.ppg, away.ppg)]));
            }
            _ => {}
        }
    }
    out
}

// ---------- odds attachment ----------

/// Attach Pinnacle true-prob + Bet365 odds to a fixture's candidates, and compute
/// EV. The feed prices match-result (→ team-win/handicap), goals O/U, BTTS and
/// anytime scorer; player props stay likelihood-only. When Bet365 is present but
/// Pinnacle isn't, EV falls back to our model probability (tagged "model").
pub fn attach_odds(
    cands: &mut [Candidate],
    odds: &crate::odds::FixtureOdds,
    fixture_label: &str,
    home_team: &str,
) {
    let home_lc = home_team.to_lowercase();
    for c in cands.iter_mut() {
        if c.fixture != fixture_label {
            continue;
        }
        let is_home = c.subject == home_team || c.team == home_team;
        // Side + half-line from a leg's line, e.g. "Over 2.5" → ("over","2.5").
        let ou_side = if c.line.to_lowercase().starts_with("under") { "under" } else { "over" };
        let ou_num: String = c.line.chars().filter(|ch| ch.is_ascii_digit() || *ch == '.').collect();
        // Threshold from a player-prop line, e.g. "2+ shots on target" → 2.
        let thr: i64 = c.line.chars().take_while(|ch| ch.is_ascii_digit()).collect::<String>().parse().unwrap_or(1);

        let priced: Option<crate::odds::Priced> = match c.market_group.as_str() {
            "scorer" => Some(odds.scorer(&c.subject)),
            "assists" => Some(odds.prop("assist", &c.subject, 1)),
            "cards" => Some(odds.prop("card", &c.subject, 1)),
            "sot" => Some(odds.prop("sot", &c.subject, thr)),
            "pshots" => Some(odds.prop("pshots", &c.subject, thr)),
            "fouls" => Some(odds.prop("fouls", &c.subject, thr)),
            "tackles" => Some(odds.prop("tackles", &c.subject, thr)),
            "passes" => Some(odds.prop("passes", &c.subject, thr)),
            "btts" => Some(odds.get("btts|yes")),
            "ou25" => Some(odds.get(&format!("ou|{ou_side}|{ou_num}"))),
            "tgoals" => {
                let side = if is_home { "tgoals_home" } else { "tgoals_away" };
                Some(odds.get(&format!("{side}|{ou_side}|{ou_num}")))
            }
            "tcorners" => {
                let side = if is_home { "corners_home" } else { "corners_away" };
                Some(odds.get(&format!("{side}|{ou_side}|{ou_num}")))
            }
            "win" => Some(odds.get(if is_home { "1x2|home" } else { "1x2|away" })),
            "ahandicap" if c.line.contains("to win") => {
                Some(odds.get(if is_home { "1x2|home" } else { "1x2|away" }))
            }
            "dc" => Some(odds.get(if c.subject.to_lowercase().starts_with(&home_lc) {
                "dc|homedraw"
            } else {
                "dc|awaydraw"
            })),
            _ => None,
        };
        if let Some((pin, book)) = priced {
            c.pinnacle_prob = pin;
            if let Some((o, name)) = book {
                c.book_odds = Some(o);
                c.book = Some(name);
            }
            // EV: prefer the sharp (Pinnacle) line; else fall back to our model prob.
            c.ev = match (pin, c.book_odds) {
                (Some(p), Some(o)) => {
                    c.ev_source = Some("sharp".to_string());
                    Some(r2c(o * p - 1.0))
                }
                (None, Some(o)) => {
                    c.ev_source = Some("model".to_string());
                    Some(r2c(o * c.est_prob - 1.0))
                }
                _ => None,
            };
        }
    }
}

fn r2c(x: f64) -> f64 {
    (x * 10000.0).round() / 10000.0
}

/// Claude's "Oracle" (Confluence) score. The thesis: a bet earns conviction only
/// when independent signals line up — the sharp Pinnacle de-vig says the true
/// probability is solid, OUR model agrees (low divergence, so it isn't a trap
/// line or a model hallucination), the takeable price beats that fair
/// probability (a real edge), and context tilts in favour. It deliberately fades
/// odds-on chalk, lottery longshots, and any leg where the model fights the market.
fn oracle_score(c: &Candidate) -> f64 {
    let p_model = c.est_prob;
    // Consensus probability: sharp-weighted where Pinnacle priced it, else model.
    let (p, divergence, sharp_backed) = match c.pinnacle_prob {
        Some(ps) => (0.6 * ps + 0.4 * p_model, (ps - p_model).abs(), true),
        None => (p_model, 0.0, false),
    };
    // Real edge at the price we can actually take, judged on the consensus prob —
    // plus a sweet-spot odds band (peak ~2.2; fade chalk <1.5 and lottos >3.6).
    let (edge_term, band) = match c.book_odds {
        Some(o) => {
            let edge = o * p - 1.0;
            let edge_term = if edge > 0.0 { edge * 1.3 } else { edge * 0.6 };
            let band = if (1.7..=3.2).contains(&o) {
                0.22 - (o - 2.2).abs() * 0.08
            } else if (1.5..1.7).contains(&o) {
                0.05
            } else if o < 1.5 {
                -0.18 // boring chalk — risk without edge
            } else if o > 3.6 {
                -0.16 // lottery ticket — variance dressed as value
            } else {
                0.04
            };
            (edge_term, band)
        }
        None => (-0.15, -0.10), // unpriced → I can't trust the edge, so I pass
    };
    // Conviction from context (in-form, suited role, trend, certainty to play).
    let has = |needle: &str| c.flags.iter().any(|f| f.contains(needle));
    let mut conviction = 0.0;
    if has("in-form") {
        conviction += 0.10;
    }
    if has("aerial threat") {
        conviction += 0.05;
    }
    if has("unlikely to feature") {
        conviction -= 0.45;
    }
    if has("minutes at risk") {
        conviction -= 0.12;
    }
    match c.form_state.as_deref() {
        Some("cold_falling_off") => conviction -= 0.18,
        Some("due_regression") => conviction += 0.06,
        _ => {}
    }
    // Trust where model & market agree; stand down where they fight.
    let trust = if sharp_backed && divergence < 0.08 { 0.08 } else { 0.0 };
    let disagree = (divergence * 1.2).min(0.30);
    p * 0.5 + edge_term + band + conviction + trust - disagree
}

/// "Power Stacker" score. Hunts HIGH-LIKELIHOOD outcomes ("this should happen")
/// that the book still prices GENEROUSLY (~1.8-2.5) to entice action — so a
/// 2-leg stack clears 4x with few things to connect. Price-driven: an unpriced
/// leg is useless here. Fades odds-on chalk (too short to stack to 4x) and pure
/// longshots (drift from "must happen").
fn power_score(c: &Candidate) -> f64 {
    let p_model = c.est_prob;
    let (p, divergence) = match c.pinnacle_prob {
        Some(ps) => (0.55 * ps + 0.45 * p_model, (ps - p_model).abs()),
        None => (p_model, 0.0),
    };
    let has = |n: &str| c.flags.iter().any(|f| f.contains(n));
    let mut conviction = 0.0;
    if has("in-form") {
        conviction += 0.06;
    }
    if has("unlikely to feature") {
        conviction -= 0.45;
    }
    if has("minutes at risk") {
        conviction -= 0.12;
    }
    if matches!(c.form_state.as_deref(), Some("cold_falling_off")) {
        conviction -= 0.15;
    }
    let disagree = divergence.min(0.25);
    match c.book_odds {
        Some(o) => {
            let edge = o * p - 1.0;
            // Genuinely more-likely-than-not, and priced in the "enticing" band.
            let likely = (p - 0.42).max(0.0);
            let price = if (1.8..=2.6).contains(&o) {
                0.20 // the core: certain-ish at a generous price
            } else if (2.6..=3.2).contains(&o) {
                0.12 // the "less-expected pairer" for ~10x doubles
            } else if (1.6..1.8).contains(&o) {
                0.04
            } else if o < 1.6 {
                -0.25 // too short to stack to 4x efficiently
            } else {
                -0.06 // >3.2 drifts from "must happen"
            };
            p * 0.45 + likely * 0.9 + price + edge.max(0.0) + conviction - disagree
        }
        None => -0.6,
    }
}

// ---------- shortlist + compact table ----------

/// Pre-filter for tokens: rank by likelihood boosted by +EV, capping each market
/// so the shortlist stays diverse enough to build singles, SGPs and SGP+.
/// Rank and trim the candidate pool by strategy `mode`:
/// - "value": likelihood + a +EV bonus (the exploitable edge).
/// - "favorites": in-form picks at USEFUL odds (~1.5-2.5), avoiding boring chalk.
/// - "likely": PURE likelihood (most-probable lines regardless of price).
/// - "oracle": Claude's CONFLUENCE read — sharp + model + edge + context must
///   agree; fades chalk, longshots and model-vs-market disagreement.
/// `per_market_cap` bounds how many of one market (e.g. scorers) reach the model.
pub fn shortlist(mut cands: Vec<Candidate>, n: usize, mode: &str, per_market_cap: usize) -> Vec<Candidate> {
    // Haiku plausibility (1-5) as a small ranking weight: +0.12 at 5, −0.12 at 1,
    // 0 at the neutral 3 (or when unscored). Re-ranks only — never a probability.
    let plaus = |c: &Candidate| -> f64 { c.plausibility.map(|p| (p as f64 - 3.0) * 0.06).unwrap_or(0.0) };
    let base = |c: &Candidate| match mode {
        "value" => c.est_prob + c.ev.unwrap_or(0.0).max(0.0) * 1.5,
        "oracle" => oracle_score(c),
        "power" => power_score(c),
        "favorites" => {
            // Reward a useful odds band; penalise odds-on chalk; weigh form.
            let band = match c.book_odds {
                Some(o) if (1.4..=2.6).contains(&o) => 0.25,
                Some(o) if o < 1.4 => -0.10, // boring chalk
                Some(_) => 0.0,              // longer than the band
                None => -0.05,               // unpriced
            };
            let form = match c.form_state.as_deref() {
                Some("cold_falling_off") => -0.20,
                Some("due_regression") => 0.08,
                _ => 0.0,
            };
            c.est_prob + band + form
        }
        _ => c.est_prob, // "likely"
    };
    let score = |c: &Candidate| base(c) + plaus(c);
    cands.sort_by(|a, b| score(b).partial_cmp(&score(a)).unwrap_or(std::cmp::Ordering::Equal));
    let mut per_market: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut out = Vec::new();
    for c in cands {
        let count = per_market.entry(c.market.clone()).or_insert(0);
        if *count >= per_market_cap {
            continue;
        }
        *count += 1;
        out.push(c);
        if out.len() >= n {
            break;
        }
    }
    out
}

/// Minified JSON the model receives — only what it needs to rank/explain.
pub fn compact_table_json(cands: &[Candidate]) -> String {
    let rows: Vec<Value> = cands
        .iter()
        .map(|c| {
            json!({
                "subject": c.subject,
                "kind": c.subject_kind,
                "fixture": c.fixture,
                "market": c.market,
                "line": c.line,
                "est_prob": c.est_prob,
                "pinnacle_prob": c.pinnacle_prob,
                "book_odds": c.book_odds,
                "ev": c.ev,
                "form_state": c.form_state,
                "xg_source": c.xg_source,
                "plausibility": c.plausibility,
                "flags": c.flags,
            })
        })
        .collect();
    serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string())
}
