# Decisions log

Autonomous decisions made while building, per the brief ("make it without me, record it here").

## D1 — Key storage: app-data `settings.json`, not OS keychain
**Options**: (a) OS keychain via `keyring` crate, (b) plain config file in the app data dir,
(c) `.env` only.
**Chose**: (b) for the user key + (c) `.env` for the dev default. The keychain crate adds a
platform dependency and can trigger OS unlock prompts — overkill for a single-user local tool.
Keys live in `<app_data_dir>/settings.json` (file perms set to 0600 on unix). The README
states the distribution caveat: if shipped to others, keys must move to a backend proxy.

## D2 — Form source: season aggregates from `/players`, not per-fixture last-5
**Options**: (a) `/fixtures/players?fixture=` for each player's last 5 fixtures (true L5),
(b) `/players?id=&season=` season aggregates.
**Chose**: (b). The spec's REQUEST BUDGET is a HARD CONSTRAINT (≤20 fresh requests for
3 matches / 6 players cold). True last-5 needs ~5 fixture-player fetches *per player* = 30+
requests for 6 players, which blows the budget. The spec itself says `/players` is the
"Primary form source — prefer this over per-fixture loops." So we compute rates (per90,
shots, sot, goals) from season totals and label the basis honestly. Feature fields keep the
`_l5`-style names for continuity but are documented as season-derived estimates in the UI
data-quality notes. The regression guard still runs on these numbers.

## D3 — xG is always a proxy on this tier
API-Football's free tier does not expose measured player xG. We therefore always compute
`proxy_xg = sot*0.30 + (shots-sot)*0.05` and emit `xg_source: "proxy"`. If a future tier or
field exposes real xG, `features.rs::measured_xg()` is the single hook to switch to
`"measured"`. The UI badges every proxy.

## D4 — Opponent xGA + days_rest: flagged unknown to protect the budget
Fetching per-fixture team statistics and last-fixture dates for rest would add 6–12 requests.
For the prototype we set `opp_xga_per_game = None` (neutral) and `days_rest = None`, both
surfaced as data-quality flags. The market scoring degrades gracefully (neutral weight) when
these are absent. This is a deliberate budget trade, not a hidden gap — it is flagged.

## D5 — Squads for player chips via `/players/squads?team=`
The click-driven player step needs a tappable list per team. `/players/squads` returns the
squad in one call per team (cached 24h). Lineups (`/fixtures/lineups`) are often empty until
near kickoff, so squads are the reliable source for selection.

## D6 — Anthropic call from Rust via raw HTTP (reqwest)
There is no official Anthropic Rust SDK, so the backend calls `POST /v1/messages` directly
with `anthropic-version: 2023-06-01`, `model: claude-opus-4-8`, `max_tokens: 1500`, strict
JSON instructions. Adaptive thinking is left off (single structured JSON response, keep tokens
low). On JSON parse failure we retry once with a stricter instruction, then surface a clean
error.

## D7 — TLS + SQLite without system deps
`reqwest` uses `rustls-tls` (no OpenSSL system dependency); `rusqlite` uses the `bundled`
feature (no system SQLite). Keeps the build self-contained on a fresh machine.

## D8 — Cold-cache request budget accounting (3 matches / 6 players)
1 fixtures + 6 squads + 3 injuries + 6 player-season = **16 fresh requests** ≤ 20. Warm
cache for the same day = 0. Verified against the meter logic in `apifootball.rs`.

## D9 — Addendum: xG demoted, all markets first-class, rank by likelihood
Per the addendum, xG is no longer central — it's one optional input used only on
scorer/BTTS markets. The engine's core principle is now **underlying rate vs line** for
every market. `features.rs` emits market-agnostic *candidate legs* (`Candidate`), each with
its own deterministically-computed `est_prob` (Poisson for count props, normal approx for
continuous props, season-rate Poisson blends for team lines). The drought guard is kept as
the scorer-specific case of the general rule. Ranking is purely by `est_prob`; the model is
instructed to prefer a **diverse** mix of markets and never to bias toward scorer markets.
Markets covered: Anytime Scorer, Shots on Target, Tackles, Fouls (committed + drawn), Cards,
Passes Completed (players); BTTS, Win 1st/2nd Half, Over/Under 2.5, Asian Handicap (team).

## D10 — Team-market data vs the request budget
Team lines (BTTS, halves, O/U, handicap) need `/teams/statistics` (1 call per team). The
HARD ≤20 budget is stated for the canonical **3 matches / 6 players player-prop** build,
which stays at 16 fresh requests. Selecting team markets adds up to 2 fetches per fixture
(both teams), an opt-in extra the user chose; the daily meter (blocks at 100) remains the
guardrail. Team stats are cached 24h, so warm rebuilds cost 0. Documented so the canonical
constraint still holds for the player-prop path.

## D11 — Odds/implied-prob not fetched (budget); shown as likelihood-only
The implied-prob comparison (§5) needs `/odds` (1 call per fixture). To protect the budget
in the prototype we don't fetch odds; every leg surfaces its deterministic **model
likelihood** (`est_prob`) and marks the price as unavailable with a flag. The toggle and UI
slot remain so odds can be wired in later via `apifootball::cached_get("/odds", ...)`.

## D12 — Pivot: auto slate of ~10 value tickets (singles + SGP + SGP+)
Per user direction, the tool moved from "one cautious ticket, manual player selection" to an
auto value engine: pick matches → it pulls everything and builds ~10 tickets (Single / SGP /
SGP+), leaning to value/longshots rather than only high-likelihood. Player selection step
removed; markets default to all.

## D13 — Pinnacle (sharp) + Bet365 (price) → +EV
`/odds?fixture=` is parsed for **Pinnacle (id 4)** — de-vigged to a "true" probability — and
**Bet365 (id 8)** — the price you'd take. Per leg: `pinnacle_prob`, `book_odds`,
`ev = book_odds*pinnacle_prob - 1`. The shortlist and the model prioritise +EV. Markets the
feed prices: 1X2-derived, Over/Under 2.5, BTTS, anytime scorer. Player props (shots/tackles/
cards/etc.) are usually unpriced → likelihood-only, flagged "no price". Stake isn't in the
API-Football feed.

## D14 — Auto player discovery via /players?team=&season= (paged)
Instead of per-player `/players?id=`, fetch each team's full squad+stats in 1–3 paged calls,
rank by minutes, and price the top ~9 per team. Removes manual selection and is request-cheap.
Predictions (`/predictions?fixture=`) added as weak context.

## D15 — Deterministic re-grounding of combined numbers
The model returns ticket legs (selection+market+line only, copied from the table). Rust
re-derives every leg's est_prob / pinnacle_prob / book_odds / ev from our candidate data and
computes `combined_prob` (product of est_prob), `combined_odds` (product of Bet365 odds when
all legs priced) and `combined_ev`. Ticket type (Single/SGP/SGP+) is inferred from leg count
and distinct fixtures. SGP combined odds are flagged estimates (no correlated SGP pricing in
the feed; correlated legs are usually cheaper in reality) — honest-data rule preserved.

## D16 — 10-ticket runs auto-saved + cached
`max_tokens` raised to 4096 for the larger output. Every fresh run is auto-saved to
`saved_tickets` (viewable in History) and cached by input hash (now including notes +
predictions); re-running an identical selection returns the cached slate at 0 tokens/requests.
Throttle relaxed earlier (300ms) suits the bigger fetch volume on the paid plan.
