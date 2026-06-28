# powabet

A click-driven desktop app (Tauri 2 + React + Rust) for researching football
player-prop and match bets. You tap a date → matches → players → markets → **Build
Tickets**, and a deterministic Rust engine plus a single Claude Opus 4.8 call return a
ranked, mixed ticket with reasoning.

> Research and price context only — **not financial advice**, no claimed market edge.
> Stacking short-priced legs compounds the bookmaker margin across the ticket regardless
> of pick quality.

## What it does

- **Click-driven flow**: the only typing anywhere is the optional "My notes" box and the
  API keys in Settings.
- **Deterministic numbers, LLM synthesis**: every base rate, probability and the
  scoring-drought guard are computed in Rust (`src-tauri/src/features.rs`). The model only
  weighs, ranks, and explains — it never invents a number.
- **All markets first-class**: anytime scorer, shots on target, tackles, fouls, cards,
  passes (player props); BTTS, win 1st/2nd half, over/under 2.5, Asian handicap (team
  lines). Legs are ranked purely by likelihood and the model builds a **diverse** ticket.
- **Request-budget safe**: the API-Football free tier (~100 req/day) is protected by a
  cache-first layer and a visible daily meter that blocks fresh calls at 100. A canonical
  3-match / 6-player player-prop build costs ≤ 20 fresh requests cold, ~0 warm.
- **One model call per Build**, cached by a hash of its input — re-running an identical
  selection costs 0 tokens.
- **Honest data**: proxied or missing stats (estimated xG, season-rate team proxies, no
  odds) are always flagged, never presented as measured.

## Architecture

```
React UI (src/)  ──invoke──▶  Tauri commands (src-tauri/src/commands.rs)
   never calls an API              │
        ┌──────────────────────────┼───────────────────────────┐
   apifootball.rs            features.rs                    llm.rs
   (cache-first HTTP,        (deterministic engine:         (one Opus 4.8 call,
    request meter)            rate-vs-line per market,       cached by input hash)
        │                     drought guard, ranking)            │
        └──────────────── db.rs (SQLite: cache / meter / ai_results / tickets) ┘
```

See `CLAUDE.md` for conventions and `decision.md` for every autonomous design choice.

## Setup

Prereqs: Rust (stable), Node 18+, and the
[Tauri 2 system prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```bash
npm install
cp .env.example .env        # dev only — fill in keys (optional; Settings works too)
npm run tauri dev           # launches the desktop app
```

Build a release bundle:

```bash
npm run tauri build
```

## Keys

Two keys are needed:

- **API-Football** (`api-sports.io` v3) — for fixtures, squads, player/team stats.
- **Anthropic** (`claude-opus-4-8`) — for the single ranking/explanation call.

Provide them either way:

1. **Settings screen** (recommended for the user) → stored in
   `<app data dir>/settings.json`, locked to `0600` on unix.
2. **`.env`** at the repo root (dev convenience) → `API_FOOTBALL_KEY`, `ANTHROPIC_API_KEY`.
   `.env` is gitignored; a `.env.example` with empty values is committed.

Keys are read by the Rust backend only. They are **never** hardcoded in source and never
embedded in the binary.

### ⚠️ Distribution caveat

This is a **single-user local tool**. Local key storage is fine here. If you ever
distribute the app to other users, the keys must **not** live in (or be reachable from)
the binary — they are extractable. That case requires a thin backend proxy that holds the
keys and exposes only the endpoints this app needs. Do not ship this as-is to third
parties with real keys.

## Data-quality notes (read these)

This prototype deliberately trades some data depth for the request budget (see
`decision.md`):

- **Form is season-derived**, not literal last-5 fixtures (true L5 would blow the budget).
- **xG is always a proxy** on this API tier (`sot·0.30 + other shots·0.05`) and is used
  only as a supporting input on scorer/BTTS markets — never as a backbone.
- **Team lines use crude season-rate proxies**; defensive workload for tackles/fouls is
  proxied from home/away.
- **Odds are not fetched**, so the implied-prob comparison shows likelihood only and marks
  price as unavailable. The hook to wire `/odds` in later is in
  `apifootball::cached_get`.

Every one of these is surfaced in the UI's "Data quality" panel on each build.

## Project layout

```
src/                     React + TS frontend (one component per step)
src-tauri/src/
  commands.rs            Tauri commands — the only frontend surface
  apifootball.rs         cache-first API-Football client + request meter
  features.rs            deterministic engine: rates, drought guard, ranking
  llm.rs                 Anthropic call + result cache
  db.rs                  SQLite schema + helpers
  models.rs              serde structs shared with the frontend
  lib.rs / main.rs       app wiring, state, key loading
```
