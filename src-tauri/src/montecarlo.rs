//! Correlated same-game-parlay (SGP) pricing by Monte Carlo.
//!
//! The naive parlay price multiplies the legs' probabilities — which assumes
//! they're INDEPENDENT. For a same-game parlay that's wrong: "team to win" and
//! "their striker to score" move together, so the true joint probability is
//! HIGHER than the product. Treating them as independent under-prices the ticket
//! (you'd think it's less likely than it is) and hides real value.
//!
//! We fix this with a Gaussian copula in a one-factor-per-group form, which is
//! POSITIVE-SEMIDEFINITE by construction (so it always samples cleanly):
//!
//!   z_i = Lg·G(fixture) + Lt·T(fixture,theme) + Ls·S(fixture,subject) + Li·e_i
//!
//! Each factor is a standard normal; the squared loadings sum to 1, so every
//! leg's latent z_i is still N(0,1) and its MARGINAL probability is preserved
//! EXACTLY (leg i hits iff z_i ≤ Φ⁻¹(p_i)). We never invent or alter a leg's
//! probability — we only model how the legs co-occur. Legs in different fixtures
//! share no factor, so multi-match accumulators stay ≈ the independent product.
//!
//! The RNG is deterministic (seeded from the legs) so the same ticket always
//! prices identically — reproducible and cacheable.

use serde::Serialize;

// Per-fixture common factor (every leg in a match shares it).
const L_GAME: f64 = 0.25;

// NON-scoreline legs (player props, corners, cards…): game + theme + subject.
// Theme raised to 0.55 after the model-implied-correlation study (see below).
const L_THEME: f64 = 0.55;
const L_SUBJ: f64 = 0.50;
const L_IDIO: f64 = 0.620_484; // sqrt(1 − 0.25² − 0.55² − 0.50²)

// SCORELINE legs (result, DC, BTTS, totals, handicap, correct score, goals
// range, team goals) are all DETERMINISTIC FUNCTIONS OF THE SAME FINAL SCORE, so
// they share an extra strong per-fixture "scoreline" factor on top of the goal
// theme. My study found the DC model implies φ≈0.5-0.6 between such legs while a
// flat theme loading only gave ≈0.16-0.24; this brings same-fixture scoreline
// pairs up to φ≈0.35-0.5 (toward the model) without the heavier exact-grid
// pricing — and they still correlate with player goal legs (scorer, shots) via
// the shared goal theme.
const LS_THEME: f64 = 0.45; // shared with non-scoreline goal-theme legs (e.g. scorer)
const LS_SCORE: f64 = 0.55; // the "same final score" factor (scoreline legs only)
const LS_SUBJ: f64 = 0.40;
const LS_IDIO: f64 = 0.522_015; // sqrt(1 − 0.25² − 0.45² − 0.55² − 0.40²)

/// Does this market resolve purely from the full-time score? (Then it shares the
/// scoreline factor.) Halves use a different score, so they're excluded.
pub fn is_scoreline_market(market: &str) -> bool {
    let m = market.to_lowercase();
    if m.contains("half") {
        return false;
    }
    m.contains("match result")
        || m.contains("double chance")
        || m == "btts"
        || m.contains("both teams")
        || m.contains("asian handicap")
        || m.contains("correct score")
        || m.contains("goals range")
        || m.contains("team total goals")
        || (m.contains("goal") && (m.contains("over") || m.contains("under")))
}

#[derive(Clone)]
pub struct SimLeg {
    pub fixture_id: i64,
    pub subject: String, // folded
    pub theme: &'static str,
    pub prob: f64,        // marginal — preserved exactly
    pub scoreline: bool,  // resolves from the final score → shares the score factor
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SgpPrice {
    /// Joint P(all legs hit) WITH correlation — the honest number.
    pub correlated: f64,
    /// Naive product of marginals (independence assumption).
    pub independent: f64,
    /// correlated / independent. >1 ⇒ legs help each other (typical SGP).
    pub lift: f64,
    /// Fair decimal odds from the correlated probability (1/correlated).
    pub fair_odds: f64,
    pub legs: usize,
    pub sims: usize,
}

/// Group a leg into a correlation "theme" from its market + direction. Same-theme
/// legs in the same fixture move together; opposite directions are split so an
/// "over" and an "under" don't get forced to correlate positively.
pub fn theme_of(market: &str, line: &str, selection: &str) -> &'static str {
    let m = market.to_lowercase();
    let s = format!("{} {}", line, selection).to_lowercase();
    let under = s.contains("under") || s.trim_start().starts_with("no");
    if m.contains("card") {
        "cards"
    } else if m.contains("corner") {
        "corners"
    } else if m.contains("offside") {
        "offsides"
    } else if m.contains("save") {
        "saves"
    } else if m.contains("foul") {
        "fouls"
    } else if m.contains("tackle") || m.contains("pass") {
        "possession"
    } else if m.contains("goal")
        || m.contains("score")
        || m.contains("win")
        || m.contains("both teams")
        || m.contains("shot")
        || m.contains("assist")
        || m.contains("over/under")
        || m.contains("result")
        || m.contains("double chance")
        || m.contains("handicap")
        || m.contains("clean sheet")
    {
        if under {
            "goals_down"
        } else {
            "goals_up"
        }
    } else {
        "other"
    }
}

