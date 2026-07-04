//! Anthropic (Opus 4.8) integration. Exactly one call per Build. The caller
//! (commands.rs) handles the input-hash cache; this module builds the request,
//! calls `/v1/messages`, and validates strict JSON with a single stricter retry.

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::models::BuildResult;
use crate::AppState;

/// The default BUILD/analysis model: Haiku — sharp on qualitative work (picking,
/// combining, explaining) and cheap. DeepSeek does the deterministic DATA
/// crunching (plausibility, ingest extraction); Sonnet/Opus are premium, opt-in.
pub const DEFAULT_MODEL: &str = "claude-haiku-4-5";
/// The model for pure DATA extraction (ingest scraping) — cheap DeepSeek, huge
/// context, thinking disabled.
pub const DETERMINISTIC_MODEL: &str = "deepseek-v4-pro";
/// The model for cheap QUALITATIVE helpers (plausibility, tactics) — Haiku is
/// sharper here and caches reliably (DeepSeek was failing and re-running).
pub const QUAL_MODEL: &str = "claude-haiku-4-5";

/// Premium models — high cost, so only used when the user explicitly picks them.
pub fn is_premium_model(m: &str) -> bool {
    matches!(m, "claude-sonnet-5" | "claude-opus-4-8")
}
const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const OPENAI_ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";
const DEEPSEEK_ENDPOINT: &str = "https://api.deepseek.com/anthropic/v1/messages";

/// A DeepSeek model id (OpenAI-compatible API).
pub fn is_deepseek_model(m: &str) -> bool {
    m.starts_with("deepseek")
}

/// Allowed model ids for the main BUILD.
pub fn is_allowed_model(m: &str) -> bool {
    is_deepseek_model(m) || matches!(m, "claude-opus-4-8" | "claude-sonnet-5" | "claude-haiku-4-5")
}

/// An OpenAI (GPT) model id.
pub fn is_openai_model(m: &str) -> bool {
    m.starts_with("gpt-")
}

/// Models allowed for the quick ANALYSIS (a second angle).
pub fn is_allowed_analysis_model(m: &str) -> bool {
    is_allowed_model(m) || matches!(m, "gpt-5-nano" | "gpt-5-mini")
}

/// (input, output) USD price per 1M tokens. GPT/DeepSeek prices are estimates.
/// Unknown Claude ids fall back to Opus.
pub fn model_pricing(model: &str) -> (f64, f64) {
    match model {
        _ if model.starts_with("deepseek") => (0.30, 1.20),
        "claude-sonnet-5" => (3.0, 15.0),
        "claude-haiku-4-5" => (1.0, 5.0),
        "gpt-5-nano" => (0.05, 0.40),
        "gpt-5-mini" => (0.75, 1.25),
        _ if model.starts_with("gpt-") => (0.75, 1.25),
        _ => (5.0, 25.0), // claude-opus-4-8
    }
}

pub fn cost_usd(model: &str, input: i64, output: i64) -> f64 {
    let (pi, po) = model_pricing(model);
    let cost = (input as f64) / 1_000_000.0 * pi + (output as f64) / 1_000_000.0 * po;
    (cost * 10000.0).round() / 10000.0
}

const SYSTEM_PROMPT: &str = r#"You are a football betting research assistant building a SLATE of value-oriented tickets. You receive a compact table of PRE-COMPUTED candidate legs across many markets (player props AND team/match lines). Each row has est_prob (our model probability the line hits) and, where the market is priced, pinnacle_prob (Pinnacle's de-vigged TRUE probability — sharp), book_odds (Bet365 decimal odds — the price you'd take) and ev (= book_odds * pinnacle_prob - 1, the +EV edge). All numbers are pre-computed; never invent or recompute a probability or odd.

Your job: assemble the requested slate of tickets leaning toward VALUE and calculated longshots — NOT just safe bankers. Quality beats quantity: a smaller slate of coherent tickets beats a padded one. Each ticket is one of:
- "Single": one leg.
- "SGP": 2-5 legs from the SAME fixture (same-game parlay).
- "SGP+": a Bet365-style build that COMBINES the most VIABLE same-game pieces across fixtures. It is assembled from BUILDING BLOCKS: a highly-viable same-game mini-SGP (2-3 CORRELATED legs from one match), and/or strong singles, and/or another mini-SGP from a DIFFERENT match — stacked together. At least ONE fixture MUST contribute a 2+ leg correlated mini-SGP. ALWAYS order the legs GROUPED BY FIXTURE (all of match A's legs together, then match B's) so it reads cleanly, and only include pieces that are genuinely viable on their own — an SGP+ is a bundle of GOOD bets, not padding. NOTE: one leg per fixture is just a cross-game acca, NOT an SGP+.

DEFAULT lean (VALUE): prioritise legs with positive ev (price beats the true probability) — that is the exploitable edge. HOWEVER, the STRATEGY block in the user message OVERRIDES this default: when it asks for stat-led/likelihood-led picks (Bankers, Scout, Secret Picks, …), rank on est_prob, measured recent hit-rates and the player's role/ability — and treat price/ev as a TIE-BREAKER only, never a veto. Legs without odds (most player props aren't priced) can still be used to build bet builders from est_prob. Make the slate a MIX: some +EV singles, several SGPs, and longshot bet builders.

BET BUILDERS: unless the strategy or allowed ticket types say otherwise, include multi-leg bet builders (SGP — same fixture, or SGP+ — across fixtures) of 3-5 legs, typically ~3 of them where the pool supports it, chosen so the legs' est_prob values MULTIPLY to roughly a 10-20% combined hit chance (genuine longshots, ~5x-10x). Use each leg's est_prob to gauge this. Skip this when it conflicts with the requested strategy/types.

CORRELATED & THEMATIC SGPs (STRONGLY PREFERRED): the best same-game builders tell ONE story where the legs reinforce each other, so they hit TOGETHER more often than naive independence implies. Build at least one THEMED SGP, e.g.:
- A "goals" theme: a team to win + their key player to score + over 1.5 team goals + BTTS.
- A "cards/physical" theme: both teams to receive a card + a known booker to be carded + the dirtier team for most cards + over the match cards line — these all rise together in a heated/derby game.
- A "shots/attacking" theme: a high-volume side's two main shooters for shots + team shots over + team corners over.
Pick legs that move in the SAME direction (positively correlated). NEVER stack contradictory or mutually-exclusive legs, and never two nested lines for the same player (a goal implies a shot). NEVER pad a ticket with a near-certainty (est_prob ≥ ~0.90 or odds under ~1.15): it adds almost no payout while its real failure chance can still sink the whole ticket — risk without return. Every leg must EARN its place with meaningful odds.

Rules: a player who hasn't scored recently is only a strong scorer pick when form_state="due_regression"; "cold_falling_off" means down-rank. Injured/suspended subjects cannot feature. If xg_source="proxy" or a leg carries a proxy/crude flag, lower confidence. NEVER stack nested/correlated legs for the SAME player in one ticket — a goal implies a shot on target which implies a shot, so pick only ONE of {anytime scorer, shots on target, player shots} per player (the others are redundant). Likewise never combine two lines of the same team goals/corners market. NEVER combine MUTUALLY-IMPLIED legs in one ticket: "Both teams to score" already GUARANTEES "Over 1.5 goals" (and almost always Over 0.5/1.5/the lower over lines) — pick ONE, not both; "Over 2.5" + "Over 1.5" is redundant (keep the higher); a team to win + that same team Double-Chance is redundant. Each leg in a ticket must add a GENUINELY new condition.

