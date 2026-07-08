# POWABET — EXIT / SALE REPORT

*Generated 2026-07-05 from a full-repo audit (every Rust module, every React component, extension, proxy, schema). Nothing in this document is invented; every claim traces to code.*

---

## 1. WHAT THIS IS

A Tauri 2.x desktop app for click-driven football player-prop research. Single-user. One direction of data flow:

```
React UI → Tauri commands (63) → { API-Football client | deterministic feature engine | one LLM call } → SQLite
```

Five hard rules enforced in code, not convention: (1) every external call goes through a cache-first client with a hard daily request meter; (2) at most ONE main model call per build, cached by input hash; (3) click-driven; (4) all arithmetic deterministic in Rust — the model only ranks/combines/explains; (5) honest data — proxies labelled, gaps flagged, never fabricated.

---

## 2. STATISTICAL & MATHEMATICAL CORE (the actual IP)

### 2.1 Goal model — Dixon-Coles-corrected bivariate Poisson
- Per-side goal expectation via **Maher (1982) attack×defence interaction**: `λ = 0.5·(atk·opp_def/LEAGUE_AVG) + 0.5·(atk+opp_def)/2` — 50/50 damped toward the plain mean to stop small-sample extrapolation. `HOME_ADV=1.10`, `AWAY_ADJ=0.95`, `LEAGUE_AVG_GOALS=1.35`.
- Full score grid with **Dixon-Coles ρ=−0.08** low-score dependence correction (lifts 0-0/1-1, trims 1-0/0-1). Every scoreline-derived market (1X2, DC, BTTS, totals, correct score, goals range, handicap, halves via first-half share) reads off this ONE grid, so they are mutually consistent by construction.

### 2.2 Volume stats — Negative Binomial with per-market overdispersion
Same Maher crossing applied to shots/corners (`LEAGUE_AVG_SHOTS=12.5`, `CORNERS=5.0`). Counts priced with NB using empirically-set φ: corners 1.18, team cards 1.30, shots 1.20, player SOT 1.15, player shots 1.20, tackles 1.20, fouls 1.15, saves 1.20 — and **passes φ=6.0** (pass counts are wildly overdispersed; priced with a Gaussian on a quantile-derived line instead of naive Poisson).

### 2.3 Player props — per-90 rates × expected minutes, shrunken twice
- Rates = season totals / nineties, scaled by expected minutes (`exp_min/90`).
- **James-Stein shrink toward position-group baselines** (`SHRINK_K=3` pseudo-games) for low-signal props (tackles/fouls/assists/saves).
- **Empirical-Bayes consistency blend** (`K=4`): the Poisson estimate blended with the player's measured recent hit-rate ("carded 4 of last 6") — the "does this actually happen most games" signal a season average misses. A ≥20-point gap flags `form-gap` (role change; book may lag).
- Scorer legs: proxy xG (`SOT·0.30 + other shots·0.05`) **always labelled `proxy`**; drought/regression classifier (`due_regression`, `cold_falling_off`, `hot_in_form`) is context, never a number change.
- Availability multipliers: injured/suspended ×0.15, doubtful ×0.85 — evidence-based sinks, flagged.

