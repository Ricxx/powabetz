//! Scout strategy — picks derived DIRECTLY and DETERMINISTICALLY from the
//! statistics scraped out of your ingested web pages (corners, cards, shots per
//! game). This is deliberately INDEPENDENT of the main feature engine: it never
//! touches `features.rs`'s own numbers, so a 3rd-party site's figures can never
//! masquerade as our measured data. It only borrows the shared count-model
//! primitives (Negative-Binomial tails) — the same maths a bookmaker would use
//! on those rates. Every candidate it emits is flagged as ingest-sourced.

use crate::features::{nb_over, PHI_CARDS, PHI_CORNERS, PHI_SHOTS, PHI_P_SHOTS, PHI_P_SOT};
use crate::models::Candidate;
use serde_json::Value;

/// First floating-point number found in a string ("6.4 corners" → 6.4).
fn first_num(s: &str) -> Option<f64> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_digit() || (c == '.' && i + 1 < bytes.len() && (bytes[i + 1] as char).is_ascii_digit()) {
            let start = i;
            while i < bytes.len() {
                let d = bytes[i] as char;
                if d.is_ascii_digit() || d == '.' {
                    i += 1;
                } else {
                    break;
                }
            }
            if let Ok(v) = s[start..i].parse::<f64>() {
                return Some(v);
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Running mean accumulator — multiple ingested pages for one match get averaged.
#[derive(Default, Clone)]
struct Avg {
    sum: f64,
    n: u32,
}
impl Avg {
    fn add(&mut self, v: f64) {
        self.sum += v;
        self.n += 1;
    }
    fn get(&self) -> Option<f64> {
        if self.n > 0 {
            Some(self.sum / self.n as f64)
        } else {
            None
        }
    }
}

#[derive(Default, Clone)]
struct TeamStats {
    corners_for: Avg,
    corners_against: Avg,
    cards: Avg,
    shots: Avg,
}

/// Per-game player rates scraped from a page ("Hakimi shots/game 2.1").
#[derive(Default, Clone)]
struct PlayerRates {
    shots: Avg,
    sot: Avg,
    goals: Avg,
}

fn fold(s: &str) -> String {
    crate::odds::fold(s)
}

/// Parse the `data` array of every matched ingest into per-team stat means and
/// per-PLAYER per-game rates ("Hakimi shots/game 2.1" — a label that names
/// neither team but carries an explicit per-game rate).
fn parse(items: &[&Value], home: &str, away: &str) -> (TeamStats, TeamStats, std::collections::HashMap<String, PlayerRates>) {
    let mut h = TeamStats::default();
    let mut a = TeamStats::default();
    let mut players: std::collections::HashMap<String, PlayerRates> = std::collections::HashMap::new();
    for it in items {
        let data = match it.get("data").and_then(|d| d.as_array()) {
            Some(d) => d,
            None => continue,
        };
        for e in data {
            let label = e.get("label").and_then(|x| x.as_str()).unwrap_or("");
            let value = e.get("value").and_then(|x| x.as_str()).unwrap_or("");
            let lf = fold(label);
            let num = match first_num(value).or_else(|| first_num(label)) {
                Some(n) if n.is_finite() && n >= 0.0 && n < 60.0 => n,
                _ => continue,
            };
            // Whose stat? Token-aware team matching (page spellings vary).
            let team = if crate::odds::team_match(&lf, home) {
                Some(&mut h)
            } else if crate::odds::team_match(&lf, away) {
                Some(&mut a)
            } else {
                None
            };
            if let Some(team) = team {
                // Classify the stat (order matters: SOT before generic shot).
                if lf.contains("corner") {
                    if lf.contains("against") || lf.contains("conced") || lf.contains("faced") {
                        team.corners_against.add(num);
                    } else {
                        team.corners_for.add(num);
                    }
                } else if lf.contains("card") || lf.contains("booking") {
                    team.cards.add(num);
                } else if lf.contains("sot") || lf.contains("ontarget") || lf.contains("shotsontarget") {
                    // team SOT has no settle market — fold into nothing for now
                    continue;
                } else if lf.contains("shot") {
                    team.shots.add(num);
                }
                continue;
            }
            // PLAYER rate: names neither team AND is an explicit per-game rate
            // (totals are useless without appearance counts — skip those).
            let per_game = lf.contains("/game") || lf.contains("per game") || lf.contains("pergame");
            if !per_game {
                continue;
            }
            // Name = the label text before the stat keyword.
            let ll = label.to_lowercase();
            let kw = ["shots on target", "sot", "shots", "shot", "goals", "goal"]
                .iter()
                .filter_map(|k| ll.find(k).map(|p| (p, *k)))
                .min_by_key(|(p, _)| *p);
            let Some((pos, kw)) = kw else { continue };
            let name = label[..pos].trim().trim_end_matches(['-', ':', '—']).trim();
            let nl = fold(name);
            const NOT_NAMES: [&str; 8] = ["total", "team", "match", "both", "home", "away", "avg", "average"];
            if nl.len() < 3 || NOT_NAMES.iter().any(|w| nl == *w) {
                continue;
            }
            let p = players.entry(name.to_string()).or_default();
            match kw {
                "shots on target" | "sot" => {
                    if (0.2..=5.0).contains(&num) {
                        p.sot.add(num);
                    }
                }
                "shots" | "shot" => {
                    if (0.3..=7.0).contains(&num) {
                        p.shots.add(num);
                    }
                }
                _ => {
                    if (0.05..=1.2).contains(&num) {
                        p.goals.add(num);
                    }
                }
            }
        }
    }
    (h, a, players)
}

/// Validate a scraped per-game mean for a ONE-TEAM market: None unless it lies
/// inside [lo, hi]. Out-of-band values are SKIPPED, not clamped — a clamped
/// number is no longer "from ingested stats" (honest-data rule), and above
/// `skip_above` it's almost certainly a both-teams/match total mis-scraped onto
/// one team.
fn sane(mu: f64, lo: f64, hi: f64, skip_above: f64) -> Option<f64> {
    let _ = skip_above; // subsumed by the hi bound; kept for call-site clarity
    if mu.is_finite() && (lo..=hi).contains(&mu) {
        Some(mu)
    } else {
        None
    }
}

fn clampp(p: f64) -> f64 {
    p.clamp(0.02, 0.98)
}
fn r2(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// Build a team's O/U candidates for one count market from a scraped mean.
fn lines_for(
    out: &mut Vec<Candidate>,
    team: &str,
    opponent: &str,
    fixture_label: &str,
    fixture_id: i64,
    market: &str,
    group: &str,
    mu: f64,
    phi: f64,
    offsets: &[f64],
    floor: f64,
    src: &str,
) {
    let base = mu.round();
    for &off in offsets {
        let line = (base + off).max(floor);
        let thr = line.floor() as u32;
        let over = clampp(nb_over(thr, mu, phi));
        for (lbl, p) in [(format!("Over {line:.1}"), over), (format!("Under {line:.1}"), 1.0 - over)] {
            out.push(Candidate {
                subject: team.to_string(),
                subject_kind: "team".to_string(),
                team: team.to_string(),
                opponent: opponent.to_string(),
                fixture: fixture_label.to_string(),
                fixture_id,
                market: market.to_string(),
                market_group: group.to_string(),
                line: lbl,
                base_rate: r2(mu),
                est_prob: r2(clampp(p)),
                pinnacle_prob: None,
                book_odds: None,
                book: None,
                ev: None,
                ev_source: None,
                form_state: None,
                xg_source: None,
                support: vec![format!("{src} {mu:.1}/game"), "from ingested stats · Neg-Binomial".to_string()],
                flags: vec!["ingested stats (independent source)".to_string()],
                plausibility: None,
                raw_prob: None,
            });
        }
    }
}

/// Deterministic candidates for ONE fixture, derived purely from its matched
/// ingested pages. Returns empty if the pages carried no usable stats.
pub fn candidates_for_fixture(items: &[&Value], home: &str, away: &str, fixture_label: &str, fixture_id: i64) -> Vec<Candidate> {
    let (h, a, players) = parse(items, home, away);
    let mut out = Vec::new();
    // PLAYER candidates from page-scraped per-game rates — the pages' player
    // numbers were previously prompt-context only; now they price real lines
    // (same NB maths as the engine, clearly flagged as ingest-sourced).
    for (name, pr) in &players {
        let mk = |market: &str, group: &str, line: String, mu: f64, p: f64, src: &str| Candidate {
            subject: name.clone(),
            subject_kind: "player".to_string(),
            team: String::new(),
            opponent: String::new(),
            fixture: fixture_label.to_string(),
            fixture_id,
            market: market.to_string(),
            market_group: group.to_string(),
            line,
            base_rate: r2(mu),
            est_prob: r2(clampp(p)),
            pinnacle_prob: None,
            book_odds: None,
            book: None,
            ev: None,
            ev_source: None,
            form_state: None,
            xg_source: None,
            support: vec![format!("{src} {mu:.2}/game (page)")],
            flags: vec!["ingested stats (independent source)".to_string(), "team unknown (page-derived)".to_string()],
            plausibility: None,
            raw_prob: None,
        };
        if let Some(mu) = pr.shots.get() {
            let (line, p) = if nb_over(1, mu, PHI_P_SHOTS) >= 0.5 {
                ("2+ shots".to_string(), nb_over(1, mu, PHI_P_SHOTS))
            } else {
                ("1+ shot".to_string(), nb_over(0, mu, PHI_P_SHOTS))
            };
            out.push(mk("Player Shots", "pshots", line, mu, p, "shots"));
        }
        if let Some(mu) = pr.sot.get() {
            let (line, p) = if nb_over(1, mu, PHI_P_SOT) >= 0.5 {
                ("2+ shots on target".to_string(), nb_over(1, mu, PHI_P_SOT))
            } else {
                ("1+ shot on target".to_string(), nb_over(0, mu, PHI_P_SOT))
            };
            out.push(mk("Shots on Target", "sot", line, mu, p, "sot"));
        }
        if let Some(mu) = pr.goals.get() {
            let p = 1.0 - (-mu).exp();
            out.push(mk("Anytime Scorer", "scorer", "1+ goal".to_string(), mu, p, "goals"));
        }
    }
    for (team, opp, ts, opp_ts) in [(home, away, &h, &a), (away, home, &a, &h)] {
        // Corners — blend the team's "for" with the opponent's "against" when both
        // are present (a corner takes two teams), else use whichever we have.
        let corners = match (ts.corners_for.get(), opp_ts.corners_against.get()) {
            (Some(f), Some(ag)) => Some((f + ag) / 2.0),
            (Some(f), None) => Some(f),
            (None, Some(ag)) => Some(ag),
            (None, None) => None,
        };
        // Sanity bands per ONE-TEAM market: a number above the "total" cutoff is
        // almost certainly a both-teams/match figure mis-scraped onto one team
        // (e.g. "13 corners" is a match total, not Sweden's per-game) — SKIP it
        // rather than emit an impossible "Over 13.5" team line. Otherwise clamp the
        // mean into a realistic per-game range so the lines stay sane.
        if let Some(mu) = corners.and_then(|m| sane(m, 2.5, 8.5, 11.0)) {
            lines_for(&mut out, team, opp, fixture_label, fixture_id, "Team Corners", "tcorners", mu, PHI_CORNERS, &[-1.5, -0.5, 0.5], 2.5, "corners");
        }
        if let Some(mu) = ts.cards.get().and_then(|m| sane(m, 0.8, 3.5, 7.0)) {
            lines_for(&mut out, team, opp, fixture_label, fixture_id, "Team Total Cards", "tcards", mu, PHI_CARDS, &[-0.5, 0.5, 1.5], 0.5, "cards");
        }
        if let Some(mu) = ts.shots.get().and_then(|m| sane(m, 3.0, 18.0, 30.0)) {
            lines_for(&mut out, team, opp, fixture_label, fixture_id, "Team Shots", "tshots", mu, PHI_SHOTS, &[-2.5, -0.5, 1.5], 2.5, "shots");
        }
    }
    out
}