CRITICAL — TEAM vs MATCH totals: a "Team Shots / Team Corners / Team Total Cards" line is for ONE team only (e.g. Mexico's own shots), NOT the match total of both teams. Never attach a both-teams figure to a single team: one team almost never exceeds ~18 shots, ~9 corners or ~4 cards in a game, so an "over 26.5 shots" or "over 14.5 corners" line on a SINGLE team is wrong — that is a match total. Only ever use a row EXACTLY as given in the table; never invent a line or move a total onto one team.

CRITICAL for matching: in every leg, copy the row's "subject" verbatim into "selection" and the row's "market" verbatim into "market" (and its "line"). Do NOT put probabilities or odds in legs — those are filled in automatically afterwards. Treat predictions and the user's notes as soft context. HONESTY in "why": cite ONLY numbers and facts that appear in the table or the context blocks — NEVER invent a stat, a form run ("scored 5 in his last 3"), an injury, or an odds figure that is not shown. With no shown fact to cite, describe the construction ("high-likelihood legs, one per match") instead of inventing one. Output strict JSON only."#;

/// Options that shape the requested slate.
pub struct PromptOpts<'a> {
    pub count: u32,
    pub types: &'a [String],
    pub variation: u32,
    pub exclude: &'a [String],
    pub bias_builders: bool,
    pub grok_veto: bool,
    /// "value" | "favorites" | "likely".
    pub strategy: String,
    /// "Feeling Lucky" tiers: bankers (>~75%), moderate (~40%), longshots (>~10%).
    pub lucky_safe: u32,
    pub lucky_moderate: u32,
    pub lucky_risky: u32,
    /// Minimum legs per ticket (1 = off). Forces 4-folds, 6-folds, etc.
    pub min_legs: u32,
    /// Every multi-leg ticket must span EVERY fixture (≥1 leg each).
    pub cover_all: bool,
    /// Max times a single player/team may appear across the slate (0 = model default).
    pub max_per_subject: u32,
    /// Apex strategy only: deterministically-priced correlated combos (Monte-
    /// Carlo copula) rendered as prompt lines. Empty = none / not Apex.
    pub apex_combos: String,
}

impl PromptOpts<'_> {
    fn lucky_total(&self) -> u32 {
        self.lucky_safe + self.lucky_moderate + self.lucky_risky
    }
}

/// input_hash covers everything that affects the output, including the variation
/// seed (so a "new set" is a distinct cache entry).
pub fn input_hash(
    table: &str,
    markets: &[String],
    reasoning: bool,
    model: &str,
    notes: &str,
    predictions: &[String],
    grok: Option<&str>,
    opts: &PromptOpts,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(table.as_bytes());
    hasher.update(b"|");
    hasher.update(markets.join(",").as_bytes());
    hasher.update(b"|");
    hasher.update(if reasoning { b"reason=1" as &[u8] } else { b"reason=0" });
    hasher.update(b"|");
    hasher.update(model.as_bytes());
    hasher.update(b"|");
    hasher.update(notes.trim().as_bytes());
    hasher.update(b"|");
    hasher.update(predictions.join(";").as_bytes());
    hasher.update(b"|");
    hasher.update(opts.count.to_le_bytes());
    hasher.update(opts.types.join(",").as_bytes());
    hasher.update(opts.variation.to_le_bytes());
    hasher.update(opts.exclude.join(";").as_bytes());
    hasher.update(if opts.bias_builders { b"bias=1" as &[u8] } else { b"bias=0" });
    hasher.update(opts.strategy.as_bytes());
    hasher.update(if opts.grok_veto { b"veto=1" as &[u8] } else { b"veto=0" });
    hasher.update(opts.lucky_safe.to_le_bytes());
    hasher.update(opts.lucky_moderate.to_le_bytes());
    hasher.update(opts.lucky_risky.to_le_bytes());
    hasher.update(opts.min_legs.to_le_bytes());
    hasher.update(opts.max_per_subject.to_le_bytes());
    hasher.update(b"|");
    hasher.update(opts.apex_combos.as_bytes());
    hasher.update(if opts.cover_all { b"cover=1" as &[u8] } else { b"cover=0" });
    hasher.update(b"|");
    hasher.update(grok.unwrap_or("").as_bytes());
    format!("{:x}", hasher.finalize())
}

