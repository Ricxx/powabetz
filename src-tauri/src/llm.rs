//! Anthropic (Opus 4.8) integration. Exactly one call per Build. The caller
//! (commands.rs) handles the input-hash cache; this module builds the request,
//! calls `/v1/messages`, and validates strict JSON with a single stricter retry.

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::models::BuildResult;
use crate::AppState;

pub const DEFAULT_MODEL: &str = "claude-opus-4-8";
const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const OPENAI_ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";

/// Allowed model ids for the main BUILD (Claude only — the deterministic engine
/// feeds it; we keep the sharp model on the home provider).
pub fn is_allowed_model(m: &str) -> bool {
    matches!(m, "claude-opus-4-8" | "claude-sonnet-4-6" | "claude-haiku-4-5")
}

/// An OpenAI (GPT) model id.
pub fn is_openai_model(m: &str) -> bool {
    m.starts_with("gpt-")
}

/// Models allowed for the quick ANALYSIS (a second angle) — Claude + GPT.
pub fn is_allowed_analysis_model(m: &str) -> bool {
    is_allowed_model(m) || matches!(m, "gpt-5-nano" | "gpt-5-mini")
}

/// (input, output) USD price per 1M tokens. GPT prices are estimates. Unknown
/// Claude ids fall back to Opus.
pub fn model_pricing(model: &str) -> (f64, f64) {
    match model {
        "claude-sonnet-4-6" => (3.0, 15.0),
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

Your job: assemble ABOUT 10 TICKETS that lean toward VALUE and calculated longshots — NOT just safe bankers. Each ticket is one of:
- "Single": one leg.
- "SGP": 2-5 legs from the SAME fixture (same-game parlay).
- "SGP+": legs spanning MULTIPLE fixtures (or a bigger same-game build).

Prioritise legs with positive ev (price beats the true probability) — that is the exploitable edge. Legs without odds (most player props aren't priced) can still be used to build bet builders from est_prob. Make the slate a MIX: some +EV singles, several SGPs, and longshot bet builders.

BET BUILDERS (REQUIRED): include AT LEAST 3 multi-leg bet builders (SGP — same fixture, or SGP+ — across fixtures) of 3-5 legs each, deliberately chosen so the legs' est_prob values MULTIPLY to roughly a 10-20% combined hit chance (genuine longshots, ~5x-10x). Use each leg's est_prob to gauge this.

CORRELATED & THEMATIC SGPs (STRONGLY PREFERRED): the best same-game builders tell ONE story where the legs reinforce each other, so they hit TOGETHER more often than naive independence implies. Build at least one THEMED SGP, e.g.:
- A "goals" theme: a team to win + their key player to score + over 1.5 team goals + BTTS.
- A "cards/physical" theme: both teams to receive a card + a known booker to be carded + the dirtier team for most cards + over the match cards line — these all rise together in a heated/derby game.
- A "shots/attacking" theme: a high-volume side's two main shooters for shots + team shots over + team corners over.
Pick legs that move in the SAME direction (positively correlated). NEVER stack contradictory or mutually-exclusive legs, and never two nested lines for the same player (a goal implies a shot).

Rules: a player who hasn't scored recently is only a strong scorer pick when form_state="due_regression"; "cold_falling_off" means down-rank. Injured/suspended subjects cannot feature. If xg_source="proxy" or a leg carries a proxy/crude flag, lower confidence. NEVER stack nested/correlated legs for the SAME player in one ticket — a goal implies a shot on target which implies a shot, so pick only ONE of {anytime scorer, shots on target, player shots} per player (the others are redundant). Likewise never combine two lines of the same team goals/corners market.

CRITICAL for matching: in every leg, copy the row's "subject" verbatim into "selection" and the row's "market" verbatim into "market" (and its "line"). Do NOT put probabilities or odds in legs — those are filled in automatically afterwards. Treat predictions and the user's notes as soft context. Output strict JSON only."#;

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
    /// Max times a single player/team may appear across the slate (0 = model default).
    pub max_per_subject: u32,
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
        "EVERY ticket MUST be an SGP+ — 3-6 legs spanning AT LEAST TWO different fixtures (never all from one match, never a single). If too few fixtures are selected to make that many distinct SGP+ tickets, still produce as many genuinely different ones as possible."
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
        "bankers" => "BANKERS mode — build tickets from HIGH-LIKELIHOOD RECURRING events: things that happen most games for that player/team. Lead with picks that carry a 'hit N/M recent' note (we measured how often it actually happened lately) — a regular booker To Be Carded, a reliable shooter for 1+ shot, a high-volume passer over their passes line, a corner-heavy team's corners over. Prefer ~60%+ legs; combine 3-6 of them into bankable parlays. These are the 'this basically always happens' picks — favour reliability and recent consistency over price or flashiness, but DO say in the 'why' how often each leg has landed recently.",
        "power" => "POWER-STACKER mode — low-leg, high-conviction parlays: lottery-like payouts with FEWER things to connect. Build cross-game DOUBLES (occasionally a treble). Every leg must be a HIGH-LIKELIHOOD outcome that 'should happen' yet is still priced GENEROUSLY (~1.8-2.5) because the book is enticing action — a dominant favourite to win (~2.0), an in-form scorer the book shades (~2.2), a soft over. Stack TWO such legs so the COMBINED odds clear AT LEAST 4.0 (ideally 5-10x). You may pair ONE slightly-less-expected but still-likely leg (~2.5-5.0) with a near-certain ~2.0 leg to reach ~10x on something genuinely simple. Each leg from a DIFFERENT fixture; across the whole slate MAXIMISE diversity — different teams, players AND markets every ticket, never reuse the same selection. Lower variance is the point: do not over-stack. In each 'why', state the combined odds and explain why BOTH legs 'must happen'.",
        _ => "Lean value/longshot: prioritise +EV legs (best price beats the true probability) — that is the exploitable edge.",
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

Build EXACTLY {count} main tickets — hitting this exact COUNT is the #1 HARD requirement, above every preference below. {typerule}{minlegs} {strategy} Each ticket must differ from every other by at least one leg, but tickets MAY SHARE legs: if the candidate pool is thin, REUSE legs and subjects across tickets (re-combining them differently) to reach the full count — that is REQUIRED, not optional. Any FEELING LUCKY tickets requested below are ADDITIONAL, on top of these {count}. NEVER return fewer than {count} main tickets; if you cannot make them all maximally diverse, return them anyway with repeated legs.

DIVERSITY IS PREFERRED (but never at the cost of the count):
- Prefer to spread subjects around so one result can't sink the whole slate — ideally a single subject (player or team) appears in only a small share of tickets — but the exact ticket count ALWAYS wins: if respecting diversity would mean returning fewer than {count}, repeat subjects/legs instead and still return {count}.{diversitycap}
- Vary the leg combinations and odds ranges (some shorter banker-ish, some bigger longshots) so tickets aren't near-duplicates — never just swap one leg.
- Spread MARKETS too: don't lean on one market (e.g. all anytime-scorer). Mix scorer/SOT/goals/corners/result/etc. across the slate so no single market or subject is over-used.
{why}{lucky}{bias}{variation}

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

FINAL CHECK before you answer: count the objects in "tickets". It MUST equal {count} main tickets PLUS every Feeling-Lucky ticket requested. If it's short, ADD more by re-combining existing legs until the count is exact — do not stop early."#,
        markets = markets.join(", "),
        types = types,
        notes = notes_block,
        preds = pred_block,
        grok = grok_block,
        table = table,
        count = count,
        typerule = typerule,
        minlegs = minlegs_block,
        diversitycap = diversity_cap,
        strategy = strategy_block,
        why = why_clause,
        lucky = lucky_block,
        bias = bias_block,
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
    let api_key = {
        let keys = state.keys.lock().map_err(|_| "keys lock")?;
        keys.anthropic
            .clone()
            .ok_or_else(|| "Anthropic key not set. Add it in Settings.".to_string())?
    };

    let prompt = user_prompt(table, markets, reasoning, notes, predictions, grok, opts);

    // Scale the output budget to the requested slate size so the JSON isn't cut off.
    let count = if opts.count == 0 { 10 } else { opts.count } as i64;
    let max_tokens = (2400 + count * 540 + opts.lucky_total() as i64 * 560).clamp(3000, 16000);

    // First attempt — strict parse, then salvage a truncated slate.
    let (text, in1, out1) = request_text(state, model, &api_key, &prompt, max_tokens).await?;
    if let Ok(parsed) = parse_result(&text) {
        return Ok(ModelCall { result: parsed, input_tokens: in1, output_tokens: out1 });
    }

    // Retry once, bigger budget + stricter nudge.
    let stricter = format!(
        "{prompt}\n\nIMPORTANT: Output ONLY the JSON object, starting with {{ and ending with }}. No markdown, no commentary. Keep each 'why' to one short sentence so the JSON is complete."
    );
    let (text2, in2, out2) =
        request_text(state, model, &api_key, &stricter, (max_tokens + 2000).min(12000)).await?;
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
    _api_key: &str,
    prompt: &str,
    max_tokens: i64,
) -> Result<(String, i64, i64), String> {
    anthropic_call(state, model, SYSTEM_PROMPT, prompt, max_tokens).await
}

/// Generic Anthropic call → (text, input_tokens, output_tokens). Fetches the key.
pub async fn anthropic_call(
    state: &AppState,
    model: &str,
    system: &str,
    user: &str,
    max_tokens: i64,
) -> Result<(String, i64, i64), String> {
    let (api_key, proxy) = {
        let keys = state.keys.lock().map_err(|_| "keys lock")?;
        (keys.anthropic.clone(), keys.proxy())
    };
    if proxy.is_none() && api_key.is_none() {
        return Err("Anthropic key not set. Add it in Settings.".to_string());
    }
    let body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "system": system,
        "messages": [{ "role": "user", "content": user }]
    });

    let endpoint = match &proxy {
        Some((base, _)) => format!("{base}/anthropic/v1/messages"),
        None => ENDPOINT.to_string(),
    };
    let mut req = state
        .http
        .post(&endpoint)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json");
    req = match &proxy {
        Some((_, token)) => req.header("x-proxy-token", token),
        None => req.header("x-api-key", api_key.unwrap_or_default()),
    };
    let resp = req.json(&body)
        .send()
        .await
        .map_err(|e| format!("anthropic request failed: {e}"))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("anthropic {status}: {text}"));
    }

    let json: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let out = json
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.iter().find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text")))
        .and_then(|b| b.get("text"))
        .and_then(|t| t.as_str())
        .ok_or_else(|| "no text block in model response".to_string())?;
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
    let body = serde_json::json!({
        "model": model,
        "max_completion_tokens": max_tokens,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user }
        ]
    });
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

