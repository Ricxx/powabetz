//! Scout strategy — picks derived DIRECTLY and DETERMINISTICALLY from the
//! statistics scraped out of your ingested web pages (corners, cards, shots per
//! game). This is deliberately INDEPENDENT of the main feature engine: it never
//! touches `features.rs`'s own numbers, so a 3rd-party site's figures can never
//! masquerade as our measured data. It only borrows the shared count-model
//! primitives (Negative-Binomial tails) — the same maths a bookmaker would use
//! on those rates. Every candidate it emits is flagged as ingest-sourced.

use crate::features::{nb_over, PHI_CARDS, PHI_CORNERS, PHI_SHOTS};
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

fn fold(s: &str) -> String {
    crate::odds::fold(s)
}

/// Parse the `data` array of every matched ingest into per-team stat means.
fn parse(items: &[&Value], home: &str, away: &str) -> (TeamStats, TeamStats) {
    let (hf, af) = (fold(home), fold(away));
    let mut h = TeamStats::default();
    let mut a = TeamStats::default();
    for it in items {
        let data = match it.get("data").and_then(|d| d.as_array()) {
            Some(d) => d,
            None => continue,
        };
        for e in data {
            let label = e.get("label").and_then(|x| x.as_str()).unwrap_or("");
            let value = e.get("value").and_then(|x| x.as_str()).unwrap_or("");
            let lf = fold(label);
            // Whose stat? Need an unambiguous team mention in the label.
            let team = if !hf.is_empty() && lf.contains(&hf) {
                &mut h
            } else if !af.is_empty() && lf.contains(&af) {
                &mut a
            } else {
                continue;
            };
            let num = match first_num(value).or_else(|| first_num(label)) {
                Some(n) if n.is_finite() && n >= 0.0 && n < 60.0 => n,
                _ => continue,
            };
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
        }
    }
    (h, a)
}

/// Validate a scraped per-game mean for a ONE-TEAM market: return None if it's
/// above `skip_above` (almost certainly a both-teams/match total mis-scraped onto
/// one team), otherwise clamp it into [lo, hi] so the emitted lines stay realistic.
fn sane(mu: f64, lo: f64, hi: f64, skip_above: f64) -> Option<f64> {
    if !mu.is_finite() || mu >= skip_above {
        None
    } else {
        Some(mu.clamp(lo, hi))
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
                flags: vec!["source: ingested 3rd-party stats (independent of the engine)".to_string()],
                plausibility: None,
            });
        }
    }
}

/// Deterministic candidates for ONE fixture, derived purely from its matched
/// ingested pages. Returns empty if the pages carried no usable stats.
pub fn candidates_for_fixture(items: &[&Value], home: &str, away: &str, fixture_label: &str, fixture_id: i64) -> Vec<Candidate> {
    let (h, a) = parse(items, home, away);
    let mut out = Vec::new();
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