fn user_prompt(
    table: &str,
    markets: &[String],
    reasoning: bool,
    notes: &str,
    predictions: &[String],
    grok: Option<&str>,
    opts: &PromptOpts<'_>,
) -> String {
    let grok_block = match grok {
        Some(g) if !g.trim().is_empty() => format!(
            "\n\nX/News & team-news digest (Grok live search — SOFT context). Use it especially to AVOID any player reported INJURED, SUSPENDED or RULED OUT, and to weigh late team news; do NOT let it override the computed numbers:\n{}",
            g.trim()
        ),
        _ => String::new(),
    };
    let why_clause = if reasoning {
        r#"Include a "why" field (one to two sentences) per ticket."#
    } else {
        r#"OMIT the "why" field entirely from every ticket."#
    };
    let notes_block = if notes.trim().is_empty() {
        "(none)".to_string()
    } else {
        notes.trim().to_string()
    };
    let pred_block = if predictions.is_empty() {
        "(none)".to_string()
    } else {
        predictions.join("\n")
    };
    let types = if opts.types.is_empty() {
        "Single, SGP, SGP+".to_string()
    } else {
        opts.types.join(", ")
    };
    let count = if opts.count == 0 { 10 } else { opts.count };
    let lucky_block = if opts.lucky_total() > 0 {
        let mut tiers = String::new();
        if opts.lucky_safe > 0 {
            tiers.push_str(&format!(
                "\n- {} 'Feeling Lucky · Safe' ticket(s): 2-3 legs, each leg individually strong, chosen so the combined hit chance stays ABOVE ~75% (short-priced banker parlay).",
                opts.lucky_safe
            ));
        }
        if opts.lucky_moderate > 0 {
            tiers.push_str(&format!(
                "\n- {} 'Feeling Lucky · Moderate' ticket(s): 3-4 legs, chosen so the combined hit chance lands around ~40% (mid-priced).",
                opts.lucky_moderate
            ));
        }
        if opts.lucky_risky > 0 {
            tiers.push_str(&format!(
                "\n- {} 'Feeling Lucky · Risky' ticket(s): 4-6 legs, chosen so the combined hit chance is ABOVE ~10% but well under 25% (big-odds longshot).",
                opts.lucky_risky
            ));
        }
        format!(
            "\n\nFEELING LUCKY: in ADDITION to the {} main tickets above, include these {} EXTRA tickets — they are SEPARATE and NEVER count toward the main {} (final output = {} main + {} lucky = {} tickets). Use each leg's est_prob to gauge the combined hit chance, and title each EXACTLY as named:{tiers}",
            count, opts.lucky_total(), count, count, opts.lucky_total(), count + opts.lucky_total()
        )
    } else {
        String::new()
    };
    let bias_block = if opts.bias_builders {
        "\n\nBias bet builders toward PRICED markets (anytime scorer, goals over/under, BTTS, match result) so each builder has real Bet365 odds and a real combined price — minimise unpriced player-prop legs inside builders."
    } else {
        ""
    };
    let only = |t: &str| opts.types.len() == 1 && opts.types[0].eq_ignore_ascii_case(t);
    let typerule = if opts.strategy == "power" {
        "EVERY ticket MUST be a cross-game DOUBLE — exactly 2 legs from 2 DIFFERENT fixtures — or, only occasionally, a TREBLE (3 legs, 3 fixtures). NEVER a single, NEVER 4+ legs, NEVER two legs from the same match. The legs together MUST reach AT LEAST 4.0 combined odds. Label the type \"Double\" or \"Treble\"."
    } else if only("SGP+") {
        "EVERY ticket MUST be a TRUE SGP+ — 4-6 legs spanning 2+ fixtures, where AT LEAST ONE fixture contributes 2+ CORRELATED same-game legs (a real mini-SGP), then joined with leg(s) from other fixture(s). NEVER one leg per fixture (that's just an acca), NEVER all from one match, NEVER a single. Group legs by fixture, make each group's legs reinforce each other, and in the 'why' note which fixture is the same-game core. If too few fixtures are selected, still produce as many genuinely different ones as possible."
    } else if only("SGP") {
        "EVERY ticket MUST be an SGP — 2-5 legs ALL within ONE fixture (never a single, never cross-fixture)."
    } else if only("Single") {
        "EVERY ticket MUST be a SINGLE — exactly one leg."
    } else {
        "Use only the allowed ticket types; mix singles, SGPs and SGP+ across the slate."
    };
    let strategy_block = match opts.strategy.as_str() {
        "likely" => "SECRET-PICKS mode — ignore the +EV preference. Hunt for the selections most likely to LAND FOR A REASON, including non-obvious ones the market underrates. Lean hard on the Grok digest and the API predictions for CONTEXT: a must-win/elimination motivation, momentum, a favourable matchup, an in-form scorer, weakened opposition. Actively include UNDERDOGS (team-win odds ~2.5-6.0) and value scorers (~3.5+) when the context genuinely supports them — e.g. a side that desperately needs the win, plus their in-form forward to score — and combine those into bigger-odds tickets. Do NOT just stack chalk favourites; surface the plausible upset and the under-the-radar scorer, and say WHY in the ticket.",
        "favorites" => "FORM-FAVOURITES mode — build CONFIDENT tickets from IN-FORM favourites priced at USEFUL odds (roughly 1.5-2.5 per leg): a known scorer in good form vs a leaky defence, a strong team to win, a reliable shots-on-target line. AVOID boring odds-on chalk (under ~1.4) and AVOID longshots. Combine 3-5 such legs into bankable parlays (~5-12x total). Lean on form, league standings and head-to-head to judge reliability. +EV is a bonus, not required — prioritise genuine confidence and recent form.",
        "oracle" => "ORACLE mode — this is Claude's OWN read, built on CONFLUENCE. Pick a leg ONLY when independent signals AGREE: (1) the sharp Pinnacle de-vig (pinnacle_prob) says the true probability is genuinely solid, (2) our model (est_prob) independently agrees — a SMALL gap between the two, so it isn't a trap line or a lone-wolf guess, and (3) the takeable price (book_odds) beats that fair probability — a REAL edge, not an imaginary one. Favour UNDER-THE-RADAR selections at fair-to-generous odds (~1.7-3.2): in-form players (see the in-form flag), suited roles (a tall striker for headers, a high-volume shooter for shots/SOT), confirmed to feature. FADE three things on purpose and say so when relevant: odds-on chalk (<1.5 — risk with no edge), lottery longshots (>3.6 — variance masquerading as value), and ANY leg where est_prob and pinnacle_prob disagree sharply (one read is wrong — pass). Build tickets with genuine conviction. In each ticket's 'why', name the CONFLUENCE explicitly — the 2-3 signals that line up (e.g. 'sharp 41% + our 39% + in-form + 2.4 price = real edge') — and flag the single biggest point of failure. Quality over quantity; do not pad with weak legs to hit a number — but still return the requested count using only legs that clear the bar.",
        "bankers" => "BANKERS mode — build tickets from HIGH-LIKELIHOOD RECURRING events: things that happen most games for that player/team. Lead with picks that carry a 'hit N/M recent' note (we measured how often it actually happened lately) — a regular booker To Be Carded, a reliable shooter for 1+ shot, a high-volume passer over their passes line, a corner-heavy team's corners over. Prefer ~60%+ legs; combine 3-6 of them into bankable parlays. These are the 'this basically always happens' picks — favour reliability and recent consistency over price or flashiness, but DO say in the 'why' how often each leg has landed recently. STAT-FIRST RANKING: conviction comes from est_prob, the measured hit-rates and the player's role/volume (a regular booker books, a high-volume shooter shoots) — ev and price are a tie-breaker between otherwise-equal picks, NEVER a reason to drop a statistically strong leg (unpriced or negative-EV stat-bankers still belong).",
        "power" => "POWER-STACKER mode — low-leg, high-conviction parlays: lottery-like payouts with FEWER things to connect. Build cross-game DOUBLES (occasionally a treble). Every leg must be a HIGH-LIKELIHOOD outcome that 'should happen' yet is still priced GENEROUSLY (~1.8-2.5) because the book is enticing action — a dominant favourite to win (~2.0), an in-form scorer the book shades (~2.2), a soft over. Stack TWO such legs so the COMBINED odds clear AT LEAST 4.0 (ideally 5-10x). You may pair ONE slightly-less-expected but still-likely leg (~2.5-5.0) with a near-certain ~2.0 leg to reach ~10x on something genuinely simple. Each leg from a DIFFERENT fixture; across the whole slate MAXIMISE diversity — different teams, players AND markets every ticket, never reuse the same selection. Lower variance is the point: do not over-stack. In each 'why', state the combined odds and explain why BOTH legs 'must happen'.",
        "predictor" => "MATCH-PREDICTOR mode — a DEEP read of ONE single game. The whole market for this fixture is in the table (every player prop and match prop). Build 6-8 DIFFERENT same-game tickets, each a distinct ANGLE on how the match plays out, so together they paint the full picture: e.g. (1) a SAFE banker SGP of high-likelihood legs, (2) a GOALS-themed SGP (a side to win + their scorer + over + BTTS), (3) a CARDS/physical SGP, (4) a STAR-PLAYERS SGP (the key men for shots/SOT/to score), (5) a CORRECT-SCORE-led build, (6) a VALUE longshot. Each ticket 3-5 correlated same-game legs that tell ONE coherent story. Mix player props AND match props across the set; never reuse the same selection across tickets. In each 'why', say what scenario it's betting on.",
        "jackpot" => "JACKPOT mode — deliberate LOTTERY TICKETS: small stake, life-changing-if-it-lands payout. Build big multi-leg parlays (5-8 legs) whose COMBINED hit chance is roughly 1-5% (use each leg's est_prob; the product should land ~0.01-0.05) and whose combined odds are LARGE (~20x to 150x+). Crucially these are PLAUSIBLE longshots, NOT random junk: every single leg must be a genuinely REASONABLE outcome on its own (an in-form scorer to score, a strong favourite to win, a likely card/shot/corner over) — the ticket is a longshot only because MANY reasonable things must ALL happen. Prefer POSITIVELY CORRELATED legs (same-game builds: a team to win + their striker to score + match over) so the legs reinforce each other and the true joint chance beats the naive product. Lean on the in-form flag, the API predictions and the Grok digest for which longshots are live. NEVER stack contradictory or mutually-exclusive legs. In each 'why', state the combined odds and the rough hit chance, and name the one leg most likely to break it.",
        "scout" => "SCOUT mode — FUSE TWO INDEPENDENT SOURCES into the picks: (A) OUR full engine table above (every market, built from our API data + models), and (B) the FULL INGESTED PAGE(S) the user hand-fed for these fixtures (injected in the notes below — corners, cards, shots, form, xG, injuries, predictions, analyst reads, whatever the page carried). Some legs in the table are flagged 'from ingested stats' (derived straight from the page's numbers); the rest are ours. Your job: cross-reference the two and surface the picks they JOINTLY support. Rules: (1) Where our number and the ingested number AGREE on a line, that's your strongest conviction — lead with those. (2) Where they DISAGREE, judge which source to trust for that market and SAY why (e.g. the page has fresher corner data; our injury read is better). (3) Use the page's EXTRA angles our table can't see (a noted suspension, a tactical mismatch, a predicted scoreline) to pick or veto legs across ANY market, not just corners/cards/shots. (4) The ingested data is the user's EDGE — it must materially shape the slate, not be ignored. Build a WIDE, varied set — lots of singles plus 2-5 leg builds across the fixtures, so the user can cherry-pick. Surface not just near-certainties but SOLID MODERATE picks too: any leg with est_prob down to ~0.30 is legitimate data worth showing (a ~30-45% shot on a decent price is exactly the kind of edge to surface) — do NOT restrict to only high-probability lines. Never stack mutually-implied or same-team-same-market lines. In each 'why', name what BOTH sources said (e.g. 'our 5.6 + page 6.4 corners → over 4.5 strong' or 'page flags their CB suspended → fade their clean sheet'). STAT-FIRST RANKING: this mode ranks on the STATS — est_prob, the two sources' agreement, measured recent rates, and the player's role/playstyle fit (volume shooter, set-piece taker, physical matchup). ev/price is ONLY a tie-breaker between otherwise-equal picks; never discard a statistically strong pick because it is unpriced or its EV is negative.",
        "stacker" => "STACKER mode — a RISK-CONTROLLED, STAT-LED accumulator builder. Canvas ALL the fixtures and stack 5-8 measured picks into parlays that pay a real multiplier while staying grounded in the data. This is NOT +EV hunting — conviction comes from est_prob, measured hit-rates and role/playstyle. THE CORE TRICK — STEP UP CHALK: when the obvious line is priced too short to matter (under ~1.30, or est_prob ≥ ~0.85), take the NEXT line up instead when it is still genuinely plausible (est_prob ≥ ~0.40): corners over 5.5 at 1.10 becomes over 6.5/7.5; a near-certain scorer becomes Multi Scorer (2+); a passer's soft line steps up a band. Every leg should sit roughly in the 1.3-2.2 sweet spot after stepping — real payout contribution, still likely. Build the FULL REQUESTED COUNT of cross-game parlays (5-8 legs each; the count instruction above is the contract — never return fewer because the shape feels repetitive; vary the combinations instead), mixing SAFE stepped legs (~60-75%) with 1-2 measured risks (~40-55%) per ticket; target combined odds ~6x-25x with a genuine fighting chance (combined est_prob product ~8-25%). COHERENCE — the best stacks read like ONE idea, not a grab-bag. Two proven shapes, build both across the slate: (1) THEMED single-market stack (e.g. a scorer 5-fold, one per match: two regulars ~1.4-1.6, two mid ~1.9-2.2, one measured risk ~2.5-2.8), titled for the theme. (2) MIXED-BUT-CLEAN multi: ONE leg per match, each leg being THAT match's strongest mainstream story per the data — the clear favourite to win, the in-form star to score, a high-tempo pairing over 2.5, BTTS where both sides attack — every leg ~1.4-2.2, reading like a sharp bettor's Saturday multi. SAME-GAME STARS ARE LEGITIMATE: two strong scorers from ONE match — even opposing teams (an open Brazil-Norway: Vinicius AND Haaland to score) — are positively correlated in high-scoring games and can anchor a ticket together as its same-game core. PICK THE MATCH'S BEST STORY: don't force the same market everywhere — if the data's strongest read for one game is a tackle machine or a corner-heavy side, use it — but PREFER RECOGNIZABLE MAINSTREAM markets (result, over/under, BTTS, scorer, SOT, corners) and avoid niche legs (offsides, fouls drawn, passes) unless the number is exceptional AND the leg still reads naturally. TRUE GOLD over scraps: the right leg is the one any sharp bettor would take once shown the stat, at a fair price — never an obscure line chosen just because a formula likes it. NEVER pad with sub-1.15 chalk (it adds risk without payout) and NEVER lottery legs under ~35% est_prob. In each 'why', name the step-ups (e.g. 'their corner rate 6.8/g → over 6.5 instead of the 1.10-priced 5.5').",
        "apex" => "APEX mode — the discipline strategy: bet ONLY where a PROVEN edge mechanism exists, pass on everything else. Two ticket sources, nothing more: (1) SHARP-EDGE SINGLES — legs where ev_source is sharp and ev ≥ +2% (the takeable price beats Pinnacle's de-vigged truth) AND our est_prob roughly agrees with pinnacle_prob (a big gap = one read is wrong = pass). (2) CORRELATED SGPs — ONLY the pre-priced combos listed in the CORRELATED COMBOS block below; copy each one verbatim as its own SGP ticket. Do NOT assemble any other multi-leg ticket, do NOT pad with likelihood-only legs, do NOT chase longshots. If few legs qualify, return a SHORT slate and say so — Apex passing on a thin slate IS the strategy working. In each 'why', name the edge mechanism and its size (e.g. 'sharp 55% vs 1.95 takeable = +7% EV' or 'copula lift x1.22 → corr-EV +9%').",
        _ => "Lean value/longshot: prioritise +EV legs (best price beats the true probability) — that is the exploitable edge.",
    };
    let apex_block = if opts.apex_combos.trim().is_empty() {
        String::new()
    } else {
        format!(
            "\n\nCORRELATED COMBOS (deterministically priced by our Monte-Carlo copula; 'corr' already accounts for the legs reinforcing each other — these are the ONLY multi-leg tickets allowed in Apex mode):\n{}\nBuild EACH combo as its own SGP ticket, copying every leg's subject/market/line verbatim from the table. Drop a combo only if team news clearly invalidates a leg (say so).",
            opts.apex_combos.trim()
        )
    };
    let variation_block = if opts.variation > 0 || !opts.exclude.is_empty() {
        format!(
            "\n\nThis is an ALTERNATIVE slate (variation {}). Produce a DIFFERENT set — do NOT repeat any of these tickets, and vary the selections, markets and combinations:\n{}",
            opts.variation,
            if opts.exclude.is_empty() { "(none listed)".to_string() } else { opts.exclude.join("\n") }
        )
    } else {
        String::new()
    };
    // Hard min-legs (forces 4-folds/6-folds) and a hard per-subject diversity cap.
    // Power Stacker defines its own leg count (doubles), so min-legs doesn't apply.
    let minlegs_block = if opts.min_legs > 1 && opts.strategy != "power" {
        format!(
            " EVERY main ticket MUST have AT LEAST {} legs (a {}-fold or bigger) — NO singles{}; never return a ticket with fewer legs.",
            opts.min_legs,
            opts.min_legs,
            if opts.min_legs > 2 { ", and nothing smaller than that fold" } else { "" }
        )
    } else {
        String::new()
    };
    // Cover-all: multi-leg tickets must span every fixture on the slate.
    let cover_block = if opts.cover_all {
        " COVERAGE REQUIREMENT (OVERRIDES everything below, including any leg-count range in the strategy description): every multi-leg ticket (SGP+/parlay) MUST include AT LEAST ONE leg from EVERY fixture in the table — a strategy that says '3-6 legs' still builds ALL-fixture tickets when this is on; pick each match's best fit for the strategy and don't fret the combined hit chance. If a fixture truly has no usable row in the table, name it in data_quality_notes rather than silently skipping it."
    } else {
        ""
    };
    let diversity_cap = if opts.max_per_subject > 0 {
        format!(
            "\n- Diversity cap: try to keep any single player or team in at most {} ticket(s) across the slate — but if honouring this would drop you below the required count, exceed the cap and still return the full count.",
            opts.max_per_subject
        )
    } else {
        String::new()
    };

    format!(
        r#"Requested market groups: {markets}
Allowed ticket types: {types}

User notes (soft context only): {notes}

Match context — predictions, league standings (motivation), head-to-head, weather, referee (weak signals, weigh them):
{preds}{grok}

Pre-computed candidate legs (each may carry "plausibility" 1-5 — a real-world context score from a scout pass: 5 = very plausible for this match, 1 = implausible/trap (rotation/role/matchup risk). PREFER higher-plausibility legs; avoid plausibility 1-2 unless the value is exceptional and you say why):
{table}

Build UP TO {count} main tickets — aim for the full {count} when the candidate pool genuinely supports it, but QUALITY BEATS COUNT: never pad the slate with weak, incoherent or near-duplicate tickets just to hit a number. If the pool is thin, return fewer, better tickets and say why in data_quality_notes. {typerule}{minlegs}{cover} {strategy} Each ticket must differ from every other by at least one leg; tickets MAY share legs when re-combined into genuinely different constructions. Any FEELING LUCKY tickets requested below are ADDITIONAL, on top of the main slate.

DIVERSITY:
- Spread subjects around so one result can't sink the whole slate — ideally a single subject (player or team) appears in only a small share of tickets.{diversitycap}
- Vary the leg combinations and odds ranges (some shorter banker-ish, some bigger longshots) so tickets aren't near-duplicates — never just swap one leg.
- Spread MARKETS too: don't lean on one market (e.g. all anytime-scorer). Mix scorer/SOT/goals/corners/result/etc. across the slate so no single market or subject is over-used.
{why}{lucky}{bias}{apex}{variation}

Output STRICT JSON ONLY, no prose outside the JSON, matching exactly:
{{
  "tickets": [
    {{
      "type": "Single | SGP | SGP+",
      "title": "short label, e.g. 'City win + Haaland anytime'",
      "confidence": "Low | Medium | High | Very High",
      "legs": [
        {{ "match": "Team A vs Team B", "market": "Anytime Scorer", "selection": "Player X", "line": "1+ goal" }}
      ],
      "flags": ["+EV", "longshot", "no price"],
      "why": "one-two sentences (omitted entirely if reasoning is off)"
    }}
  ],
  "data_quality_notes": ["string"]
}}

Each leg's "selection" and "market" MUST be copied verbatim from a table row's "subject" and "market". Do not put numbers in legs.

FINAL CHECK before you answer: (1) no ticket stacks mutually-implied, contradictory or nested same-player legs; (2) every leg's selection/market/line exists VERBATIM in the table; (3) every requested Feeling-Lucky ticket is present and titled exactly as named. Prefer dropping a weak ticket over shipping it."#,
        markets = markets.join(", "),
        types = types,
        notes = notes_block,
        preds = pred_block,
        grok = grok_block,
        table = table,
        count = count,
        typerule = typerule,
        minlegs = minlegs_block,
        cover = cover_block,
        diversitycap = diversity_cap,
        strategy = strategy_block,
        why = why_clause,
        lucky = lucky_block,
        bias = bias_block,
        apex = apex_block,
        variation = variation_block,
    )
}