/// Deterministic splitmix64 RNG → reproducible pricing for a given ticket.
struct Rng(u64);
impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unif(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) * (1.0 / (1u64 << 53) as f64)
    }
    fn normal(&mut self) -> f64 {
        let u1 = self.unif().max(1e-12);
        let u2 = self.unif();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

/// Inverse standard-normal CDF (Acklam's rational approximation, ~1e-9 abs err).
fn inv_norm(p: f64) -> f64 {
    if p <= 0.0 {
        return -1e9;
    }
    if p >= 1.0 {
        return 1e9;
    }
    const A: [f64; 6] = [-3.969683028665376e+01, 2.209460984245205e+02, -2.759285104469687e+02, 1.383577518672690e+02, -3.066479806614716e+01, 2.506628277459239e+00];
    const B: [f64; 5] = [-5.447609879822406e+01, 1.615858368580409e+02, -1.556989798598866e+02, 6.680131188771972e+01, -1.328068155288572e+01];
    const C: [f64; 6] = [-7.784894002430293e-03, -3.223964580411365e-01, -2.400758277161838e+00, -2.549732539343734e+00, 4.374664141464968e+00, 2.938163982698783e+00];
    const D: [f64; 4] = [7.784695709041462e-03, 3.224671290700398e-01, 2.445134137142996e+00, 3.754408661907416e+00];
    let plow = 0.02425;
    let phigh = 1.0 - plow;
    if p < plow {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= phigh {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    }
}

fn round4(x: f64) -> f64 {
    (x * 10000.0).round() / 10000.0
}

/// Price an SGP: returns the correlation-aware joint probability alongside the
/// naive independent product, so the UI can show the lift (the hidden value).
pub fn sgp_probability(legs: &[SimLeg], sims: usize) -> SgpPrice {
    let n = legs.len();
    let independent: f64 = legs.iter().map(|l| l.prob.clamp(1e-6, 1.0 - 1e-6)).product();
    if n == 0 {
        return SgpPrice::default();
    }
    if n == 1 {
        let p = round4(legs[0].prob);
        return SgpPrice { correlated: p, independent: p, lift: 1.0, fair_odds: round4(1.0 / p.max(1e-6)), legs: 1, sims: 0 };
    }

    // Map each leg to its three factor indices.
    let key_index = |keys: &mut Vec<String>, k: String| -> usize {
        if let Some(i) = keys.iter().position(|x| *x == k) {
            i
        } else {
            keys.push(k);
            keys.len() - 1
        }
    };
    let (mut gk, mut tk, mut sk): (Vec<String>, Vec<String>, Vec<String>) = (Vec::new(), Vec::new(), Vec::new());
    let mut gi = vec![0usize; n];
    let mut ti = vec![0usize; n];
    let mut si = vec![0usize; n];
    let mut thr = vec![0f64; n];
    let mut seed: u64 = 0x243F_6A88_85A3_08D3;
    for (j, l) in legs.iter().enumerate() {
        gi[j] = key_index(&mut gk, format!("{}", l.fixture_id));
        ti[j] = key_index(&mut tk, format!("{}|{}", l.fixture_id, l.theme));
        si[j] = key_index(&mut sk, format!("{}|{}", l.fixture_id, l.subject));
        thr[j] = inv_norm(l.prob.clamp(1e-6, 1.0 - 1e-6));
        // Fold leg identity into the seed for reproducible-but-distinct streams.
        for b in l.subject.bytes().chain(l.theme.bytes()) {
            seed = seed.wrapping_mul(0x0100_0000_01B3).wrapping_add(b as u64);
        }
        seed ^= (l.fixture_id as u64).rotate_left((j as u32) & 63);
    }

    let mut rng = Rng(seed | 1);
    // game + scoreline factors are both per-fixture (indexed by gi); theme and
    // subject have their own keyed factor vectors.
    let (mut gfac, mut scfac) = (vec![0f64; gk.len()], vec![0f64; gk.len()]);
    let (mut tfac, mut sfac) = (vec![0f64; tk.len()], vec![0f64; sk.len()]);
    let mut hits = 0usize;
    for _ in 0..sims {
        for v in gfac.iter_mut() {
            *v = rng.normal();
        }
        for v in scfac.iter_mut() {
            *v = rng.normal();
        }
        for v in tfac.iter_mut() {
            *v = rng.normal();
        }
        for v in sfac.iter_mut() {
            *v = rng.normal();
        }
        let mut all = true;
        for j in 0..n {
            // Scoreline legs add the shared "same final score" factor (and use a
            // slightly lower theme loading so total variance stays 1).
            let z = if legs[j].scoreline {
                L_GAME * gfac[gi[j]] + LS_THEME * tfac[ti[j]] + LS_SCORE * scfac[gi[j]] + LS_SUBJ * sfac[si[j]] + LS_IDIO * rng.normal()
            } else {
                L_GAME * gfac[gi[j]] + L_THEME * tfac[ti[j]] + L_SUBJ * sfac[si[j]] + L_IDIO * rng.normal()
            };
            if z > thr[j] {
                all = false;
                break;
            }
        }
        if all {
            hits += 1;
        }
    }

    let correlated = round4(hits as f64 / sims as f64);
    let independent = round4(independent);
    let lift = if independent > 1e-9 { round4(correlated / independent) } else { 1.0 };
    SgpPrice {
        correlated,
        independent,
        lift,
        fair_odds: round4(1.0 / correlated.max(1e-6)),
        legs: n,
        sims,
    }
}
