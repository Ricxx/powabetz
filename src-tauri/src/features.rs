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

/// Overdispersion (variance ÷ mean) for football count markets. Cards, corners,
/// shots and offsides are OVERDISPERSED — their real variance exceeds the mean,
/// so a Poisson (which assumes var = mean) understates the tails (the high-count
/// games you bet "over" on). Negative-Binomial with these factors fixes that.
/// Values follow the empirical literature: corners ~mild, cards the most spread
/// (refs/fixtures vary a lot), shots moderate. (Karlis & Ntzoufras 2000 on count
/// models for football; corners/cards overdispersion is well documented.)
/// P(count A strictly exceeds count B) for two independent Poisson means — used
/// for "which team gets the most corners/shots" (an either-team 1x2-style market).
fn prob_more(la: f64, lb: f64) -> f64 {
    let mut p = 0.0;
    for b in 0..=40u32 {
        p += pois_pmf(b, lb) * (1.0 - pois_cdf(b, la));
    }
    p.clamp(0.0, 1.0)
}

pub const PHI_CORNERS: f64 = 1.18;
pub const PHI_CARDS: f64 = 1.30;
pub const PHI_SHOTS: f64 = 1.20;
const PHI_OFFSIDES: f64 = 1.15;
// Per-PLAYER count props are overdispersed too — a player's shots/tackles vary
// game to game with role, opponent and game state. Same NB treatment, slightly
// milder factors (single-player counts are lower).
pub const PHI_P_SHOTS: f64 = 1.20;
pub const PHI_P_SOT: f64 = 1.15;
const PHI_P_TACKLES: f64 = 1.20;
const PHI_P_FOULS: f64 = 1.15;
const PHI_P_SAVES: f64 = 1.20;

/// NB P(X≥1) and P(X≥2) for mean `mu`, dispersion `phi` — convenience wrappers.
fn nb_ge1(mu: f64, phi: f64) -> f64 {
    nb_over(0, mu, phi)
}
fn nb_ge2(mu: f64, phi: f64) -> f64 {
    nb_over(1, mu, phi)
}

/// Negative-Binomial P(X > k) for mean `mu` with overdispersion `phi` (var/mean).
/// Reduces to Poisson when phi ≈ 1. Parameterized by mean + dispersion:
/// r = mu/(phi−1), p = r/(r+mu); pmf via the standard NB recurrence.
pub fn nb_over(k: u32, mu: f64, phi: f64) -> f64 {
    if phi <= 1.0001 || mu <= 1e-6 {
        return (1.0 - pois_cdf(k, mu)).clamp(0.0, 1.0);
    }
    let r = mu / (phi - 1.0);
    let p = r / (r + mu);
    let mut term = p.powf(r); // pmf(0)
    let mut cdf = term;
    for i in 1..=k {
        term *= (r + i as f64 - 1.0) / i as f64 * (1.0 - p);
        cdf += term;
    }
    (1.0 - cdf).clamp(0.0, 1.0)
}

/// Dixon-Coles (1997) low-score dependence factor τ. With ρ<0 it lifts 0-0 and
/// 1-1 and trims 1-0 / 0-1 — correcting independent Poisson's well-known
/// under-weighting of draws and tight games. ρ is a fixed literature-typical
/// value (we can't fit it per match without more data), so this is a principled
/// CORRECTION, not a measured per-match quantity.
const DC_RHO: f64 = -0.08;
fn dc_tau(x: u32, y: u32, lh: f64, la: f64, rho: f64) -> f64 {
    match (x, y) {
        (0, 0) => 1.0 - lh * la * rho,
        (0, 1) => 1.0 + lh * rho,
        (1, 0) => 1.0 + la * rho,
        (1, 1) => 1.0 - rho,
        _ => 1.0,
    }
}
/// Home advantage as a multiplicative tilt on goal expectation. Home sides
/// reliably score more and concede less; classic estimates put the edge near
/// 0.3-0.4 goals (Pollard 1986; Dixon & Coles 1997 use a home factor γ≈1.35),
/// though it has eased since ~2020. We apply a CONSERVATIVE tilt because some of
/// the edge is already baked into overall scoring rates. Makes every goal-derived
/// market home-aware and consistent.
const HOME_ADV: f64 = 1.10;
const AWAY_ADJ: f64 = 0.95;
/// Typical per-team goals/game baseline that normalizes the Maher attack×defence
/// interaction (so an average attack vs an average defence returns the average).
const LEAGUE_AVG_GOALS: f64 = 1.35;
/// Per-team baselines for the volume stats — normalize the same attack×defence
/// crossing on shots/corners/offsides. (Top-league typical values; the per-league
/// team index refines these with the league's real averages when built.)
const LEAGUE_AVG_SHOTS: f64 = 12.5;
const LEAGUE_AVG_CORNERS: f64 = 5.0;
const LEAGUE_AVG_OFFSIDES: f64 = 2.0;

/// (home win, draw, away win) read straight off a score grid. The difference of
/// two independent Poissons is the Skellam distribution — summing the grid below
/// / on / above the diagonal gives exact 1X2 probabilities (and, with the DC τ
/// baked into the grid, the low-score correction comes along for free). This
/// replaces a hand-tuned sigmoid + a constant 27% draw.
fn wdl_from_matrix(m: &[Vec<f64>]) -> (f64, f64, f64) {
    let (mut ph, mut pd, mut pa) = (0.0, 0.0, 0.0);
    for (h, row) in m.iter().enumerate() {
        for (a, p) in row.iter().enumerate() {
            if h > a {
                ph += p;
            } else if h == a {
                pd += p;
            } else {
                pa += p;
            }
        }
    }
    (ph, pd, pa)
}
/// P(home goals − away goals ≥ k) from a score grid — for Asian handicap lines.
fn margin_ge_from_matrix(m: &[Vec<f64>], k: i64) -> f64 {
    let mut s = 0.0;
    for (h, row) in m.iter().enumerate() {
        for (a, p) in row.iter().enumerate() {
            if (h as i64 - a as i64) >= k {
                s += p;
            }
        }
    }
    s.clamp(0.0, 1.0)
}