/// Result of one model call: parsed tickets + token usage.
pub struct ModelCall {
    pub result: BuildResult,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

/// Make the model call and parse strict JSON, retrying once with a stricter nudge.
pub async fn call_model(
    state: &AppState,
    model: &str,
    table: &str,
    markets: &[String],
    reasoning: bool,
    notes: &str,
    predictions: &[String],
    grok: Option<&str>,
    opts: &PromptOpts<'_>,
) -> Result<ModelCall, String> {
    // No local-key guard here: anthropic_call (below) fetches the key OR routes
    // through the configured proxy, so proxy-mode installs (no local key) work.

    let prompt = user_prompt(table, markets, reasoning, notes, predictions, grok, opts);

    // Scale the output budget to the requested slate size so the JSON isn't cut
    // off. Thinking-capable models (Sonnet 5, Opus) can spend a chunk of the
    // budget on reasoning, so keep a generous ceiling.
    let count = if opts.count == 0 { 10 } else { opts.count } as i64;
    // Enough for the JSON slate + some thinking headroom, but NOT so high that a
    // thinking model runs for minutes and trips the request timeout.
    // Cover-all tickets carry ~fixtures-many legs — nearly double the JSON per
    // ticket. Starving the output truncated slates to fewer tickets than asked.
    let per_ticket: i64 = if opts.cover_all { 980 } else { 560 };
    let max_tokens = (3000 + count * per_ticket + opts.lucky_total() as i64 * 560).clamp(4000, 24000);

    // First attempt — strict parse, then salvage a truncated slate. A request
    // error here (e.g. "no text block" when the model spent the budget thinking)
    // is NOT fatal: fall through to the bigger-budget retry.
    let (mut in1, mut out1) = (0i64, 0i64);
    if let Ok((text, i, o)) = request_text(state, model, &prompt, max_tokens).await {
        in1 = i;
        out1 = o;
        if let Ok(parsed) = parse_result(&text) {
            return Ok(ModelCall { result: parsed, input_tokens: in1, output_tokens: out1 });
        }
    }

    // Retry once, bigger budget + stricter nudge (covers both unparseable JSON
    // and a first attempt that returned no usable text). The retry budget must
    // be STRICTLY LARGER than the first attempt — the old `.min(18000)` made it
    // SMALLER for big slates, exactly the case where truncation happens.
    let stricter = format!(
        "{prompt}\n\nIMPORTANT: Output ONLY the JSON object, starting with {{ and ending with }}. No markdown, no commentary, no preamble. Be concise — keep each 'why' to one short sentence so the JSON is COMPLETE and not truncated."
    );
    let (text2, in2, out2) =
        request_text(state, model, &stricter, (max_tokens + 4000).min(30000)).await?;
    let result =
        parse_result(&text2).map_err(|e| format!("model returned unparseable JSON: {e}"))?;
    Ok(ModelCall {
        result,
        input_tokens: in1 + in2,
        output_tokens: out1 + out2,
    })
}

async fn request_text(
    state: &AppState,
    model: &str,
    prompt: &str,
    max_tokens: i64,
) -> Result<(String, i64, i64), String> {
    chat_call(state, model, SYSTEM_PROMPT, prompt, max_tokens).await
}

const REFINE_SYSTEM: &str = r#"You are the senior analyst doing a FINAL pass on draft betting tickets a faster model assembled from a PRE-COMPUTED candidate table. Every number (est_prob, pinnacle_prob, book_odds, ev) is fixed and authoritative — NEVER change or invent a number. Return the FINAL polished slate as strict JSON with the SAME shape: {"tickets":[ ... ]} (each ticket: type, title, confidence, legs[{match,market,selection,line}], why).
Improve the draft: DROP incoherent, weak or duplicate tickets; FIX any ticket that stacks mutually-implied or contradictory legs (BTTS + Over 1.5; a team to win + that same team's Double Chance; two lines of the same team market; a both-teams total pinned on ONE team); where the table clearly offers a stronger leg, SWAP it in. Every leg's selection/market/line MUST exist VERBATIM in the table. Any "why" you keep or write must cite only facts shown in the table — never invented stats or form claims. Keep the best tickets and roughly the same count where quality allows. Output ONLY the JSON object."#;

/// Stage 2 of a two-stage build: a sharper model finalises the cheap model's
/// draft tickets. It only SELECTS/DROPS/SWAPS from the same table — never invents
/// numbers. Returns the refined tickets (an Err lets the caller keep the draft).
pub async fn refine_tickets(
    state: &AppState,
    model: &str,
    draft: &BuildResult,
    table: &str,
    strategy: &str,
) -> Result<(Vec<crate::models::Ticket>, i64, i64), String> {
    let draft_json = serde_json::to_string(&serde_json::json!({ "tickets": draft.tickets })).unwrap_or_default();
    let user = format!(
        "STRATEGY: {strategy}\n\nCANDIDATE TABLE (the ONLY legs allowed; all numbers are fixed):\n{table}\n\nDRAFT TICKETS to finalise:\n{draft_json}\n\nReturn the FINAL slate as strict JSON: {{\"tickets\":[ ... ]}}."
    );
    // Scale the output budget to the slate: the old flat 4000 truncated any
    // 10+-ticket slate, the parse failed, and the caller silently shipped the
    // draft — you paid for the refine model and got nothing.
    let refine_budget = (2000 + draft.tickets.len() as i64 * 450).clamp(4000, 16000);
    let (text, gin, gout) = chat_call(state, model, REFINE_SYSTEM, &user, refine_budget).await?;
    let parsed = parse_result(&text)?;
    if parsed.tickets.is_empty() {
        return Err("refine returned no tickets".to_string());
    }
    Ok((parsed.tickets, gin, gout))
}

/// Two-stage build: a cheap high-throughput model (Haiku) drafts the slate from
/// the full data, then a sharper model (Sonnet) does a tight final pass. Returns
/// (final call, draft_input_tokens, draft_output_tokens) so the caller can bill
/// each model separately. If the final pass fails, the draft is shipped as-is.
pub async fn call_model_two_stage(
    state: &AppState,
    draft_model: &str,
    final_model: &str,
    table: &str,
    markets: &[String],
    reasoning: bool,
    notes: &str,
    predictions: &[String],
    grok: Option<&str>,
    opts: &PromptOpts<'_>,
) -> Result<(ModelCall, i64, i64), String> {
    // Stage 1 — cheap model drafts from the full context.
    let draft = call_model(state, draft_model, table, markets, reasoning, notes, predictions, grok, opts).await?;
    let (draft_in, draft_out) = (draft.input_tokens, draft.output_tokens);
    // Stage 2 — sharper model finalises. On any failure, keep the draft.
    match refine_tickets(state, final_model, &draft.result, table, &opts.strategy).await {
        Ok((tickets, in2, out2)) => {
            let mut result = draft.result;
            result.tickets = tickets;
            Ok((ModelCall { result, input_tokens: in2, output_tokens: out2 }, draft_in, draft_out))
        }
        Err(_) => Ok((ModelCall { result: draft.result, input_tokens: 0, output_tokens: 0 }, draft_in, draft_out)),
    }
}

/// Generic Anthropic call → (text, input_tokens, output_tokens). Fetches the key.
pub async fn anthropic_call(
    state: &AppState,
    model: &str,
    system: &str,
    user: &str,
    max_tokens: i64,
) -> Result<(String, i64, i64), String> {
    // DeepSeek exposes an Anthropic-format API, so the same request shape works —
    // we just swap the endpoint + key. A LOCAL DeepSeek key wins; otherwise
    // proxy-mode installs route via the worker's /deepseek/ path (which holds
    // the real key) — DeepSeek used to be the one provider that ignored the
    // proxy, so Server-mode users couldn't use it at all.
    let deepseek = is_deepseek_model(model);
    let (api_key, proxy) = {
        let keys = state.keys.lock().map_err(|_| "keys lock")?;
        if deepseek {
            let local = keys.deepseek.clone();
            let proxy = if local.is_none() { keys.proxy() } else { None };
            (local, proxy)
        } else {
            (keys.anthropic.clone(), keys.proxy())
        }
    };
    if proxy.is_none() && api_key.is_none() {
        return Err(if deepseek { "DeepSeek key not set. Add it in Settings (or configure the proxy)." } else { "Anthropic key not set. Add it in Settings." }.to_string());
    }
    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": if deepseek { max_tokens.clamp(2000, 32000) } else { max_tokens },
        "system": system,
        "messages": [{ "role": "user", "content": user }]
    });
    if deepseek {
        // The numbers are already computed deterministically in Rust — DeepSeek is
        // only INTERPRETING/selecting, so heavy chain-of-thought just adds minutes
        // and burns the output budget on hidden reasoning. Turn extended thinking
        // OFF so responses are fast and the answer isn't starved.
        body["thinking"] = serde_json::json!({ "type": "disabled" });
    }