### 2.4 Odds layer — power-method de-vig + line shopping
- Pinnacle prices de-vigged with the **power method** (solve k: Σ(1/oᵢ)^k = 1 by 60-iteration bisection) — corrects favourite-longshot bias that proportional de-vig ignores. One-sided prop prices get a 7% margin haircut instead (can't de-vig without the other side).
- Best takeable price line-shopped across user-selected books (Pinnacle excluded from "takeable"). `EV = book_odds × pinnacle_true − 1`, with `ev_source` honestly marked `sharp` vs `model`.

### 2.5 Correlated SGP pricing — Gaussian copula Monte Carlo
One-factor-per-group latent model: `z = L_game·G + L_theme·T + L_subj·S + L_idio·ε` (loadings 0.25/0.55/0.50; scoreline legs get an extra shared "same final score" factor 0.55). PSD by construction, marginals preserved exactly. Deterministic splitmix64 RNG seeded from leg identity → reproducible prices. Acklam inverse-normal (~1e-9). Output: correlated joint prob, naive product, lift, fair odds. Used for: SGP price display, Apex combo search (3k-sim pair screen → triple extension → 10k-sim refinement of survivors), and cherry-pick slips.

### 2.6 Self-correction loops (three, all measured not assumed)
- **Calibration shrink**: settled legs' raw probs vs outcomes → global λ applied to est_prob (`p' = 0.5 + λ(p−0.5)`); measured on `raw_prob` so it never re-corrects its own correction. Applies after ≥30 graded legs.
- **Opponent-strength index**: per-league team attack/defence factors (goals/shots/corners) from last ~10 matches, league-normalized, **schedule-corrected** (rates earned vs soft schedules deflate), shrunken `n/(n+4)`, clamped. Audited by a predicted-vs-actual ledger (`team_pred`); bounded recalibration folds 30% of observed residuals back into factors and consumes the audit rows.
- **Per-market bias table**: predicted vs actual hit-rate per market with signed O/U line margins — feeds a one-tap "use best markets" selector.

### 2.7 Paper-trading evidence engine
Every generated ticket (every strategy, every build) is recorded to a ledger with a dedup signature, auto-settled from results, and reported by strategy/kind/market with void-aware ROI, windowed recency, small-sample warnings, and A/B splits (Grok on/off, ingest on/off, index on/off). **Darwin** adds 8 deterministic micro-strategies paper-traded per slate (0 tokens) as a survival-of-the-fittest leaderboard. **CLV** (closing-line value) captured via a 10-minute pre-kickoff odds snapshot loop — the fastest-converging edge signal.

---

## 3. FEATURE INVENTORY (condensed; every item exists in code)

**Build strategies (11)**: Value (+EV), Secret Picks (likelihood + context), Form Faves, Oracle (three-signal confluence), Power Stacker (2-leg 4x+), Anchors/Bankers (recurring events, hit-rate led), Jackpot (plausible 20-150x), Match Predictor (single-game deep read + deterministic forecast), Scout (fuses ingested pages with engine data), Stacker (stat-led step-up-the-chalk accas), Apex (sharp-edge singles + copula combos only). Plus: deterministic Acca Ladder (geometric hit-target bands, cover-all, mega-acca, one-per-fixture), Feeling Lucky tiers, min-legs, cover-all, odds band, safety ceiling, diversity caps, market presets.

**Data surfaces**: Picks board (every prop ranked), Bankers board, My Picks (persistent personal shortlist → deterministic N-folds or one cached AI assembly; clipboard export), Starting XIs viewer (API → ingested → none, sourced badges), Inspector (squads/players/team stats), Ingested stats, Live (in-play snapshots, live odds, live-adjusted forecast), Tracker (bets, CLV, calibration), Ledger (strategy/kind/market reports, usage costs), History, Newsfeed (Grok digests).

**Ingest pipeline**: Chrome MV3 extension (context-menu + floating bar) → localhost token-auth server → DeepSeek structuring (stats, lineups, summaries) → fixture matching with self-healing IDs + LLM name-fixer → feeds Scout, lineup fallback, XI viewer; CSV export.

**Slip assistant**: extension pulls the latest build from the app and click-places legs on Bet365's bet builder (verified market-group names, expand-then-retry for lazy-rendered sections, accent/surname matching).

**Ops**: request meter w/ hard gate + attempt counting; per-model token/cost accounting; $2/day Grok cap; cancel button that aborts mid-model-call; manual data refresh; export/import/reset; Cloudflare Worker proxy for keyless multi-user installs.

**LLM layer**: Haiku default build model; DeepSeek for data extraction (and optional build model with 2.5× larger tables, 500 rows); optional Sonnet/Opus with two-stage draft→refine; every helper cached by input hash; reground pass rewrites every model number with ours.

---

## 4. IF I WERE YOUR TARGET MARKET: THE BIG NOs

You asked me to be the customer of a pick-selling web app built from this. Here are my honest deal-breakers, hardest first.

### NO #1 — "Show me the audited P/L or you don't exist."
Nothing in this codebase demonstrates a *proven* edge yet. It has the best **measurement machinery** I've seen in an amateur product (void-aware ROI, CLV, calibration, paper ledger) — but the ledger is young and the by-market table you screenshotted shows large negative biases in most player-prop markets (scorers −14pp, SOT −23pp, player shots −41pp). Selling picks off that today is selling noise.
**Fix**: run the paper ledger + real CLV for 3–6 months. Sell ONLY what the evidence supports (your own table says: Match Result +38pp under-confidence, Team Total Goals, Over 1.5, Both Teams Carded). Publish the full, unedited, timestamped ledger — losses included — as the landing page. CLV beating close is the honest proof that converges fastest; the code already computes it.

### NO #2 — "Your model probabilities are self-admitted proxies."
xG is a proxy from SOT counts. Recent form = last 8 games. No possession/xT/pass-network data, no minutes projection beyond season average, no true defensive matchup data. As a paying customer I'd ask "why is your 42% better than the market's?" and today the answer is "it often isn't — see the bias table."
**Fix**: narrow the product to where the pipeline is genuinely strong — **the deterministic layer against soft lines**: opponent-adjusted NB pricing of corners/shots/cards vs bookmaker lines, and the copula's correlated-SGP mispricing (books price SGP legs near-independently; your Gaussian copula lift is a real structural angle). Don't sell "who scores"; sell "this SGP is priced 12% below its correlated fair value."

### NO #3 — "One person's API budget and API-Football's data quality are your whole supply chain."
Empty lineups for friendlies, stats lagging FT by hours, phantom roster players — the codebase is *full* of hard-won guards against this feed's failures. Paying customers will hit the gaps on exactly the matches they care about.
**Fix for resale**: license a proper feed (Opta/Stats Perform tier) before charging anyone, or constrain the product to top-5 leagues + majors where API-Football coverage is verified. The guards you built (poison-proof caching, XI fallbacks) are genuinely valuable ops IP for whoever runs it.

### NO #4 — "A pick with no stake, no book, no timing is not a product."
The desktop app solves this for one user (Kelly, line-shopping, CLV, the slip assistant). A picks feed that says "Messi anytime, 42%" without *price to beat, book, stake sizing, and posted-at timestamp* is astrology.
**Fix**: every published pick must carry: fair prob, minimum acceptable odds, book(s) currently at/above it, timestamp, and suggested fractional-Kelly stake. Your codebase already computes all five — the resale product is a rendering job, not new math.

### NO #5 — "Eleven strategies tells me you don't know which one works."
As a buyer, a menu of Apex/Oracle/Jackpot/Stacker/Power reads as vibes. I want ONE feed with a track record, or at most three risk tiers.
**Fix**: the Darwin/ledger machinery exists precisely to answer this. Let the data kill 8 of the 11. Sell the survivors as "Core" (highest CLV), "Builder" (correlated SGP value), "Longshot" (jackpot tier, clearly labelled entertainment).

### NO #6 — Legal/compliance (not code, but it will kill the business first)
Selling betting picks is regulated advertising in many jurisdictions (licensing, affiliate rules, responsible-gambling requirements, age gating). The Bet365 click-automation extension almost certainly violates their ToS — fine as a personal tool, radioactive as a shipped product.
**Fix**: ship picks + deep links, not automation. Get jurisdiction advice before taking a dollar. Keep the slip assistant as a private power-user tool.

---

## 5. OVER-ENGINEERING AUDIT — WHAT A VIABLE PICKS PRODUCT ACTUALLY NEEDS

The frontend auditor and I agree: the app is a power-user cockpit. A resale product is a feed. Cut accordingly.

### Keep (the viable core, ~25% of the code)
1. **features.rs** — the entire deterministic engine (DC grid, NB pricing, shrinkage, per-90 rates). This is the product.
2. **odds.rs** — power de-vig + line shopping + EV. Non-negotiable.
3. **montecarlo.rs** — copula SGP pricing. Your most defensible angle.
4. **apifootball.rs + db.rs cache/meter** — the supply chain discipline.
5. **settle.rs + generated_tickets ledger + calibration + CLV** — the proof engine. This is your marketing department.
6. **Opponent index** (team factors + audit + bounded recalibration) — real signal, cheap to run server-side on a cron.
7. ONE strategy pipeline (likelihood-led shortlist) + ONE optional LLM pass for the "why" text on published picks.

### Cut or collapse for the web product
| Feature | Verdict | Why |
|---|---|---|
| 11 strategies + prompt blocks | **Collapse to 2–3 tiers** | Ledger picks the survivors; menu breadth is a cost centre and a trust killer |
| Grok/X sentiment + veto | **Cut** | $-metered, soft signal, injuries come from lineups; keep the lineup pipeline instead |
| Weather | **Cut** | Soft context the model reads; no measured impact in the ledger |
| Tactics/coach Haiku pass | **Cut** | Same |
| Plausibility pre-scoring + prewarm UI | **Cut for feed** | ±0.12 rank nudge; a curation human or the bias table does this better for a published feed |
| Feeling Lucky tiers | **Cut** | Gimmick by your own usage |
| Match Predictor / Live / in-play tickets | **Defer** | Live picks = latency + compliance hell; desktop-only feature |
| Ingest extension + Scout | **Keep private** | Your personal edge input; unshippable as-is (scraping third-party sites for customers) |
| Slip assistant (Bet365 automation) | **Keep private** | ToS risk (see NO #6) |
| Simple/Advanced dual mode, 30+ toggles, presets, diversity dials | **Cut** | The web customer makes zero decisions; you make them server-side |
| Two-stage draft→refine, 4-provider LLM routing | **Collapse** | One cheap model for pick rationale text; the numbers never needed an LLM |
| Proxy worker | **Repurpose** | It's already 80% of your web backend's outbound layer |
| Darwin | **Keep server-side, hide** | It's R&D, not product |

### Honest dead-code list (from the audit)
`db::setting_get/set` (dead-marked), `TeamStats.played/fts_rate` (parsed, unused), `BuildSelection.picks/implied_prob` (sent, never read), `apifootball::fetch_fresh` was dead until July (now used), `odds::predictions_summary` (verify caller), `LEAGUE_AVG_OFFSIDES` (declared, deliberately unused pending against-data).

### Port architecture note
The Rust core (features/odds/montecarlo/settle/index) is UI-free and compiles server-side as-is — an Axum service + Postgres swap for SQLite + a cron (build 10:00, refresh odds hourly, settle 23:00, index weekly) replaces the entire Tauri/React layer for the feed product. The React app remains your private cockpit. Estimated carry-over: the hard 25% ports nearly untouched; the other 75% is single-user UX you don't ship.

---

## 6. VALUATION-RELEVANT SUMMARY

**Assets**: deterministic pricing engine with literature-grounded models (Maher, Dixon-Coles, NB overdispersion, power de-vig, Gaussian copula) and every constant documented; a measurement/self-correction stack (calibration, opponent index with audited recalibration, CLV, void-aware paper ledger) that most pick-sellers cannot show; battle-hardened API ops (poison-proof caching, budget gates, honest degradation); a proxy layer that is already a proto-backend.

**Liabilities**: no proven edge yet (the machinery to prove one exists; the months of evidence don't); single low-tier data supplier; player-prop probabilities carry known negative bias pre-recalibration; compliance surface untouched; two features (extension automation, page scraping) unshippable to customers.

**The one-line pitch that survives my own NOs**: *"Correlated SGP and team-line value, priced by a copula the books don't use, sold only after a public ledger proves it."* Everything needed to earn that sentence is already in the repo — what's missing is time and discipline, not code.