/// Route an analysis call to the right provider by model id.
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
2 = shaky (role mismatch, fringe player, matchup works against it).
1 = implausible or a TRAP (likely benched/rotated/injured, wrong role, contradicted by how this game will be played).
Use your football knowledge of these specific teams and players. Do NOT invent probabilities or odds — qualitative judgement only. Give a 3-8 word reason per line. Output strict JSON only."#;

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

const INGEST_SYSTEM: &str = r#"You turn a raw web page's text into structured, betting-relevant data for ONE football fixture. Identify which match it is about (home & away team, date if shown, competition) and pull the useful facts and any analyst read. Be faithful to the page — do NOT invent numbers. Output strict JSON only."#;

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

pub async fn extract_ingest(
    state: &AppState,
    model: &str,
    page_text: &str,
    note: &str,
) -> Result<(String, i64, i64), String> {
    let text: String = clean_page_text(page_text).chars().take(24_000).collect();
    let note_line = if note.trim().is_empty() {
        String::new()
    } else {
        format!("\n\nUSER INSTRUCTIONS — follow these for WHAT to extract: {note}")
    };
    let user = format!(
        r#"Page text (may be messy navigation + content):
{text}{note_line}

Return ONLY this JSON (omit unknown fields, keep values short):
{{ "home": "home team", "away": "away team", "date": "YYYY-MM-DD or ''", "league": "competition or ''", "summary": "2-3 sentence real-world betting read from this page", "data": [ {{ "label": "e.g. Forebet 1X2 / predicted score / analyst note", "value": "the value or quote" }} ] }}"#
    );
    let (resp, gin, gout) = chat_call(state, model, INGEST_SYSTEM, &user, 1500).await?;
    let start = resp.find('{').ok_or("model returned no JSON")?;
    let end = resp.rfind('}').ok_or("model returned no JSON")?;
    Ok((resp[start..=end].to_string(), gin, gout))
}