    let endpoint = if deepseek {
        match &proxy {
            Some((base, _)) => format!("{base}/deepseek/anthropic/v1/messages"),
            None => DEEPSEEK_ENDPOINT.to_string(),
        }
    } else {
        match &proxy {
            Some((base, _)) => format!("{base}/anthropic/v1/messages"),
            None => ENDPOINT.to_string(),
        }
    };
    let (hk, hv) = match &proxy {
        Some((_, token)) => ("x-proxy-token", token.clone()),
        None => ("x-api-key", api_key.clone().unwrap_or_default()),
    };
    let resp = state
        .http
        .post(&endpoint)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .header(hk, &hv)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("anthropic request failed: {e}"))?;
    let mut status = resp.status();
    let mut text = resp.text().await.map_err(|e| e.to_string())?;
    // DeepSeek: if it rejects the `thinking` field, retry once WITHOUT it so a
    // single unsupported param can't break every call.
    if deepseek && !status.is_success() && body.get("thinking").is_some() {
        let mut b2 = body.clone();
        if let Some(o) = b2.as_object_mut() {
            o.remove("thinking");
        }
        if let Ok(resp2) = state
            .http
            .post(&endpoint)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .header(hk, &hv)
            .json(&b2)
            .send()
            .await
        {
            status = resp2.status();
            text = resp2.text().await.unwrap_or_default();
        }
    }
    if !status.is_success() {
        return Err(format!("anthropic {status}: {text}"));
    }

    let json: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    // Concatenate ALL text blocks (a response may lead with thinking blocks, then
    // text). If none, surface the stop_reason so the failure is diagnosable rather
    // than a blank "no text block" (usually max_tokens spent on reasoning).
    let out: String = json
        .get("content")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    if out.trim().is_empty() {
        let stop = json.get("stop_reason").and_then(|v| v.as_str()).unwrap_or("unknown");
        return Err(format!(
            "model returned no text (stop_reason: {stop}). It likely spent the token budget reasoning — retrying with a larger budget."
        ));
    }
    let input_tokens = json.get("usage").and_then(|u| u.get("input_tokens")).and_then(|v| v.as_i64()).unwrap_or(0);
    let output_tokens = json.get("usage").and_then(|u| u.get("output_tokens")).and_then(|v| v.as_i64()).unwrap_or(0);
    Ok((out.to_string(), input_tokens, output_tokens))
}

