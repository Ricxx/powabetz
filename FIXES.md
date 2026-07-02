# powabet — audit fixes (July 2026)

Source: full-codebase audit (engine read + backend/frontend agent audits).
Principle: **fix the measurement layer first** — until settlement, EV and calibration
are honest, no strategy can be evaluated and no tuning is meaningful.

Status legend: `[ ]` pending · `[x]` done · `[~]` partial / follow-up noted

---

## P0 — Measurement integrity (the feedback loop)

- [x] **P0.1 Settlement grading** (`settle.rs`, `models.rs`)
  - 90-minute score (`score.fulltime`) instead of `goals` (which includes ET for
    AET/PEN). Player-prop details carry an "(incl. ET)" tag on AET games since the
    API can't split player stats at 90'.
  - `LegResult.void` added. Player didn't feature → void; `PST/CANC/ABD/AWD/WO`
    → all legs void, no more fetches for that fixture.
  - Unambiguous player lookup (exact → unique containment → unique surname;
    ambiguity → ungraded "grade manually").
  - "Match Result" grades a Draw selection explicitly.
- [x] **P0.2 Settle fetch storm** (`settle.rs`, `commands.rs`)
  - Events now `fetch_priority` (cache-first, budget-exempt) instead of
    `fetch_live` (always-network).
  - `ResultCache` shared across `settle_all` / `settle_generated`: N tickets on
    one fixture = 1 fetch. Forced re-fetches store with the week TTL once final.
  - Terminal statuses short-circuit before any player/stats/events fetch.