const EVAL_SYSTEM: &str = r#"You are a sharp football betting analyst. Return CLEAR, STRUCTURED output — never a rambling paragraph. Reason about the ACTUAL match(es) — the specific teams/players, the competition (use the per-leg "competition" — a World Cup knockout is not a friendly), likely lineups and ROTATION, tactics/formation, motivation, referee, home/away and form — NOT just the supplied numbers.

For EACH ticket return exactly these fields:
- "verdict": one word — "Strong", "Fair" or "Thin".
- "analysis": ONE or TWO tight sentences — the core real-world read for the whole ticket (how the legs interact, the realistic chance it lands).
- "leg_notes": an array with ONE entry PER LEG, in order: { "leg": "<player/team + market, short>", "rating": "solid" | "ok" | "risky" | "trap", "note": "4-10 words why (will they start? does it fit the game?)" }.
- "risks": 1-3 short strings (the key things that sink it).
- "recommendations": 1-3 short ACTIONABLE changes ("drop the rested-star leg", "swap X SOT for anytime scorer", "trim to a double"); if it's already good, a single "leave as-is — well constructed".

Call out TRAPS explicitly (likely-rested star, public trap favourite, deceptively short price, nested/contradictory legs) via a "trap" rating. Be concrete and concise. Output strict JSON only."#;

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