/// One OpenAI (GPT) chat call — a second analysis angle. Returns (text, in, out).
pub async fn openai_call(
    state: &AppState,
    model: &str,
    system: &str,
    user: &str,
    max_tokens: i64,
) -> Result<(String, i64, i64), String> {
    let (api_key, proxy) = {
        let keys = state.keys.lock().map_err(|_| "keys lock")?;
        (keys.openai.clone(), keys.proxy())
    };
    if proxy.is_none() && api_key.is_none() {
        return Err("OpenAI key not set. Add it in Settings.".to_string());
    }
    let mut body = serde_json::json!({
        "model": model,
        "max_completion_tokens": max_tokens,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user }
        ]
    });
    // GPT-5 models are reasoning models: with the default effort they can burn the
    // whole token budget on hidden reasoning and return EMPTY content (the cause of
    // "model returned no JSON"). Cap the effort low so output tokens remain, and —
    // when the prompt clearly wants JSON — force a JSON object response.
    if model.starts_with("gpt-5") {
        body["reasoning_effort"] = serde_json::json!("low");
        if system.contains("JSON") || user.contains("Return ONLY this JSON") || user.contains("strict JSON") {
            body["response_format"] = serde_json::json!({ "type": "json_object" });
        }
    }
    let endpoint = match &proxy {
        Some((base, _)) => format!("{base}/openai/v1/chat/completions"),
        None => OPENAI_ENDPOINT.to_string(),
    };
    let mut req = state.http.post(&endpoint).header("content-type", "application/json");
    req = match &proxy {
        Some((_, token)) => req.header("x-proxy-token", token),
        None => req.header("authorization", format!("Bearer {}", api_key.unwrap_or_default())),
    };
    let resp = req.json(&body).send().await.map_err(|e| format!("openai request failed: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("openai {status}: {text}"));
    }
    let json: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let out = json
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|t| t.as_str())
        .ok_or_else(|| "no content in OpenAI response".to_string())?;
    let usage = json.get("usage");
    let input_tokens = usage.and_then(|u| u.get("prompt_tokens")).and_then(|v| v.as_i64()).unwrap_or(0);
    let output_tokens = usage.and_then(|u| u.get("completion_tokens")).and_then(|v| v.as_i64()).unwrap_or(0);
    Ok((out.to_string(), input_tokens, output_tokens))
}