- [x] **P0.3 Money settlement** (`commands.rs`)
  - Winning ticket with unknown odds stays OPEN (legs graded green — the UI can
    prompt for odds) instead of settling break-even as "won".
  - Void-aware: void legs drop out at odds 1.0 (payout recomputed from surviving
    legs' book odds); all-void ticket → status "void", stake refunded.
- [x] **P0.4 Probability/EV biases** (`odds.rs`, `features.rs`)
  - Scorer probs get a flat 7% Pinnacle prop-margin haircut (one-sided,
    multi-winner market — sum-to-1 de-vig impossible; documented estimate).
  - Scorer lambda: `0.7·goals_p90 + 0.3·(0.18·sot_p90)` blend replaces the
    upward-only `max()`.
  - Passes variance: φ=6 overdispersion; line set at −0.75 sd of the honest spread.
- [x] **P0.5 Calibration on raw probs** (`models.rs`, `commands.rs`)
  - `raw_prob` stored on Candidate + TicketLeg at shrink time; both calibration
    pair readers use `raw_prob.or(est_prob)` and skip void legs.
- [x] **P0.6 Request meter honesty** (`apifootball.rs`, `CLAUDE.md`)
  - Meter counts attempts (increment before the request, inside the gate's lock
    scope — race closed); each rate-limit retry counts too.
  - CLAUDE.md hard rules updated to describe the real budget + LLM-call reality.
- [x] **P0.7 No silent degradation** (`commands.rs`)
  - Injury/odds/player-stats fetch failures during a build push "DEGRADED — …"
    entries into data_quality_notes with a ⚠ header line.

## P1 — UX correctness

- [x] **P1.1 Scout evening blackout** — archive cutoff is now UTC *yesterday*
  (pages stay live through their match day in every timezone; they linger one
  harmless extra day).
- [x] **P1.2 Double-tap duplicate bets** — synchronous `placingRef` guard on all
  four Place paths (Results, PicksBoard, CustomSlip, Live).
- [x] **P1.3 Simple-mode regenerate** — `newSet()` replays the last build's kind
  (Simple stays Simple, a ladder re-ladders via "add more"); hidden Advanced
  knobs (`max_leg_prob`, `bias_builders`, `min_plausibility`, `use_*`) forced to
  defaults in Simple builds.
- [x] **P1.4 Results forward navigation** — the 4th step dot is jumpable whenever
  a result exists.
- [x] **P1.5 Honesty flags on PicksBoard** — non-`style:` candidate flags render
  as badges plus an explicit `proxy xG` tag; `void` shown in Tracker (∅ mark,
  "void" status badge).

## P2 — Structure

- [x] **P2.1 Stratified shortlist** (`features.rs`, `commands.rs`)
  - Probability-band quotas (≥0.60 / 0.40–0.60 / 0.15–0.40 at 50/30/20% of n),
    filled in strategy-score order, spill by global score; final order by
    strategy score. Jackpot-grade plausible longshots now reach the model.
  - "Model saw X of Y candidate legs" data-quality note (no silent caps).
- [x] **P2.2 LLM layer** (`llm.rs`)
  - Refine budget scales with slate size (was flat 4000 → silently shipped the
    draft while billing the refine model).
  - Retry budget now strictly larger than attempt one (was capped *below* it for
    big slates).
  - Count-above-quality mandate removed: "up to {count}, quality beats count,
    return fewer + say why"; final check now verifies coherence, not count.
  - System prompt: "~10 tickets" and "≥3 builders REQUIRED" softened to defer to
    the requested strategy/types.
- [x] **P2.3 Referee multiplier removed** — referee stays soft context in
  pred_notes; the LLM-invented scalar no longer mutates est_prob (and the
  "4 or 5"→45 parse bug goes with it).
- [x] **P2.4 Align candidate pipelines** — `gather_candidates` (Picks/Bankers
  boards, ladder) now applies the calibration shrink (+raw_prob), filters to
  confirmed starters ("starting" availability), and applies xG in its form
  closure — the same leg now shows the same probability on every screen. (Boards
  deliberately keep top-24 players vs builds' 16 for browsing breadth.)
- [ ] **P2.5 Strategy consolidation** *(product decision — recommended)*: track
  Value(+Oracle), Scout, Bankers; Power → a Value "doubles" preset; Predictor
  stays a feature; Favorites/Likely/Jackpot cut or demoted to entertainment.

## P3 — Hygiene

- [x] Ingest truncate walks back to a char boundary (was a UTF-8 panic that
  silently killed the ingest server).
- [x] `ingeststats::sane()` skips out-of-band scraped values instead of clamping
  (a clamped number is no longer "from ingested stats").
- [x] IngestedStats requires BOTH team names to match a fixture.
- [x] DB: startup pruning (expired cache rows >1 day, ai_results >60 days) +
  `idx_gen_settled` index.
- [x] `lib.rs` throttle uses `checked_sub` (Instant underflow panic on fresh boot).
- [x] Frontend dedup: one `classifyTicket` (backend-matching SGP+/Acca rule),
  one `stratLabel`, one `pct()` (with "<1%" + 1-decimal-under-5% display) in
  types.ts; stale copy fixed ("one request per day" → real request count, dead
  "Players step" instruction).

## Live-match path (second-pass audit)

- [x] `live_adjust_forecast` no longer treats a missing/unparseable kickoff date
  as "live" (`unwrap_or(true)` → `false`) — that fired a fresh, budget-exempt
  4-request live snapshot per fixture for matches possibly not started.
- [x] Live odds "implied" probabilities get a documented 10% in-play margin
  haircut — they were raw 1/odds despite the doc comment claiming de-vig, and
  live margins are the book's fattest. Affects the live menu, edge flags, and
  placed live tickets' probs.
- [x] Live match-predict is now `forecast_only`: the backend returns the
  deterministic forecast with ZERO model tokens (it used to run a full — often
  premium — ticket build and discard the tickets).

## DeepSeek deterministic budget

- [x] Ingest extraction input cut 24k → 16k chars with stats-aware truncation
  (over the cap, lines carrying numbers + short headings are kept first — a
  blind head-truncate paid for nav/prose and could cut the stats table off the
  page bottom).
- Remaining (accepted) spend: the two-stage draft (scout/simple + premium final)
  sends the full table+pages to DeepSeek (~$0.005-0.01/build at $0.30/M in) and
  the refine model sees the table again — cheap in dollars; revisit only if
  latency hurts.

## Round 3 — deferred items built + the Apex strategy

- [x] **CLV tracking** — at settlement each priced leg's placed odds are compared
  to the closing price (post-match `/odds` re-fetch ≈ last pre-kickoff update;
  cache-first, best-effort). Avg CLV stored per bet (`placed_bets.clv` +
  `clv_json` detail), shown as a badge per bet and an avg-CLV card in the
  Tracker. Beating the close consistently = proof of edge in dozens of bets,
  not hundreds.
- [x] **Per-market calibration report** — now measures the RAW engine prob
  (pre-shrink) and skips void legs, so it grades the engine per market rather
  than the calibration layered on top.
- [x] **Banker definitions reconciled** — the shortlist "bankers" arm now calls
  `banker_score` (the Bankers board's ranking); one definition everywhere.
- [x] **Grok daily spend cap** — $2/day (const in grok.rs), checked against the
  `grok_usage` table before fresh searches; cached digests unaffected.
- [x] **Tracker add-odds flow** — an all-green open bet with no recorded price
  shows "add the ticket odds to settle" (new `set_bet_odds` command; validates,
  re-settles at the real price).
- [x] **APEX strategy 🎯** — the composite of the three evidence-backed edges:
  1. *Sharp-edge singles*: `ev_source=sharp` and EV ≥ +2% (price beats
     Pinnacle's de-vigged truth) with model↔market agreement as the trap filter.
  2. *Correlation-hunted SGPs*: `apex_combo_block` searches each fixture's
     priced legs (top 10 by fair prob; pairs + triples; no nested subjects/
     markets; odds 1.30–3.60; |est−pin| < 0.10) and prices every combo with the
     Monte-Carlo copula — kept only when lift ≥ ×1.08 AND
     `product_odds × correlated_prob − 1 ≥ +2%` (top 3/fixture, top 8 overall).
     The combos are injected pre-priced into the prompt; the model copies them
     verbatim (deterministic selection, LLM explains).
  3. *Longshot-bias guard*: legs over 3.6 excluded (overpriced tails).
  A thin slate is the strategy working — Apex passes rather than pads. Judge it
  by CLV. New shortlist arm, prompt block (`PromptOpts.apex_combos`, hashed),
  UI chip + description, ledger label.

## Round 4 — multi-fixture scale, timezone, proxy, leftovers

**The 10-fixture problem** (fixed): the pool was a FIXED size per strategy, so 10
fixtures fought for the same 50–90 table rows while per-match context grew
linearly — starved table, drowned model.
- [x] Pool scales with fixture count (+10 rows/fixture past 4, cap 220) and the
  per-market cap widens (so "the 10 best shooters" can all be SOT legs).
- [x] `shortlist` gained a **per-fixture cap** (~2.5× fair share) so data-rich
  games can't crowd the rest out of the table; noted in data-quality.
- [x] **Lean context** past 6 fixtures: weather/H2H/tactics skipped (requests +
  tokens saved); predictions/standings/lineups/injuries kept; noted.
- [x] **Scout at scale**: full ingested pages only injected up to 4 fixtures;
  bigger slates get the tight per-page digest (still fused).
- [x] **Ladder "one leg per match" toggle** — the deterministic answer to the
  cross-game acca: select 10 matches + one market (e.g. SOT), min prob ~60%,
  toggle on → the 10 best independent legs, one per game, product-priced,
  **0 model tokens**. This is the cheapest and best path for that ticket shape;
  the model build path is now viable for 10 fixtures too, but the ladder is free.

**Timezone** (fixed): `keys.timezone` now applies everywhere user-facing via
`local_today()` (chrono-tz): placed-bet day, generated-ledger day, Grok digest
date, ingest archive cutoff (exact local-today now, replacing the UTC-yesterday
approximation). The REQUEST METER intentionally stays UTC — API-Football's
quota resets 00:00 UTC (documented in code).

**Leftovers from earlier rounds** (fixed):
- [x] Grok cache key includes `live` + categories (a pre-match digest can no
  longer be served mid-game; changing categories fetches fresh).
- [x] Prewarm wedge: changing fixtures mid-prewarm now re-runs for the new
  slate when the current run finishes (`prewarmBusy` in the effect deps).
- [x] Voided subjects now feed REGULAR rebuilds (`exclude_subjects` on
  BuildSelection), not just ladders.

**Proxy / DeepSeek** (fixed): the Cloudflare worker had NO DeepSeek route and
the app never proxied DeepSeek — Server-mode installs couldn't use it at all.
- [x] `/deepseek/` route added to `proxy/worker.js` (x-api-key auth, secret
  `DEEPSEEK_KEY`); Rust routes DeepSeek via the proxy when no local key is set
  (a local key still wins). **Requires `wrangler secret put DEEPSEEK_KEY` and a
  `wrangler deploy`** (the route is code, not config).

## Round 5 — closing snapshots, form-gap, and 🧬 Darwin

- [x] **At-kickoff CLV snapshots** — a background loop (10-min tick, lib.rs)
  watches OPEN bets' fixtures; within [KO−20m, KO+10m] it snapshots `/odds`
  once into the cache (marker in ai_results), so `capture_clv` later reads a
  TRUE closing line instead of the post-match approximation. Best-effort —
  only runs while the app is open.
- [x] **Form-gap signal** (`features.rs`) — when a player's recent hit-rate
  exceeds the season-implied probability by ≥20pts over 4+ games, the leg is
  flagged "form-gap: … book may lag". This is the role-change edge: books
  price props off season averages and are slow to reprice volume shifts.
  Visible on boards/prompts; fuels Darwin's `dw:formgap`.
- [x] **🧬 DARWIN** — the creative strategy: a POPULATION of deterministic
  micro-strategies paper-trades every swept slate at ZERO token cost into the
  generated ledger (`dw:*` strategies), which auto-settles and reports — a
  survival-of-the-fittest leaderboard. Variants (one narrow hypothesis each):
  `dw:sharp2` / `dw:sharp5` (which sharp-EV bar survives the vig?),
  `dw:formgap` (role-change lag), `dw:lineroom` (margin mining: overs on
  markets whose settled history clears the line by ≥0.75 on 8+ samples),
  `dw:corrlift` (top copula combos as tickets), `dw:shooters` (one-per-match
  SOT acca), `dw:chalk3` (favourite-longshot bias played from the short side),
  `dw:contra-under` (our model under-read vs the sharp line). Sweep button on
  the markets step; idempotent per day (gen_add dedups); Ledger shows 🧬 rows.
  Promote a variant to real stakes only after its paper CLV/ROI earns it.

## Round 6 — input-token diet, plausibility cache, ladder polish

- [x] **Weather off by default** (backend `unwrap_or(false)`, frontend default,
  Simple no longer forces it) — low-value token clutter; Grok's news digest
  mentions weather when it actually matters. Toggle still in Advanced.
- [x] **Grok text-only guard** — the system prompt now explicitly ignores
  image/graphic posts (prediction cards, bet-slip screenshots — unreadable) and
  the PREDICTIONS category asks for TEXT predictions only. NEWS no longer asks
  about weather.
- [x] **Fixture-count copy**: "2–4 works best" → "up to ~10"; the warning now
  triggers past 10 and points at the one-per-match ladder for bigger accas.
- [x] **Plausibility cache fixed** — the per-line cache key included the LINE
  LABEL, which the engine picks dynamically from live data ("1+ SOT" flips to
  "2+ SOT" as rates move), so every re-selection re-scored already-prewarmed
  fixtures. Key is now (fixture, subject, market) — threshold-agnostic, one
  score per fixture that sticks. (Also closes the old Over/Under double-apply
  concern: plausibility is a subject/role read, symmetric across thresholds.)
- [x] **Ladder ranking sharpened** — legs now rank by the sharp-blended
  probability (½ Pinnacle de-vig + ½ our est) where priced, instead of raw
  est_prob.
- [x] **Ladder scope defaults to mixed and the MARKETS win** — scope is now an
  optional narrowing; if it conflicts with the selected markets (e.g. scope
  "team" with only player props picked) the ladder auto-falls back to the full
  selection with a note instead of erroring.
- [x] Toggle labels de-bracketed ("One leg per match", "Reset diversity on
  'Add more'") — the parentheticals broke the layout.

## Remaining deferred

- P2.5 strategy consolidation (product decision) — Apex + Darwin now cover the
  recommended ground; cutting Favorites/Likely is still advised.
- Darwin v2 ideas: auto-sweep on every build; fitness = calibration-weighted
  CLV once paper CLV capture exists; parameter mutation (spawn variants of the
  current champion with jittered thresholds).
## Round 7 — plausibility context + the context-waste sweep

- [x] **Plausibility v2** — the scorer now receives a CACHE-ONLY context per
  fixture: confirmed XI (+formation) and the injury list, read via `peek`
  (never touches the network, 0 requests). A context-presence marker joins the
  cache key, so lines re-score exactly ONCE when lineups post — trap detection
  is mostly a lineups question, and it was scoring blind before.
- [x] **All tweets text-only** — the Grok prompt's TEXT ONLY rule applies
  globally (ignore prediction cards, bet-slip screenshots, stat graphics —
  never guess their contents); PREDICTIONS asks for text predictions only.
  The dead legacy `SYSTEM` const removed.
- [x] **Context-waste sweep** (these rode into the table ×150 rows or the
  notes every build):
  - Referee note cut from a 40-word "use your knowledge of this ref" pep talk
    (which invited hallucinated card tendencies) to just the name.
  - Verbose repeated flags shortened: "workload proxied from home/away (no
    odds)" → "workload proxy"; "in-form: among the league's top scorers/…" →
    "in-form (league top scorer/assister)"; "team line: crude season-rate
    proxy" → "season-rate proxy"; ingest flag → "ingested stats (independent
    source)".
  - Bonus bug found during the sweep: `oracle_score`'s "aerial threat"
    conviction bonus NEVER fired — it scanned flags but the note lives in
    support. Now scans both.

## Round 8 — hallucination sweep (every LLM touchpoint audited)

- [x] **Ticket "why" fields** (build + refine prompts) — the biggest vector:
  whys freely invented stats ("scored 5 in his last 3"), form runs and odds.
  Both prompts now require citing ONLY facts shown in the table/context; with
  nothing to cite, describe the construction instead.
- [x] **Plausibility scorer** — was told to "use your football knowledge" with
  no escape hatch, so unknown players got confidently invented roles/rotation.
  Now: unrecognized player/team → score 3, reason "unknown to me" ("a
  confidently wrong reason is worse than an honest 3").
- [x] **Ticket evaluator** — same unknown-player rule + "never state a stat
  that wasn't supplied".
- [x] **Grok UNAVAILABLE line** — the highest-stakes vector: this line HARD-
  REMOVES players from the candidate pool, so one hallucinated name kills real
  picks. Now requires a specific recent report that explicitly confirms;
  any doubt → leave the name out.
- [x] **Tactics profiler** — described ANY team confidently, including sides
  the model can't know (fabricated styles for obscure teams, cached 14 days).
  Now has an explicit "STYLE: unknown" escape, treated as no-data.
- [x] **Ingest summary** — must restate the page only; no extrapolated
  predictions of its own (the data-array rule was already strict).
- [x] **Live ticket whys** — cite only menu numbers and supplied live stats.
- [x] **`price_sgp` coin-flip** — a leg with no probability was silently priced
  at 0.5, fabricating the combined price. Now refuses with an explicit error
  (UI already treats it as "no SGP price").
- Previously closed in earlier rounds, part of the same class: the referee LLM
  multiplier (invented cards/game mutating est_prob) and the referee pep-talk
  note (invited invented ref tendencies); Grok text-only (no guessing image
  contents); "4 or 5"→45 parse bug.
- **Accepted residual risk** (documented, by design): Grok can still relay a
  WRONG report from X (mitigated by the "unconfirmed" rule + veto tightening —
  inherent to news); plausibility/eval remain qualitative opinions, bounded to
  a ±0.12 rank nudge and advisory text, never probabilities.

## Round 9 — Scout mode deep review

Findings (fixed):
- [x] **One-team fixture matching in the BUILD path** (both the candidate
  builder and the notes injection) — the bug fixed on the display board in
  round 3 still lived in the backend: an "Arsenal vs Chelsea" page fed stats
  and candidates into "Chelsea vs Liverpool". Both sites now require BOTH
  teams to match.
- [x] **Team-name normalization** — matching was exact-fold containment, so a
  page labelled with any spelling variant silently failed to pair with the API
  fixture ("Scout needs ingested data" despite a valid page). New
  `odds::team_match`: full-name containment → distinctive tokens (≥4 chars,
  generic suffixes like "united"/"city" excluded to prevent cross-fixture
  hits) → reverse-prefix ("Barca"→"Barcelona"). Used in the build path, the
  page-data attribution, and mirrored on the IngestedStats board so display
  and build agree.
- [x] **No-match error is now diagnosable** — it lists the processed page
  labels so a name mismatch is visible instead of a dead end.
- [x] **Reground line-collision** — engine and scout rows share subject+market
  with different lines/probs; reground matched (subject, market) first-wins,
  silently swapping the model's chosen (often ingested) line for the ENGINE's
  row — the user's edge got replaced. Exact (subject, market, line) now wins.
- [x] **Player stats from pages were completely unused** — the extraction
  captures "Hakimi shots/game 2.1" but `parse()` required a team name in the
  label, so every player entry was dropped (prompt-context only). ingeststats
  now parses per-game player rates (shots / SOT / goals, sanity-banded,
  per-game only — totals are useless without appearances) and emits real
  Player Shots / SOT / Anytime Scorer candidates, NB-priced with the engine's
  own dispersion constants, flagged ingest-sourced + "team unknown".

Judged fine as-is: the separate-pool design (engine + ingest rows both in the
table, both source-flagged, the model arbitrates); `sane()` skip-don't-clamp;
the scout 220-row pool (per-fixture caps now bound it); full-page injection
≤4 fixtures / digest beyond; team-SOT skipped (no settleable market).

## Round 10 — Ingest pipeline deep review (extension → server → extraction → matching → consumers)

Findings (fixed):
- [x] **`fixture_id` was never populated** — the column existed, every consumer
  matched by label STRINGS forever, and the page label is whatever spelling the
  page used. Now SELF-HEALING: the first build that token-matches a page to a
  selected fixture writes the resolved id back (`ingest_resolve_fixture`);
  every consumer (scout candidates, notes injection, live ticket) matches by
  id FIRST, tokens second. A manual reassignment (the existing
  `ingest_set_fixture` label editor) clears the stale auto-id so re-resolution
  happens against the new label.
- [x] **THIRD one-team matching site found** — the live-ticket ingest notes
  (`fl.contains(hf) || fl.contains(af)`) still fed wrong-fixture pages into
  in-play prompts. Now id-first + both-team token matching like the rest.
- [x] **SQLite dual-connection locking (round-1 audit 8.2, finally fixed)** —
  the app and the ingest server thread each hold a connection with no WAL and
  no busy timeout; an extension POST landing during a build's cache writes
  failed with a raw "database is locked". Both connections now set
  `journal_mode=WAL` + `busy_timeout=3000`.
- [x] **Poll vs undo resurrection (round-1 audit 1.6, finally fixed)** — the
  5s list poll re-added an optimistically-deleted page mid-undo-window. The
  poll now filters pending-delete ids; undo/completion clears the marker.

Verified sound, left alone:
- The extension: main-content-region capture (skips nav token-busters), token
  auth, badge feedback; server binds 127.0.0.1 only, CORS `*` is safe because
  the token gates writes.
- The upsert lifecycle: re-ingesting a URL refreshes content AND resets status
  to `new`, so stale extractions can't survive a content change.
- Extraction truncation (stats-aware 16k, char-boundary safe — earlier rounds).
- The manual fixture-reassignment path (label + date patch-through into the
  extracted JSON) — a good escape hatch; now interoperates with auto-resolution.

## Round 11 — Ledger review (tracking accuracy + usefulness)

Findings (fixed):
- [x] **Void tickets counted as losses** — all-void (pushed) tickets were
  settled `won=false` and the SQL report counted them in settled/hit/ROI,
  dragging every strategy's numbers down. New `voided` column set at settle;
  both reports (by strategy, by kind) exclude pushes from hit/ROI and the UI
  shows them as "+N∅".
- [x] **No per-strategy calibration** — the single most useful missing metric:
  each settled ticket carries its own predicted combined hit chance, and the
  report now shows avg `pred X%` under the actual hit rate (amber when the gap
  exceeds 10pts). This answers "is Jackpot actually hitting the ~3% it
  claims?" per strategy — the honesty check that hit-rate alone can't give.
- [x] **Lifetime-only aggregation** — edges decay; months-old results buried
  what's working NOW. The strategy report takes a `since_days` window with a
  7d / 30d / All toggle in the Ledger header.
- [x] **Small-sample noise sorted to the top** — a 3-ticket 100%-hit strategy
  ranked above a 40-ticket +8% one. Sorting now tiers by sample size (10+ /
  4+ / fewer) before ROI, and settled<10 rows carry a ⚠ "treat as noise"
  marker. This also makes the Darwin leaderboard honest — a variant can't
  look like a champion off three lucky tickets.

Judged sound: ROI's flat-1-unit-per-priced-ticket basis; the per-market table
(predicted vs actual + margins + near-misses — already strong); the AI-spend
breakdown; settle dedup via day+strategy+sig.

## Round 12 — trivial-leg policy (near-certainties are negative value)

The economics: a 1.02–1.10 leg adds ≤10% payout while its REAL failure chance
in a parlay is comparable — risk with no return. And its "wins" carry no
signal: "Goals Range 1-6" lands in ~90% of games, so any strategy stacking it
looks stellar in the ledger while predicting nothing.

- [x] **Always-on trivial-leg filter** in builds: legs with est > 93% or
  priced ≤ 1.10 are dropped before the model/shortlist/apex, with a
  data-quality note counting them. The user's Safety-ceiling slider still
  narrows further; its "off" label now reads "default (near-certainties >93%
  always dropped)".
- [x] **Same policy in the Acca Ladder** (a rung at 1.05 pads hit% for
  nothing) and in **Darwin** (a paper variant padded with near-certainties
  would fake exactly the ledger pollution Darwin exists to avoid).
- [x] **"Goals Range 1-6" no longer generated** — the worst offender priced
  nothing worth knowing (the filter would drop it anyway; now it doesn't even
  exist). Other wide bands stay and are caught by the probability filter when
  trivial.
- [x] **Prompt insurance**: the builder prompt now states that a near-certainty
  never earns a parlay slot ("risk without return; every leg must EARN its
  place with meaningful odds") — covers unpriced legs the odds test can't see.
- [x] **Forecasts keep the full distribution** — dropped trivial legs are
  retained for the Match Predictor / Simple forecast panels (a 95% favourite
  belongs in a "likely result" display even though it's a worthless bet leg).

## Round 13 — Data viewer UX + data-clarity review

- [x] **One panel, four tabs, one close** — the Data viewer was three
  half-joined systems: `boardMode` (all/bankers) fighting `dataTab`
  (picks/inspector/ingested), Bankers hiding the tab bar entirely, every child
  owning a Done button that killed the whole panel, and the Inspector being a
  separate slide-in drawer whose BACKDROP click also closed everything. Now:
  a single sticky header (📊 Data · Picks | Bankers | Inspector | Ingested ·
  ✕ Done); tabs switch freely and never close; only ✕ closes. Children lost
  their own chrome; Inspector is inline content, not a drawer.
- [x] **Picks↔Bankers stale-data race fixed for real** — PicksBoard's fetch
  was pinned to mount, so flipping modes showed the other list's data (round-1
  audit 1.5). `key={dataTab}` remounts it per tab: correct fetch, every time.
- [x] **Data-clarity pass on the board**: the Pinnacle de-vigged probability
  (the sharpest datum in the app) is now shown per priced row ("pin X%");
  model-sourced EV carries the same `*` distinction as the Results screen
  (sharp EV and model EV looked identical — they are not equally
  trustworthy); a footnote explains pin / EV* / cache-first data age.

## Round 14 — Feeling Lucky simplified + market presets

- [x] **Feeling Lucky → one toggle** — the three 0-3 counters are gone; ON adds
  2 of each tier (safe ~75%+ / moderate ~40% / risky ~10%+, 6 extra parlays),
  OFF adds none. Backend unchanged (counts still flow through lucky_safe/
  moderate/risky, so caching and prompts are untouched).
- [x] **Quick mode → named presets** — the three hardcoded combos became
  starter presets. "＋ Save current…" stores the selected markets under any
  name (same name overwrites), tap applies, × deletes, the active-matching
  preset highlights. Persisted in localStorage (`powabet.marketPresets`),
  starter presets restorable by clearing that key.
  - Fix: naming uses an INLINE input (Enter saves, Esc cancels) —
    `window.prompt()` does not exist in Tauri's webview, so the first version
    silently did nothing.
  - Fix: the `.input` CSS class was referenced (preset name, Tracker add-odds)
    but NEVER DEFINED — those inputs rendered as raw native white boxes with
    faint default text on the dark theme. Now a proper themed component class
    (ink background, edge border, readable text, styled placeholder, accent
    focus ring); the preset input is pill-shaped to sit in the chip row.

## Remaining deferred (small)