/// DC-corrected, renormalized joint score matrix m[home][away] up to `max` each.
fn dc_score_matrix(lh: f64, la: f64, max: usize) -> Vec<Vec<f64>> {
    let mut m = vec![vec![0.0f64; max + 1]; max + 1];
    let mut sum = 0.0;
    for h in 0..=max {
        for a in 0..=max {
            let p = (dc_tau(h as u32, a as u32, lh, la, DC_RHO) * pois_pmf(h as u32, lh) * pois_pmf(a as u32, la)).max(0.0);
            m[h][a] = p;
            sum += p;
        }
    }
    if sum > 1e-12 {
        for row in m.iter_mut() {
            for v in row.iter_mut() {
                *v /= sum;
            }
        }
    }
    m
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
    /// Opponent's defensive index factors (league-relative; <1 = suppressive).
    /// Dampened (^0.7) onto player shot/SOT/goal rates when the index is on.
    pub opp_def_shots: Option<f64>,
    pub opp_def_goals: Option<f64>,
}

/// Per-player recent-match CONSISTENCY: how OFTEN (over recent appearances) a
/// player actually hit each line — the "does this happen most games" signal that
/// a season average misses. Rates are fraction-of-appearances in [0,1].
#[derive(Default, Clone)]
pub struct Consistency {
    pub apps: u32,
    pub card_rate: f64,
    pub shot1_rate: f64,
    pub shot2_rate: f64,
    pub sot1_rate: f64,
    pub sot2_rate: f64,
    pub goal_rate: f64,
    pub assist_rate: f64,
    pub tackle2_rate: f64,
    pub foul1_rate: f64,
}

/// Blend a season-average Poisson probability with the observed recent hit-rate.
/// With enough appearances we lean on what ACTUALLY happens; otherwise we keep
/// the model estimate. Returns (blended_prob, optional "N/M recent" note,
/// form_gap) — form_gap flags a RECENT rate far above the season-implied one:
/// the classic role-change signal (new position, new set-piece duty, teammate
/// injured) that books pricing off season averages are slow to reprice.
fn blend_consistency(poisson: f64, rate: f64, apps: u32) -> (f64, Option<String>, bool) {
    if apps < 3 {
        return (poisson, None, false);
    }
    // Shrinkage / empirical-Bayes: treat the season-average Poisson estimate as a
    // prior worth ~K pseudo-games, and weight the observed recent hit-rate by its
    // sample size. More appearances → trust what actually happened more. This is
    // the Beta-Binomial-style shrink that beats either pure estimate alone.
    const K: f64 = 4.0;
    let w = apps as f64 / (apps as f64 + K);
    let blended = (1.0 - w) * poisson + w * rate;
    let hits = (rate * apps as f64).round() as u32;
    let gap = apps >= 4 && rate - poisson >= 0.20;
    (clampp(blended), Some(format!("hit {hits}/{apps} recent")), gap)
}

// ---------- position-group baselines (empirical-Bayes prior) ----------

/// 0=GK, 1=DEF, 2=MID, 3=FWD — used to shrink small-sample player rates toward
/// the right peer group (a striker's tackle rate should regress toward strikers,
/// not toward the whole squad).
fn pos_group(p: &str) -> usize {
    let p = p.to_lowercase();
    if p.contains("goalkeeper") || p == "g" {
        0
    } else if p.contains("defend") || p == "d" {
        1
    } else if p.contains("midfield") || p == "m" {
        2
    } else {
        3
    }
}

/// Minutes-weighted per-90 baseline per position group for the props that have
/// no recent-hit-rate consistency signal. baseline[stat][group], stat order:
/// 0 tackles, 1 fouls, 2 assists, 3 saves.
pub type PosBaseline = [[f64; 4]; 4];

/// Pseudo-games of prior weight for the squad-baseline shrink (James-Stein style).
const SHRINK_K: f64 = 3.0;