/// Route an analysis call to the right provider by model id. DeepSeek exposes an
/// Anthropic-format endpoint, so it goes through `anthropic_call` (which detects
/// the deepseek id and swaps the endpoint + key).
pub async fn chat_call(
    state: &AppState,
    model: &str,
    system: &str,
    user: &str,
    max_tokens: i64,
) -> Result<(String, i64, i64), String> {
    if is_openai_model(model) {
        openai_call(state, model, system, user, max_tokens).await
    } else {
        anthropic_call(state, model, system, user, max_tokens).await
    }
}

const PLAUSIBILITY_SYSTEM: &str = r#"You are a sharp football analyst scoring how PLAUSIBLE each pre-computed bet line is for THIS specific match. This is NOT about value or odds — it's a real-world read: given the matchup, each player's role and likely minutes, probable tactics/formations, motivation and ROTATION risk, how realistic is this exact line? Score each line 1-5:
5 = highly plausible (key starter, line fits their role and the matchup — e.g. the main striker to score, a press-heavy side's winger for shots).
4 = plausible.
3 = neutral / ordinary.
2 = shaky (role mismatch, fringe player, matchup works against it — e.g. a scorer running into an elite, settled defence; a shots line vs a deep low block; a tackles line vs a possession side that allows no duels). Use 2 as the DE-RISK signal: the raw percentage looks fine but the matchup/role context argues against paying for it.
1 = implausible or a TRAP (likely benched/rotated/injured, wrong role, contradicted by how this game will be played).
Use your football knowledge of these specific teams and players — but ONLY what you actually know. If you do NOT recognize a player or team well enough to judge role and rotation risk, score 3 with reason "unknown to me" — NEVER invent a role, rotation pattern or recent-form claim for someone you can't place; a confidently wrong reason is worse than an honest 3. Do NOT invent probabilities or odds — qualitative judgement only. Give a 3-8 word reason per line. Output strict JSON only."#;

/// One Haiku call PER FIXTURE that scores every candidate line's real-world
/// plausibility (1-5 + short reason). Cached by (fixture + lines + context) hash
/// so rebuilds are free. Returns (scores, input_tokens, output_tokens); scores is
/// a list of (subject, market, line, score, reason).
pub async fn score_plausibility(
    state: &AppState,
    model: &str,
    fixture_label: &str,
    context: &str,
    lines_compact: &str,
) -> Result<(Vec<(String, String, String, u8, String)>, i64, i64), String> {
    let user = format!(
        r#"Match: {fixture_label}
Context (lineups/injuries/predictions/tactics — weigh rotation & roles): {context}

Candidate lines to score (copy subject/market/line back EXACTLY):
{lines_compact}

Output ONLY this JSON, one entry per line in the same order:
{{ "scores": [ {{ "subject": "...", "market": "...", "line": "...", "score": 1-5, "reason": "3-8 words" }} ] }}"#
    );
    let (text, gin, gout) =
        anthropic_call(state, model, PLAUSIBILITY_SYSTEM, &user, 4000).await?;
    let start = text.find('{').ok_or("no json")?;
    let end = text.rfind('}').ok_or("no json")?;
    let v: Value = serde_json::from_str(&text[start..=end]).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    if let Some(arr) = v.get("scores").and_then(|s| s.as_array()) {
        for s in arr {
            let subj = s.get("subject").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let market = s.get("market").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let line = s.get("line").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let score = s.get("score").and_then(|x| x.as_i64()).unwrap_or(3).clamp(1, 5) as u8;
            let reason = s.get("reason").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if !subj.is_empty() && !market.is_empty() {
                out.push((subj, market, line, score, reason));
            }
        }
    }
    Ok((out, gin, gout))
}

const INGEST_SYSTEM: &str = r#"You turn a raw web page's text into structured, betting-relevant data for ONE football fixture. Identify which match it is about (home & away team, date if shown, competition). PULL EVERY STATISTIC the page shows — be a careful data-scraper, not a summariser. Capture team numbers (corners for/against per game, shots & shots-on-target per game, cards & fouls per game, possession %, xG / xGA, goals for/against per game, clean-sheet %, form W-D-L) and key-player numbers (goals, assists, shots/SOT per game), plus any predicted score / 1X2 % / analyst angle. Be faithful — copy numbers exactly, NEVER invent them; the "summary" must only RESTATE what the page itself says (no extrapolation, no predictions of your own). Output strict JSON only."#;

/// Haiku-structure an ingested web page (its visible text) into fixture-tagged
/// JSON. Honours an optional user note ("extract only xyz"). Returns the JSON
/// string + token usage.
/// Strip the common token-busters from a page's text: collapse runs of
/// whitespace, drop blank-line spam and exact consecutive duplicate lines (nav /
/// menu / cookie boilerplate tends to repeat), so Haiku sees mostly real content.
fn clean_page_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last = String::new();
    let mut blanks = 0;
    for line in s.lines() {
        let t = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if t.is_empty() {
            blanks += 1;
            if blanks <= 1 {
                out.push('\n');
            }
            continue;
        }
        blanks = 0;
        if t == last {
            continue; // skip immediate duplicate lines
        }
        last = t.clone();
        out.push_str(&t);
        out.push('\n');
    }
    out
}

