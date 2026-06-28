# CLAUDE.md — powabet engineering guide

A Tauri 2.x desktop app for click-driven football player-prop research. Single user.

## Architecture (one direction of data flow)

```
React UI (src/)  ──invoke──▶  Tauri #[command] (src-tauri/src/commands.rs)
                                     │
            ┌────────────────────────┼─────────────────────────┐
            ▼                        ▼                          ▼
   apifootball.rs (cache-first   features.rs (deterministic   llm.rs (one Opus
   HTTP, request meter)          feature engine + ranking)    call, cached)
            │                                                    │
            └──────────────── db.rs (SQLite: cache, meter, ai_results, tickets) ─┘
```

The frontend **never** calls an external API. It only invokes Rust commands.

## Hard rules (from spec — never break)

1. **Request budget**: every external call goes through `apifootball::cached_get`. Never
   hit the network if a fresh cached row exists. The daily meter blocks fresh calls at 100.
2. **Token budget**: the model is called at most once per Build. Output is cached by a
   hash of its input. Never loop the model per player/match.
3. **Click-driven**: no required typing except the optional notes box and API keys.
4. **Deterministic numbers, LLM synthesis**: all arithmetic in `features.rs`. The model
   only weighs/ranks/explains — it never invents a probability.
5. **Honest data**: proxy xG is always labelled `xg_source: "proxy"`. Never present a
   proxy as measured. Never fabricate a number to fill a gap.

## Conventions

- **Rust**: keep modules focused (one responsibility each). API responses parsed as
  `serde_json::Value` defensively (the free-tier API is inconsistent); our own data uses
  typed structs in `models.rs`. Errors bubble up as `Result<_, String>` to the frontend.
- **Cache keys**: `sha256(endpoint + sorted_params)`. AI input hash:
  `sha256(compact_table + markets + reasoning_flag + model)`.
- **No secrets in source.** Keys load from `.env` (dev) or the app-data `settings.json`
  (user). `.env` is gitignored.
- **Frontend**: TypeScript + React + Tailwind. State lives in `App.tsx`; going Back never
  re-fetches (cache + kept state). Touch-friendly buttons, one screen per step.
- **Don't over-engineer.** Cover the common path; flag uncertainty instead of handling
  every edge. Block the 99%.

## Where things live

- `src-tauri/src/db.rs` — schema + SQLite helpers
- `src-tauri/src/apifootball.rs` — cache-first API-Football client + request meter
- `src-tauri/src/features.rs` — feature engine, regression guard, ranking, compact table
- `src-tauri/src/llm.rs` — Anthropic call + result cache
- `src-tauri/src/commands.rs` — Tauri commands (the only frontend surface)
- `src-tauri/src/models.rs` — typed structs shared with the frontend (serde)
- `src/` — React UI, one component per step