pub fn squad_baselines(entries: &[Value], league_id: i64) -> PosBaseline {
    let mut sums: PosBaseline = [[0.0; 4]; 4];
    let mut nineties = [0.0f64; 4];
    for e in entries {
        if let Some(s) = parse_season_entry(e, league_id) {
            if s.minutes < 90.0 {
                continue; // need ~a full game before a player informs the baseline
            }
            let g = pos_group(&s.position);
            sums[0][g] += s.tackles;
            sums[1][g] += s.fouls_for;
            sums[2][g] += s.assists;
            sums[3][g] += s.saves;
            nineties[g] += s.minutes / 90.0;
        }
    }
    let mut out: PosBaseline = [[0.0; 4]; 4];
    for stat in 0..4 {
        for g in 0..4 {
            out[stat][g] = if nineties[g] > 0.0 { sums[stat][g] / nineties[g] } else { 0.0 };
        }
    }
    out
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
    consistency: &std::collections::HashMap<String, Consistency>,
    baselines: &PosBaseline,
) -> Vec<Candidate> {
    let s = match parse_season_entry(entry, league_id) {
        Some(s) => s,
        None => return Vec::new(),
    };
    let hot = in_form.contains(&crate::odds::fold(&s.name));
    let con = consistency.get(&crate::odds::fold(&s.name)).cloned().unwrap_or_default();
    let mut out = Vec::new();

    let gp = s.apps.max(1.0);
    let exp_min = (s.minutes / gp).clamp(0.0, 90.0);
    if s.minutes <= 0.0 {
        return out; // no minutes this season → nothing to rate on
    }
    let nineties = s.minutes / 90.0;
    let per90 = |total: f64| if nineties > 0.0 { total / nineties } else { 0.0 };

    // Empirical-Bayes shrink toward the player's position-group baseline, weighted
    // by sample size: few games → lean on peers; many games → trust the player.
    // Applied to the props with NO recent-hit-rate signal (tackles/fouls/assists/
    // saves); scorer/shots/SOT/cards already shrink via the consistency blend.
    let g = pos_group(&s.position);
    let shrink = |rate: f64, prior: f64| -> f64 {
        if prior <= 0.0 {
            return rate;
        }
        (s.apps * rate + SHRINK_K * prior) / (s.apps + SHRINK_K)
    };

    // Opponent-index suppression: a player's shot/goal volume vs THIS opponent
    // scales with what the defence concedes (dampened — game state still lets
    // shots happen). Only active when the league index is built + enabled.
    let sup_shots = ctx.opp_def_shots.map(|f| f.clamp(0.6, 1.5).powf(0.7)).unwrap_or(1.0);
    let sup_goals = ctx.opp_def_goals.map(|f| f.clamp(0.6, 1.5).powf(0.7)).unwrap_or(1.0);
    let goals_p90 = per90(s.goals) * sup_goals;
    let shots_p90 = per90(s.shots) * sup_shots;
    let sot_p90 = per90(s.sot) * sup_shots;
    let tackles_p90 = shrink(per90(s.tackles), baselines[0][g]);
    let fouls_p90 = shrink(per90(s.fouls_for), baselines[1][g]);
    let fdrawn_p90 = per90(s.fouls_drawn);
    let cards_p90 = per90(s.cards);
    let passes_p90 = per90(s.passes);
    let assists_p90 = shrink(per90(s.assists), baselines[2][g]);
    let saves_p90 = shrink(per90(s.saves), baselines[3][g]);
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
            raw_prob: None,
        }
    };

    for g in groups {
        match g.as_str() {
            "scorer" => {
                // xG only enters here, as one optional input.
                let proxy_xg_p90 = sot_p90 * 0.30 + (shots_p90 - sot_p90).max(0.0) * 0.05;
                // BLEND actual scoring rate with the SOT-implied rate. The old
                // `goals_p90.max(0.18*sot)` was one-directional — it could only
                // RAISE the estimate, systematically inflating every scorer prob
                // (and, via the model-EV fallback, manufacturing fake +EV). A
                // weighted blend regresses noisy finishers both ways.
                let lambda = (0.7 * goals_p90 + 0.3 * (0.18 * sot_p90)) * min_scale;
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
                let (est_b, gnote, gap) = blend_consistency(est, con.goal_rate, con.apps);
                est = est_b;
                if gap {
                    flags.push("form-gap: scoring well above season rate — book may lag".to_string());
                }
                let mut sup = vec![
                    format!("goals/90 {:.2}", goals_p90),
                    format!("sot/90 {:.2}", sot_p90),
                    format!("proxy_xg/90 {:.2}", proxy_xg_p90),
                    format!("exp_min {:.0}", exp_min),
                ];
                if let Some(n) = gnote {
                    sup.push(n);
                }
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
                    flags.push("in-form (league top scorer/assister)".to_string());
                }
                let mut c = base("Anytime Scorer", "scorer", "1+ goal", goals_p90, est, sup, flags);
                c.form_state = Some(form);
                c.xg_source = Some("proxy".to_string());
                out.push(c);
                // MULTI SCORER (2+ goals) — the "brace" leg for stat-led stacks.
                // Poisson on the per-match goal expectation; only emitted for
                // genuine volume scorers so the table isn't flooded with junk.
                let lam_g = goals_p90 * min_scale;
                let p2 = (1.0 - (-lam_g).exp() * (1.0 + lam_g)) * avail_mult;
                if p2 >= 0.05 {
                    let mut c2 = base(
                        "Multi Scorer (2+)",
                        "scorer",
                        "2+ goals",
                        goals_p90,
                        clampp(p2),
                        vec![format!("goals/90 {goals_p90:.2}"), format!("exp_min {exp_min:.0}"), "Poisson 2+".to_string()],
                        vec![],
                    );
                    c2.xg_source = Some("proxy".to_string());
                    out.push(c2);
                }
            }
            "sot" => {
                let lambda = sot_p90 * min_scale;
                let (line, base_est, rate) = if nb_ge2(lambda, PHI_P_SOT) >= 0.5 {
                    ("2+ shots on target", nb_ge2(lambda, PHI_P_SOT), con.sot2_rate)
                } else {
                    ("1+ shot on target", nb_ge1(lambda, PHI_P_SOT), con.sot1_rate)
                };
                let (est, note, gap) = blend_consistency(base_est, rate, con.apps);
                let mut sup = vec![format!("sot/90 {:.2}", sot_p90), format!("exp_min {:.0}", exp_min)];
                if let Some(n) = note {
                    sup.push(n);
                }
                let fl = if gap { vec!["form-gap: recent SOT rate well above season — book may lag".to_string()] } else { vec![] };
                out.push(base("Shots on Target", "sot", line, sot_p90, est, sup, fl));
            }
            "tackles" => {
                let lambda = tackles_p90 * workload * min_scale;
                let (line, est) = if nb_ge2(lambda, PHI_P_TACKLES) >= 0.45 {
                    ("2+ tackles", nb_ge2(lambda, PHI_P_TACKLES))
                } else {
                    ("1+ tackle", nb_ge1(lambda, PHI_P_TACKLES))
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
                    vec!["workload proxy".to_string()],
                ));
            }
            "fouls" => {
                let lf = fouls_p90 * workload * min_scale;
                let (cline, cest) = if nb_ge2(lf, PHI_P_FOULS) >= 0.4 {
                    ("2+ fouls committed", nb_ge2(lf, PHI_P_FOULS))
                } else {
                    ("1+ foul committed", nb_ge1(lf, PHI_P_FOULS))
                };
                out.push(base(
                    "Fouls Committed",
                    "fouls",
                    cline,
                    fouls_p90,
                    cest,
                    vec![format!("fouls/90 {:.2}", fouls_p90)],
                    vec!["workload proxy".to_string()],
                ));
            }
            "fdrawn" => {
                // Fouls Drawn — its own toggle (bet365 "Player To Be Fouled");
                // used to hide inside the committed-fouls key.
                let ld = fdrawn_p90 * min_scale;
                if fdrawn_p90 >= 0.3 {
                    out.push(base(
                        "Fouls Drawn",
                        "fdrawn",
                        "1+ foul drawn",
                        fdrawn_p90,
                        pois_ge1(ld),
                        vec![format!("fouls_drawn/90 {:.2}", fdrawn_p90)],
                        vec![],
                    ));
                }
            }
            "cards" => {
                let lambda = cards_p90 * workload * min_scale;
                let (est, note, _gap) = blend_consistency(pois_ge1(lambda), con.card_rate, con.apps);
                let mut sup = vec![format!("cards/90 {:.2}", cards_p90)];
                let mut flags = vec![];
                match note {
                    Some(n) => {
                        sup.push(n);
                        if con.card_rate >= 0.5 && con.apps >= 3 {
                            flags.push("consistent booking — carded most recent games".to_string());
                        }
                    }
                    None => flags.push("card rate is noisy season-wide".to_string()),
                }
                out.push(base("To Be Carded", "cards", "1+ card", cards_p90, est, sup, flags));
            }
            "passes" => {
                let expected = passes_p90 * min_scale;
                if expected < 8.0 {
                    continue; // not a passing role — skip rather than offer a junk line
                }
                // Pass counts are FAR overdispersed vs Poisson (role, game state,
                // scoreline chasing): var ≈ φ·mean with φ well above 1. The old
                // sd = sqrt(mean) made every self-derived line a ~72% "banker" by
                // construction. Use an honest spread so est_prob reflects reality.
                const PHI_P_PASSES: f64 = 6.0;
                let sd = (expected * PHI_P_PASSES).sqrt().max(1.0);
                let line_val = ((expected - 0.75 * sd) / 5.0).floor() * 5.0;
                let line_val = line_val.max(10.0);
                let z = (expected - line_val + 0.5) / sd;
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
                let (line, est) = if nb_over(2, lambda, PHI_P_SAVES) >= 0.5 {
                    ("3+ saves", nb_over(2, lambda, PHI_P_SAVES))
                } else {
                    ("2+ saves", nb_over(1, lambda, PHI_P_SAVES))
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
            "goalassist" => {
                // Combined involvement: P(≥1 goal OR assist) from the summed
                // rate — the bet365 "Player to Score or Assist" group.
                let ga90 = goals_p90 + assists_p90;
                if ga90 < 0.10 {
                    continue;
                }
                let lambda = ga90 * min_scale;
                let est = clampp(pois_ge1(lambda) * avail_mult);
                let af = if hot { vec!["in-form (league top scorer/assister)".to_string()] } else { vec![] };
                let mut c = base(
                    "To Score or Assist",
                    "goalassist",
                    "1+ goal or assist",
                    ga90,
                    est,
                    vec![format!("g+a/90 {ga90:.2}"), format!("exp_min {exp_min:.0}")],
                    af,
                );
                c.xg_source = Some("proxy".to_string());
                out.push(c);
            }
            "assists" => {
                let lambda = assists_p90 * min_scale;
                if assists_p90 < 0.05 {
                    continue;
                }
                let af = if hot {
                    vec!["in-form (league top scorer/assister)".to_string()]
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
                let (line, base_est, rate) = if nb_ge2(lambda, PHI_P_SHOTS) >= 0.5 {
                    ("2+ shots", nb_ge2(lambda, PHI_P_SHOTS), con.shot2_rate)
                } else {
                    ("1+ shot", nb_ge1(lambda, PHI_P_SHOTS), con.shot1_rate)
                };
                let (est, note, gap) = blend_consistency(base_est, rate, con.apps);
                let mut sup = vec![format!("shots/90 {:.2}", shots_p90), format!("exp_min {:.0}", exp_min)];
                if let Some(n) = note {
                    sup.push(n);
                }
                let fl = if gap { vec!["form-gap: recent shot volume well above season — book may lag".to_string()] } else { vec![] };
                out.push(base(
                    "Player Shots",
                    "pshots",
                    line,
                    shots_p90,
                    est,
                    sup,
                    fl,
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
    pub sot_for: Option<f64>,
    pub sot_against: Option<f64>,
    /// Recent card model: avg cards this team / the opponent per game, the rate
    /// BOTH teams got a card, and the rate this team had the MOST cards.
    pub cards_for: Option<f64>,
    pub cards_against: Option<f64>,
    pub both_card_rate: Option<f64>,
    pub most_card_rate: Option<f64>,
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
        sot_for: None,
        sot_against: None,
        offsides_for: None,
        cards_for: None,
        cards_against: None,
        both_card_rate: None,
        most_card_rate: None,
        corners_against: None,
        shots_for: None,
        shots_against: None,
        outbox_for: None,
        inbox_for: None,
    })
}

/// Team/match-line candidates. `groups` are the selected team-market keys.
/// Per-match expectations for both sides — goals λ plus the opponent-crossed
/// volume stats. One set of formulas shared by the candidate engine and the
/// team-performance prediction ledger (predicted-vs-actual audits).
pub struct MatchExp {
    pub home_goals: f64,
    pub away_goals: f64,
    pub home_shots: Option<f64>,
    pub away_shots: Option<f64>,
    pub home_corners: Option<f64>,
    pub away_corners: Option<f64>,
}

pub fn match_expectations(home: &TeamStats, away: &TeamStats) -> MatchExp {
    // Use real xG (recent fixtures) where both teams have it — averaging a team's
    // attack xG with the opponent's defensive xG-conceded — else season goal rates.
    let h_for = home.xg_for.unwrap_or(home.gf_avg);
    let h_against = home.xg_against.unwrap_or(home.ga_avg);
    let a_for = away.xg_for.unwrap_or(away.gf_avg);
    let a_against = away.xg_against.unwrap_or(away.ga_avg);
    // Maher (1982) attack×defence interaction: a side's goal expectation is its
    // attack rate times the opponent's conceded rate, normalized by a league
    // baseline — so a strong attack against a leaky defence COMPOUNDS instead of
    // averaging out. We damp 50/50 toward the plain mean to avoid over-
    // extrapolating extreme mismatches from small samples.
    let maher = |atk: f64, opp_conceded: f64| -> f64 {
        let m = atk * opp_conceded / LEAGUE_AVG_GOALS;
        0.5 * m + 0.5 * (atk + opp_conceded) / 2.0
    };
    // Same crossing for the VOLUME stats: a team's shots expectation vs THIS
    // opponent is its own rate crossed with what the opponent concedes (a 13
    // shots/g attack means little against a block conceding 8). 50/50 damped
    // like goals; falls back to the raw rate when the opponent side is unknown.
    let cross = |own: Option<f64>, opp_conceded: Option<f64>, avg: f64| -> Option<f64> {
        let own = own?;
        Some(match opp_conceded {
            Some(oc) => 0.5 * (own * oc / avg) + 0.5 * (own + oc) / 2.0,
            None => own,
        })
    };
    MatchExp {
        home_goals: (maher(h_for, a_against) * HOME_ADV).max(0.05),
        away_goals: (maher(a_for, h_against) * AWAY_ADJ).max(0.05),
        home_shots: cross(home.shots_for, away.shots_against, LEAGUE_AVG_SHOTS),
        away_shots: cross(away.shots_for, home.shots_against, LEAGUE_AVG_SHOTS),
        home_corners: cross(home.corners_for, away.corners_against, LEAGUE_AVG_CORNERS),
        away_corners: cross(away.corners_for, home.corners_against, LEAGUE_AVG_CORNERS),
    }
}

pub fn build_team_candidates(
    home: &TeamStats,
    away: &TeamStats,
    fixture_label: &str,
    fixture_id: i64,
    groups: &[String],
) -> Vec<Candidate> {
    let mut out = Vec::new();
    let exp = match_expectations(home, away);
    let (lambda_home, lambda_away) = (exp.home_goals, exp.away_goals);
    let (h_shots, a_shots) = (exp.home_shots, exp.away_shots);
    let (h_corners, a_corners) = (exp.home_corners, exp.away_corners);
    let opp_adj_shots = home.shots_against.is_some() || away.shots_against.is_some();
    let opp_adj_corners = home.corners_against.is_some() || away.corners_against.is_some();
    let _ = LEAGUE_AVG_OFFSIDES; // offsides-against isn't threaded yet — raw rate stands
    let xg_used = home.xg_for.is_some() && away.xg_for.is_some();
    let crude = if xg_used {
        "xG-based (recent form)".to_string()
    } else {
        "season-rate proxy".to_string()
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
        raw_prob: None,
    };

    // Dixon-Coles-corrected joint score matrix, shared by every scoreline-derived
    // market (BTTS / totals / correct score / goals range) so they're mutually
    // consistent and account for low-score dependence.
    let dc = dc_score_matrix(lambda_home, lambda_away, 8);
    let p_total_le = |k: i64| -> f64 {
        let mut s = 0.0;
        for (h, row) in dc.iter().enumerate() {
            for (a, p) in row.iter().enumerate() {
                if (h + a) as i64 <= k {
                    s += p;
                }
            }
        }
        s.clamp(0.0, 1.0)
    };
    let dc_note = format!("Dixon-Coles corrected (ρ {DC_RHO:+.2})");

    for g in groups {
        match g.as_str() {
            "btts" => {
                let mut btts_yes = 0.0;
                for row in dc.iter().skip(1) {
                    for p in row.iter().skip(1) {
                        btts_yes += p;
                    }
                }
                btts_yes = btts_yes.clamp(0.0, 1.0);
                out.push(mk(
                    "Both Teams",
                    "BTTS",
                    "btts",
                    "Yes",
                    btts_yes * 100.0,
                    btts_yes,
                    vec![format!("xg_home {:.2}", lambda_home), format!("xg_away {:.2}", lambda_away), dc_note.clone()],
                ));
            }
            "ou25" => {
                let total = lambda_home + lambda_away;
                // Both sides of each goal line so the user can isolate over OR under.
                for thresh in [1i64, 2, 3] {
                    let line_val = thresh as f64 + 0.5;
                    let over = (1.0 - p_total_le(thresh)).clamp(0.0, 1.0);
                    let sup = vec![format!("exp_goals {:.2}", total), dc_note.clone()];
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
                // Most-likely correct scores from the DC-corrected score grid.
                let mut scores: Vec<((usize, usize), f64)> = Vec::new();
                for h in 0..=5usize {
                    for a in 0..=5usize {
                        scores.push(((h, a), dc[h][a]));
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
                        vec![format!("xg {lambda_home:.2}-{lambda_away:.2}"), dc_note.clone()],
                    ));
                }
            }
            "goalsrange" => {
                let total = lambda_home + lambda_away;
                let sup = vec![format!("exp_goals {total:.2}"), dc_note.clone()];
                // Bet365-style total-goals ranges (inclusive), priced EXACTLY from
                // the DC score grid: tight bands pay more, wide bands are safer.
                // Covers the common book lines (e.g. 2-4, 1-6) so they can anchor an
                // SGP with player props.
                // NOTE: no ultra-wide bands (e.g. 1-6 lands in ~90% of games —
                // a "prediction" with no information; the trivial-leg filter
                // would drop it anyway, so don't even price it).
                for (lo, hi) in [(0i64, 1i64), (1, 2), (2, 3), (3, 4), (1, 3), (2, 4), (3, 5), (1, 4), (2, 5)] {
                    let p = (p_total_le(hi) - p_total_le(lo - 1)).clamp(0.0, 1.0);
                    out.push(mk("Match", "Goals Range", "goalsrange", &format!("{lo}-{hi} goals"), total, clampp(p), sup.clone()));
                }
            }
            "firstscore" => {
                // First team to score = a race between two Poisson processes:
                // P(side first) = its rate share × P(any goal); plus a "No goal" out.
                let p_no = dc[0][0]; // DC-corrected 0-0 (match ends goalless)
                let denom = (lambda_home + lambda_away).max(1e-6);
                let p_home = (lambda_home / denom * (1.0 - p_no)).clamp(0.0, 1.0);
                let p_away = (lambda_away / denom * (1.0 - p_no)).clamp(0.0, 1.0);
                let sup = vec![format!("xg {lambda_home:.2} vs {lambda_away:.2}"), "competing-Poisson race".into()];
                out.push(mk(&home.name, "First Team to Score", "firstscore", "first to score", p_home * 100.0, clampp(p_home), sup.clone()));
                out.push(mk(&away.name, "First Team to Score", "firstscore", "first to score", p_away * 100.0, clampp(p_away), sup.clone()));
                out.push(mk("No goal", "First Team to Score", "firstscore", "no goal", p_no * 100.0, clampp(p_no), sup));
            }
            "tcorners" => {
                for (team, cf) in [(&home.name, h_corners), (&away.name, a_corners)] {
                    if let Some(lam) = cf {
                        for line in [2.5_f64, 3.5, 4.5, 5.5, 6.5, 7.5] {
                            let thr = line.floor() as u32;
                            let over = nb_over(thr, lam, PHI_CORNERS);
                            let sup = vec![
                                if opp_adj_corners { format!("corners/g {lam:.1} (opp-adjusted)") } else { format!("corners/g {lam:.1}") },
                                "Neg-Binomial".into(),
                            ];
                            out.push(mk(team, "Team Corners", "tcorners", &format!("Over {line:.1}"), lam, over, sup.clone()));
                            out.push(mk(team, "Team Corners", "tcorners", &format!("Under {line:.1}"), lam, 1.0 - over, sup));
                        }
                    }
                }
            }
            "tcards" => {
                // Team total cards O/U, from recent per-game card counts.
                for (team, cf) in [(&home.name, home.cards_for), (&away.name, away.cards_for)] {
                    if let Some(lam) = cf {
                        for line in [1.5_f64, 2.5] {
                            let thr = line.floor() as u32;
                            let over = nb_over(thr, lam, PHI_CARDS);
                            let sup = vec![format!("cards/g {lam:.1}"), "Neg-Binomial".into()];
                            out.push(mk(team, "Team Total Cards", "tcards", &format!("Over {line:.1}"), lam, over, sup.clone()));
                            out.push(mk(team, "Team Total Cards", "tcards", &format!("Under {line:.1}"), lam, 1.0 - over, sup));
                        }
                    }
                }
            }
            "bothcards" => {
                // Both teams to receive a card. Prefer the empirical recent rate;
                // fall back to independent Poisson on each side's card rate.
                let rate = home.both_card_rate.or(away.both_card_rate).or_else(|| {
                    match (home.cards_for, away.cards_for) {
                        (Some(h), Some(a)) => Some(pois_ge1(h) * pois_ge1(a)),
                        _ => None,
                    }
                });
                if let Some(p) = rate {
                    out.push(mk("Both Teams", "Both Teams Carded", "bothcards", "Yes", p, clampp(p), vec!["recent card rate".to_string()]));
                }
            }
            "mostcards" => {
                // Which team picks up the most cards (cards 1x2).
                for (team, mr) in [(&home.name, home.most_card_rate), (&away.name, away.most_card_rate)] {
                    if let Some(p) = mr {
                        out.push(mk(team, "Most Cards", "mostcards", "most cards", p, clampp(p), vec!["recent card edge".to_string()]));
                    }
                }
            }
            "mostcorners" => {
                // Which team takes the most corners (either-team market).
                if let (Some(hc), Some(ac)) = (h_corners, a_corners) {
                    let (ph, pa) = (prob_more(hc, ac), prob_more(ac, hc));
                    out.push(mk(&home.name, "Most Corners", "mostcorners", "most corners", ph, clampp(ph), vec![format!("corners/g {hc:.1} vs {ac:.1}")]));
                    out.push(mk(&away.name, "Most Corners", "mostcorners", "most corners", pa, clampp(pa), vec![format!("corners/g {ac:.1} vs {hc:.1}")]));
                }
            }
            "mcorners" => {
                // MATCH total corners (both teams combined) — bet365's "Corners".
                if let (Some(hc), Some(ac)) = (h_corners, a_corners) {
                    let lam = hc + ac;
                    for line in [7.5_f64, 8.5, 9.5, 10.5, 11.5] {
                        let thr = line.floor() as u32;
                        let over = nb_over(thr, lam, PHI_CORNERS);
                        let sup = vec![format!("match corners exp {lam:.1}"), "Neg-Binomial".into()];
                        out.push(mk("Match", "Match Corners", "mcorners", &format!("Over {line:.1}"), lam, over, sup.clone()));
                        out.push(mk("Match", "Match Corners", "mcorners", &format!("Under {line:.1}"), lam, 1.0 - over, sup));
                    }
                }
            }
            "mcards" => {
                // MATCH total cards (both teams) — bet365's "Cards".
                if let (Some(hc), Some(ac)) = (home.cards_for, away.cards_for) {
                    let lam = hc + ac;
                    for line in [3.5_f64, 4.5, 5.5] {
                        let thr = line.floor() as u32;
                        let over = nb_over(thr, lam, PHI_CARDS);
                        let sup = vec![format!("match cards exp {lam:.1}"), "Neg-Binomial".into()];
                        out.push(mk("Match", "Match Cards", "mcards", &format!("Over {line:.1}"), lam, over, sup.clone()));
                        out.push(mk("Match", "Match Cards", "mcards", &format!("Under {line:.1}"), lam, 1.0 - over, sup));
                    }
                }
            }
            "mshots" => {
                // MATCH total shots (both teams) — bet365's "Total Shots".
                if let (Some(hs), Some(as_)) = (h_shots, a_shots) {
                    let lam = hs + as_;
                    let base_l = (lam / 2.0).round() * 2.0; // even anchor near the mean
                    for off in [-3.5_f64, -0.5, 2.5] {
                        let line = (base_l + off).max(14.5);
                        let thr = line.floor() as u32;
                        let over = nb_over(thr, lam, PHI_SHOTS);
                        let sup = vec![format!("match shots exp {lam:.1}"), "Neg-Binomial".into()];
                        out.push(mk("Match", "Match Shots", "mshots", &format!("Over {line:.1}"), lam, over, sup.clone()));
                        out.push(mk("Match", "Match Shots", "mshots", &format!("Under {line:.1}"), lam, 1.0 - over, sup));
                    }
                }
            }
            "msot" => {
                // MATCH total shots on target — bet365's "Total Shots on Target".
                // Recent-form SOT for/against, crossed like the other volumes.
                let cross_sot = |own: Option<f64>, oc: Option<f64>| -> Option<f64> {
                    let o = own?;
                    Some(match oc {
                        Some(c) => 0.5 * (o * c / 4.5) + 0.5 * (o + c) / 2.0,
                        None => o,
                    })
                };
                let h_sot = cross_sot(home.sot_for, away.sot_against);
                let a_sot = cross_sot(away.sot_for, home.sot_against);
                if let (Some(hs), Some(as_)) = (h_sot, a_sot) {
                    let lam = hs + as_;
                    for line in [6.5_f64, 7.5, 8.5, 9.5] {
                        let thr = line.floor() as u32;
                        let over = nb_over(thr, lam, PHI_SHOTS);
                        let sup = vec![format!("match SOT exp {lam:.1}"), "Neg-Binomial".into()];
                        out.push(mk("Match", "Match Shots on Target", "msot", &format!("Over {line:.1}"), lam, over, sup.clone()));
                        out.push(mk("Match", "Match Shots on Target", "msot", &format!("Under {line:.1}"), lam, 1.0 - over, sup));
                    }
                }
            }
            "mostshots" => {
                // Which team has the most shots (either-team market).
                if let (Some(hs), Some(as_)) = (h_shots, a_shots) {
                    let (ph, pa) = (prob_more(hs, as_), prob_more(as_, hs));
                    out.push(mk(&home.name, "Most Shots", "mostshots", "most shots", ph, clampp(ph), vec![format!("shots/g {hs:.1} vs {as_:.1}")]));
                    out.push(mk(&away.name, "Most Shots", "mostshots", "most shots", pa, clampp(pa), vec![format!("shots/g {as_:.1} vs {hs:.1}")]));
                }
            }
            "toffsides" => {
                for (team, of) in [(&home.name, home.offsides_for), (&away.name, away.offsides_for)] {
                    if let Some(lam) = of {
                        for line in [0.5_f64, 1.5, 2.5] {
                            let thr = line.floor() as u32;
                            let over = nb_over(thr, lam, PHI_OFFSIDES);
                            let sup = vec![format!("offsides/g {lam:.1}"), "Neg-Binomial".into()];
                            out.push(mk(team, "Team Offsides", "toffsides", &format!("Over {line:.1}"), lam, over, sup.clone()));
                            out.push(mk(team, "Team Offsides", "toffsides", &format!("Under {line:.1}"), lam, 1.0 - over, sup));
                        }
                    }
                }
            }
            "tshots" => {
                for (team, sf) in [(&home.name, h_shots), (&away.name, a_shots)] {
                    if let Some(lam) = sf {
                        let base = lam.round();
                        for off in [-2.5_f64, -0.5, 1.5] {
                            let line = (base + off).max(2.5);
                            let thr = line.floor() as u32;
                            let over = nb_over(thr, lam, PHI_SHOTS);
                            let sup = vec![
                                if opp_adj_shots { format!("shots/g {lam:.1} (opp-adjusted)") } else { format!("shots/g {lam:.1}") },
                                "Neg-Binomial".into(),
                            ];
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
                // Exact 1X2 from the Skellam difference of the two Poissons.
                let (ph, pd, pa) = wdl_from_matrix(&dc);
                let sup = vec![format!("xg {:.2} vs {:.2}", lambda_home, lambda_away), format!("draw {:.0}%", pd * 100.0), "Skellam (DC grid)".into()];
                out.push(mk(&home.name, "Match Result", "win", "to win", ph * 100.0, ph, sup.clone()));
                out.push(mk(&away.name, "Match Result", "win", "to win", pa * 100.0, pa, sup));
            }
            "dc" => {
                let (ph, pd, pa) = wdl_from_matrix(&dc);
                let sup = vec![format!("xg {:.2} vs {:.2}", lambda_home, lambda_away), "Skellam (DC grid)".into()];
                out.push(mk(&format!("{} or draw", home.name), "Double Chance", "dc", "1X", (ph + pd) * 100.0, ph + pd, sup.clone()));
                out.push(mk(&format!("{} or draw", away.name), "Double Chance", "dc", "X2", (pa + pd) * 100.0, pa + pd, sup));
            }
            "half1" => {
                let lh = lambda_home * home.first_half_share;
                let la = lambda_away * away.first_half_share;
                let (ph, _pd, pa) = wdl_from_matrix(&dc_score_matrix(lh, la, 6));
                let (team, line, est) = if ph >= pa { (home.name.as_str(), "to win 1st half", ph) } else { (away.name.as_str(), "to win 1st half", pa) };
                out.push(mk(team, "Win 1st Half", "half1", line, est * 100.0, est, vec![format!("1H xg {:.2} vs {:.2}", lh, la), "Skellam".into()]));
            }
            "half2" => {
                let lh = lambda_home * (1.0 - home.first_half_share);
                let la = lambda_away * (1.0 - away.first_half_share);
                let (ph, _pd, pa) = wdl_from_matrix(&dc_score_matrix(lh, la, 6));
                let (team, line, est) = if ph >= pa { (home.name.as_str(), "to win 2nd half", ph) } else { (away.name.as_str(), "to win 2nd half", pa) };
                out.push(mk(team, "Win 2nd Half", "half2", line, est * 100.0, est, vec![format!("2H xg {:.2} vs {:.2}", lh, la), "Skellam".into()]));
            }
            "ahandicap" => {
                // Exact Asian-handicap lines from the goal-margin distribution.
                let sup = vec![format!("xg {:.2} vs {:.2}", lambda_home, lambda_away), "Skellam margin".into()];
                let home_2 = margin_ge_from_matrix(&dc, 2); // home wins by 2+
                let home_1 = margin_ge_from_matrix(&dc, 1); // home wins
                out.push(mk(&home.name, "Asian Handicap", "ahandicap", "-0.5 (to win)", home_1 * 100.0, clampp(home_1), sup.clone()));
                out.push(mk(&home.name, "Asian Handicap", "ahandicap", "-1.5 (win by 2+)", home_2 * 100.0, clampp(home_2), sup.clone()));
                out.push(mk(&away.name, "Asian Handicap", "ahandicap", "+0.5 (draw/win)", (1.0 - home_1) * 100.0, clampp(1.0 - home_1), sup.clone()));
                out.push(mk(&away.name, "Asian Handicap", "ahandicap", "+1.5 (lose by ≤1)", (1.0 - home_2) * 100.0, clampp(1.0 - home_2), sup));
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
            "h1goals" => Some(odds.get(&format!("h1ou|{ou_side}|{ou_num}"))),
            "h2goals" => Some(odds.get(&format!("h2ou|{ou_side}|{ou_num}"))),
            "tcards" => {
                let side = if is_home { "tcards_home" } else { "tcards_away" };
                Some(odds.get(&format!("{side}|{ou_side}|{ou_num}")))
            }
            "toffsides" => {
                let side = if is_home { "toffsides_home" } else { "toffsides_away" };
                Some(odds.get(&format!("{side}|{ou_side}|{ou_num}")))
            }
            "bothcards" => Some(odds.get("bothcards|yes")),
            "mostcards" => Some(odds.get(if is_home { "mostcards|home" } else { "mostcards|away" })),
            "saves" => Some(odds.prop("saves", &c.subject, thr)),
            "exactscore" => {
                let score: String = c.line.chars().filter(|ch| ch.is_ascii_digit() || *ch == '-').collect();
                Some(odds.get(&format!("exact|{score}")))
            }
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
    let has = |needle: &str| {
        c.flags.iter().any(|f| f.contains(needle)) || c.support.iter().any(|f| f.contains(needle))
    };
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

/// "Banker" score: the safest, most repeatable legs to anchor an accumulator.
/// Rewards high likelihood (sharp-blended), recurring event markets that happen
/// most games, observed recency/consistency ("hit X/Y recent"), and a sane price
/// band — and fades availability risk hard, because a banker has to actually play.
pub fn banker_score(c: &Candidate) -> f64 {
    let p = c.pinnacle_prob.map(|ps| 0.5 * ps + 0.5 * c.est_prob).unwrap_or(c.est_prob);
    let mut s = p; // likelihood is the core
    let recurring = matches!(
        c.market_group.as_str(),
        "cards" | "sot" | "pshots" | "fouls" | "tackles" | "passes" | "tcorners" | "tcards" | "saves" | "win" | "dc" | "ou25" | "btts"
    );
    if recurring {
        s += 0.05;
    }
    if c.support.iter().any(|x| x.starts_with("hit ")) {
        s += 0.12; // observed to happen most recent games → a trustworthy banker
    }
    if c.support.iter().any(|x| x.contains("consisten")) {
        s += 0.05;
    }
    if let Some(o) = c.book_odds {
        if o < 1.2 {
            s -= 0.15; // too short to matter
        } else if o > 2.4 {
            s -= 0.25; // not a banker
        } else if (1.3..=1.9).contains(&o) {
            s += 0.05; // the sweet banker band
        }
    }
    let has = |needle: &str| c.flags.iter().any(|f| f.contains(needle));
    if has("unlikely to feature") {
        s -= 0.6;
    }
    if has("minutes at risk") {
        s -= 0.1;
    }
    if matches!(c.form_state.as_deref(), Some("cold_falling_off")) {
        s -= 0.1;
    }
    s
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
/// `per_fixture_cap` (0 = off) bounds how many legs ONE fixture may take, so a
/// multi-match slate can't be dominated by a couple of data-rich games — the
/// fix for "10 fixtures starve each other in a fixed-size table".
pub fn shortlist(
    mut cands: Vec<Candidate>,
    n: usize,
    mode: &str,
    per_market_cap: usize,
    per_fixture_cap: usize,
) -> Vec<Candidate> {
    // Haiku plausibility (1-5) as a small ranking weight: +0.12 at 5, −0.12 at 1,
    // 0 at the neutral 3 (or when unscored). Re-ranks only — never a probability.
    let plaus = |c: &Candidate| -> f64 { c.plausibility.map(|p| (p as f64 - 3.0) * 0.06).unwrap_or(0.0) };
    let base = |c: &Candidate| match mode {
        "value" => c.est_prob + c.ev.unwrap_or(0.0).max(0.0) * 1.5,
        "oracle" => oracle_score(c),
        "power" => power_score(c),
        // APEX: proven-edge legs only. Heavy weight on sharp-backed +EV (the
        // top-down route), model↔market agreement as the trap filter, and a
        // favourite-longshot-bias guard band. Unpriced legs sink — Apex can't
        // verify an edge it can't price.
        "apex" => {
            let sharp = c.pinnacle_prob.is_some() && c.ev_source.as_deref() == Some("sharp");
            let ev = c.ev.unwrap_or(-0.15);
            let agree = match c.pinnacle_prob {
                Some(p) => 0.20 - ((p - c.est_prob).abs() * 2.0).min(0.40),
                None => -0.05,
            };
            let band = match c.book_odds {
                Some(o) if (1.4..=3.2).contains(&o) => 0.10,
                Some(o) if o > 3.6 => -0.20, // longshot bias — overpriced tail
                Some(_) => 0.0,
                None => -0.30,
            };
            c.est_prob * 0.3 + ev.clamp(-0.15, 0.5) * 2.0 + if sharp { 0.15 } else { 0.0 } + agree + band
        }
        // ONE definition of "banker": the same score that ranks the Bankers
        // board. (This arm used to be a diverged near-duplicate with different
        // weights, so the board and a Bankers build disagreed about the same leg.)
        "bankers" => banker_score(c),
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

    // STRATIFIED selection. A single-objective top-N silently purged whole
    // classes of opportunity before the model ever saw them (e.g. the plausible
    // ~30% longshots Jackpot needs rank last under pure likelihood). Reserve
    // slots across probability bands — bankers / mid / live longshots — and fill
    // each band in strategy-score order, so every strategy sees the full
    // spectrum while its own score still decides who represents each band.
    // Unfilled band quotas spill back to the global order, so thin slates lose
    // nothing vs the old behaviour.
    let bands: [(f64, f64, usize); 3] = [
        (0.60, 1.01, n / 2),          // bankers-grade
        (0.40, 0.60, (n * 3) / 10),   // solid-moderate
        (0.15, 0.40, n - n / 2 - (n * 3) / 10), // plausible longshots
    ];
    let mut per_market: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut per_fixture: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    let mut slots: Vec<Option<Candidate>> = cands.into_iter().map(Some).collect();
    let mut out: Vec<Candidate> = Vec::new();
    let mut try_take = |slot: &mut Option<Candidate>,
                        out: &mut Vec<Candidate>,
                        per_market: &mut std::collections::HashMap<String, usize>,
                        per_fixture: &mut std::collections::HashMap<i64, usize>|
     -> bool {
        let c = match slot.as_ref() {
            Some(c) => c,
            None => return false,
        };
        if *per_market.get(&c.market).unwrap_or(&0) >= per_market_cap {
            return false;
        }
        if per_fixture_cap > 0 && *per_fixture.get(&c.fixture_id).unwrap_or(&0) >= per_fixture_cap {
            return false;
        }
        *per_market.entry(c.market.clone()).or_insert(0) += 1;
        *per_fixture.entry(c.fixture_id).or_insert(0) += 1;
        out.push(slot.take().unwrap());
        true
    };
    // FIXTURE-PRESENCE reserve: the user SELECTED every fixture — one that ends
    // up with zero rows in the shortlist is useless to any strategy and makes
    // cover-all builds structurally impossible. Guarantee each fixture its top
    // 2 rows (by strategy score) before the bands compete for the rest.
    let mut fx_reserved: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    for slot in slots.iter_mut() {
        let fid = match slot.as_ref() {
            Some(c) => c.fixture_id,
            None => continue,
        };
        if *fx_reserved.get(&fid).unwrap_or(&0) >= 2 {
            continue;
        }
        if try_take(slot, &mut out, &mut per_market, &mut per_fixture) {
            *fx_reserved.entry(fid).or_insert(0) += 1;
        }
    }
    // CORRECT-SCORE rescue: score lines live at ~5-14% — below the lowest band —
    // so on busy slates they were squeezed out before the model ever saw them
    // and "predict the score" builds had nothing to work with. Reserve the top
    // 2 per fixture up front (they're already in score order).
    let mut cs_per_fix: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    for slot in slots.iter_mut() {
        let (is_cs, fid) = match slot.as_ref() {
            Some(c) => (c.market_group == "exactscore", c.fixture_id),
            None => continue,
        };
        if !is_cs || *cs_per_fix.get(&fid).unwrap_or(&0) >= 2 {
            continue;
        }
        if try_take(slot, &mut out, &mut per_market, &mut per_fixture) {
            *cs_per_fix.entry(fid).or_insert(0) += 1;
        }
    }
    for (lo, hi, quota) in bands {
        let mut got = 0usize;
        for slot in slots.iter_mut() {
            if got >= quota || out.len() >= n {
                break;
            }
            let p = match slot.as_ref() {
                Some(c) => c.est_prob,
                None => continue,
            };
            if p < lo || p >= hi {
                continue;
            }
            if try_take(slot, &mut out, &mut per_market, &mut per_fixture) {
                got += 1;
            }
        }
    }
    // Spill: fill any remaining slots by global score order (covers <0.15 tails
    // and bands with too few members).
    for slot in slots.iter_mut() {
        if out.len() >= n {
            break;
        }
        try_take(slot, &mut out, &mut per_market, &mut per_fixture);
    }
    // Keep the model's view ordered by strategy score, not by band.
    out.sort_by(|a, b| score(b).partial_cmp(&score(a)).unwrap_or(std::cmp::Ordering::Equal));
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