/// Budget-aware truncation for scraped pages: if the cleaned text exceeds the
/// cap, keep every line that carries a NUMBER (the stats we're extracting) plus
/// short heading-like lines, and only then trim. A blind head-truncate at 24k
/// chars paid for nav/prose and could cut the stats table off the bottom.
fn cap_page_text(cleaned: &str, cap: usize) -> String {
    if cleaned.len() <= cap {
        return cleaned.to_string();
    }
    let mut out = String::with_capacity(cap);
    for line in cleaned.lines() {
        let keep = line.chars().any(|c| c.is_ascii_digit()) || line.len() <= 60;
        if !keep {
            continue;
        }
        if out.len() + line.len() + 1 > cap {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    if out.is_empty() {
        cleaned.chars().take(cap).collect()
    } else {
        out
    }
}

pub async fn extract_ingest(
    state: &AppState,
    model: &str,
    page_text: &str,
    note: &str,
) -> Result<(String, i64, i64), String> {
    let text = cap_page_text(&clean_page_text(page_text), 16_000);
    let note_line = if note.trim().is_empty() {
        String::new()
    } else {
        format!("\n\nUSER INSTRUCTIONS — follow these for WHAT to extract: {note}")
    };
    let user = format!(
        r#"Page text (may be messy navigation + content):
{text}{note_line}

Return ONLY this JSON (omit unknown fields, keep values short):
{{ "home": "home team", "away": "away team", "date": "YYYY-MM-DD or ''", "league": "competition or ''", "summary": "2-3 sentence real-world betting read from this page", "lineups": [ {{ "team": "team name", "players": ["Full Name", "..."] }} ], "data": [ {{ "label": "...", "value": "..." }} ] }}

For "lineups": ONLY when the page clearly shows starting lineups or confirmed/probable XIs (SofaScore-style) — copy each side's 11 starter names EXACTLY as printed (no shirt numbers, no positions); omit the field entirely if the page has no lineups.

For "data": capture EVERY useful NUMBER, not just a few. Prefer STATS with a clear subject in the label, e.g. "Morocco corners/game 6.4", "Morocco corners against 4.1", "Netherlands shots on target/game 5.2", "Netherlands cards/game 1.8", "Morocco possession 47%", "Netherlands xG 1.7", "Morocco form WWDLW", "Hakimi shots/game 2.1", "Brobbey goals 4". Include team form, corners, shots/SOT, cards/fouls, possession, xG/xGA, goals for/against, and key players' goals/assists/shots — whatever the page shows. Also keep any predicted score, 1X2 %, or analyst note. Copy numbers EXACTLY; never invent."#
    );
    let (resp, gin, gout) = chat_call(state, model, INGEST_SYSTEM, &user, 3000).await?;
    let start = resp.find('{');
    let end = resp.rfind('}');
    match (start, end) {
        (Some(s), Some(e)) if e > s => Ok((resp[s..=e].to_string(), gin, gout)),
        _ => {
            let snip: String = resp.trim().chars().take(120).collect();
            Err(format!(
                "model returned no JSON (got {} chars: \"{}\"). Try a different model — gpt-5-nano can struggle here; Haiku is reliable.",
                resp.trim().len(),
                snip
            ))
        }
    }
}

const EVAL_SYSTEM: &str = r#"You are a sharp football betting analyst. Return CLEAR, STRUCTURED output — never a rambling paragraph. Reason about the ACTUAL match(es) — the specific teams/players, the competition (use the per-leg "competition" — a World Cup knockout is not a friendly), likely lineups and ROTATION, tactics/formation, motivation, referee, home/away and form — NOT just the supplied numbers.

For EACH ticket return exactly these fields:
- "verdict": one word — "Strong", "Fair" or "Thin".
- "analysis": ONE or TWO tight sentences — the core real-world read for the whole ticket (how the legs interact, the realistic chance it lands).
- "leg_notes": an array with ONE entry PER LEG, in order: { "leg": "<player/team + market, short>", "rating": "solid" | "ok" | "risky" | "trap", "note": "4-10 words why (will they start? does it fit the game?)" }.
- "risks": 1-3 short strings (the key things that sink it).
- "recommendations": 1-3 short ACTIONABLE changes ("drop the rested-star leg", "swap X SOT for anytime scorer", "trim to a double"); if it's already good, a single "leave as-is — well constructed".

RATING MUST BE CONSISTENT AND ANCHORED TO THE LEG'S PROBABILITY (the "est"/"pin" given): est ≥ 0.60 → "solid"; 0.45-0.60 → "ok"; 0.30-0.45 → "risky"; < 0.30 OR genuinely contradicted/trap → "trap". A given leg gets the SAME rating no matter which ticket it appears in — do not flip Hakimi "2+ shots" between "risky" and "won't happen" across tickets; its probability is fixed. Only deviate from the probability band when there's a HARD real-world reason (player ruled out, obvious rotation, a real trap) — and then say it in the note. You are HELPING the user bet, not talking them out of it: flag genuine traps, but never reflexively rate a decent-probability leg as risky.

Call out TRAPS explicitly (likely-rested star, public trap favourite, deceptively short price, nested/contradictory legs, or a bet the LIVE score has already settled) via a "trap" rating. KNOW YOUR LIMITS: if you do not genuinely recognize a player/team, do NOT invent lineups, rotation or form — rate by the probability band and note "no strong real-world read". Never state a stat (goals, cards, a form run) that was not supplied. Be concrete and concise. Output strict JSON only."#;

/// Evaluate a set of tickets with a (usually cheaper) model. Returns the
/// per-ticket analysis + token usage.
pub async fn evaluate(
    state: &AppState,
    model: &str,
    tickets_compact: &str,
) -> Result<(Vec<crate::models::TicketEval>, i64, i64), String> {
    let user = format!(
        r#"Tickets to evaluate (one entry per ticket, same order). Each leg shows the match (Team A vs Team B), the selection (player/team), market and line:
{tickets_compact}

Each leg carries its "competition" — USE IT (a World Cup knockout is not a friendly/qualifier). Give CLEAR STRUCTURED output, one evaluation per ticket in order, with a leg_notes entry for EVERY leg. Keep strings short so the JSON stays complete.
Output ONLY this JSON object — no markdown, no prose:
{{ "evaluations": [ {{
  "verdict": "Strong | Fair | Thin",
  "analysis": "1-2 sentences, the real-world read",
  "leg_notes": [ {{ "leg": "Player/team + market", "rating": "solid | ok | risky | trap", "note": "4-10 words" }} ],
  "risks": ["..."],
  "recommendations": ["..."]
}} ] }}"#
    );
    let (text, gin, gout) = chat_call(state, model, EVAL_SYSTEM, &user, 6000).await?;
    Ok((parse_evals(&text), gin, gout))
}

/// Parse the evaluations array robustly — tolerate markdown fences, leading
/// prose, and truncation (keep the complete entries).
fn parse_evals(text: &str) -> Vec<crate::models::TicketEval> {
    // Strict whole-object first.
    if let (Some(s), Some(e)) = (text.find('{'), text.rfind('}')) {
        if e > s {
            if let Ok(v) = serde_json::from_str::<Value>(&text[s..=e]) {
                if let Some(arr) = v
                    .get("evaluations")
                    .and_then(|x| serde_json::from_value::<Vec<crate::models::TicketEval>>(x.clone()).ok())
                {
                    return arr;
                }
            }
        }
    }
    // Salvage: scan the evaluations array, keep complete objects.
    if let Some(i) = text.find("\"evaluations\"") {
        if let Some(rel) = text[i..].find('[') {
            let arr_start = i + rel;
            let bytes = text.as_bytes();
            let (mut j, mut depth, mut in_str, mut esc, mut last) = (arr_start + 1, 0i32, false, false, None);
            while j < bytes.len() {
                let ch = bytes[j] as char;
                if in_str {
                    if esc {
                        esc = false;
                    } else if ch == '\\' {
                        esc = true;
                    } else if ch == '"' {
                        in_str = false;
                    }
                } else {
                    match ch {
                        '"' => in_str = true,
                        '{' | '[' => depth += 1,
                        '}' | ']' => {
                            depth -= 1;
                            if depth == 0 {
                                last = Some(j + 1);
                            } else if depth < 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                j += 1;
            }
            if let Some(end) = last {
                let elems = text[arr_start + 1..end].trim_end_matches([',', ' ', '\n', '\r', '\t']);
                if let Ok(v) = serde_json::from_str::<Vec<crate::models::TicketEval>>(&format!("[{elems}]")) {
                    return v;
                }
            }
        }
    }
    Vec::new()
}

/// Extract the JSON object from model text and deserialize it. If the JSON was
/// truncated (max_tokens), salvage as many COMPLETE tickets as possible.
fn parse_result(text: &str) -> Result<BuildResult, String> {
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            if end > start {
                if let Ok(mut r) = serde_json::from_str::<BuildResult>(&text[start..=end]) {
                    r.from_cache = false;
                    return Ok(r);
                }
            }
        }
    }
    // Salvage truncated output: keep complete ticket objects only.
    if let Some(mut r) = salvage(text) {
        if !r.tickets.is_empty() {
            r.from_cache = false;
            r.data_quality_notes
                .push("Note: the model's output was truncated; showing the complete tickets only.".to_string());
            return Ok(r);
        }
    }
    Err("EOF / unparseable JSON".to_string())
}

/// Rebuild `{"tickets":[...]}` from however many ticket objects fully closed
/// before the model ran out of tokens.
fn salvage(text: &str) -> Option<BuildResult> {
    let ti = text.find("\"tickets\"")?;
    let rel = text[ti..].find('[')?;
    let arr_start = ti + rel;
    let bytes = text.as_bytes();
    let mut i = arr_start + 1;
    let mut depth: i32 = 0;
    let mut in_str = false;
    let mut esc = false;
    let mut last_complete: Option<usize> = None;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        if in_str {
            if esc {
                esc = false;
            } else if ch == '\\' {
                esc = true;
            } else if ch == '"' {
                in_str = false;
            }
        } else {
            match ch {
                '"' => in_str = true,
                '{' | '[' => depth += 1,
                '}' | ']' => {
                    depth -= 1;
                    if depth == 0 {
                        last_complete = Some(i + 1); // end of a complete ticket object
                    } else if depth < 0 {
                        break; // reached the array's closing ']'
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    let end = last_complete?;
    let elements = text[arr_start + 1..end].trim_end_matches([',', ' ', '\n', '\r', '\t']);
    let json = format!("{{\"tickets\":[{elements}]}}");
    serde_json::from_str(&json).ok()
}
