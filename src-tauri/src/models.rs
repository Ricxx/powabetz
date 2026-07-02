//! Typed structs shared with the frontend (serde-serialised). API-Football
//! responses are parsed as `serde_json::Value` elsewhere; these are *our* shapes.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct Fixture {
    pub fixture_id: i64,
    pub league_id: i64,
    pub league_name: String,
    pub season: i64,
    pub date_utc: String,
    pub home_team_id: i64,
    pub home_team: String,
    pub away_team_id: i64,
    pub away_team: String,
    pub status: String,
    #[serde(default)]
    pub venue_city: Option<String>,
    #[serde(default)]
    pub venue_name: Option<String>,
    #[serde(default)]
    pub referee: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LeagueOption {
    pub id: i64,
    pub name: String,
    pub country: String,
    /// how many times the user has picked this league (drives the sort order).
    pub picks: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SquadPlayer {
    pub player_id: i64,
    pub name: String,
    pub position: String,
    pub team_id: i64,
    pub team_name: String,
    /// availability badge from injuries endpoint, defaulting to "unknown".
    pub availability: String,
}

/// Returned to the players step: chips grouped per team for the selected matches.
#[derive(Debug, Clone, Serialize)]
pub struct TeamSquad {
    pub team_id: i64,
    pub team_name: String,
    pub fixture_id: i64,
    pub players: Vec<SquadPlayer>,
}

/// A candidate betting *leg*, market-agnostic. The deterministic engine compares
/// the subject's underlying rate against the line and emits an estimated
/// probability the line hits. This is the ONLY data the model sees. The model
/// ranks legs by likelihood and prefers a diverse set of markets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub subject: String,      // player name, or team name for match lines
    pub subject_kind: String, // "player" | "team"
    pub team: String,
    pub opponent: String,
    pub fixture: String,
    pub fixture_id: i64,
    pub market: String,       // e.g. "Anytime Scorer", "2+ Tackles", "BTTS"
    pub market_group: String, // toggle key it came from
    pub line: String,         // human line label, e.g. "1+ goal"
    pub base_rate: f64,        // the underlying per-90 / per-game rate vs the line
    pub est_prob: f64,         // our deterministic P(line hits)
    pub pinnacle_prob: Option<f64>, // Pinnacle de-vigged "true" probability (sharp)
    pub book_odds: Option<f64>,     // best decimal odds across books — the price to take
    pub book: Option<String>,       // which book offers that best price
    pub ev: Option<f64>,            // book_odds * true_prob - 1 (sharp if pinnacle, else model)
    pub ev_source: Option<String>,  // "sharp" (Pinnacle) | "model" (our prob)
    pub form_state: Option<String>, // scorer-style drought guard, when relevant
    pub xg_source: Option<String>,  // only set on markets that use xG
    pub support: Vec<String>,  // short supporting facts for the model
    pub flags: Vec<String>,    // proxy / missing-data labels (honest-data rule)
    /// Haiku per-fixture plausibility (1-5) — a QUALITATIVE context weight, never a
    /// probability. None = not scored. The one-line reason lives in `support`.
    #[serde(default)]
    pub plausibility: Option<u8>,
    /// The RAW engine probability before any calibration shrink / adjustment.
    /// Calibration must be measured against this (measuring the shrunk value
    /// makes the loop re-correct its own correction). None = never adjusted.
    #[serde(default)]
    pub raw_prob: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestMeter {
    pub day: String,
    pub count: i64,
    pub limit: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageTotal {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettingsView {
    pub has_api_football_key: bool,
    pub has_anthropic_key: bool,
    pub has_grok_key: bool,
    pub has_openai_key: bool,
    pub has_deepseek_key: bool,
    pub has_parlay_key: bool,
    pub model: String,
    pub books: Vec<String>,
    pub kelly_fraction: f64,
    pub default_stake: f64,
    pub timezone: String,
    pub proxy_url: String,
    pub has_proxy_token: bool,
    pub meter: RequestMeter,
    pub usage: UsageTotal,
}

/// Per-build token usage + cost, shown on the results card.
#[derive(Debug, Clone, Serialize)]
pub struct BuildUsage {
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
    pub from_cache: bool,
}

/// One leg within a ticket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketLeg {
    pub r#match: String,
    #[serde(default)]
    pub fixture_id: i64,
    pub market: String,
    pub selection: String,
    /// The subject's team (player legs only) — for a short badge in the UI.
    #[serde(default)]
    pub team: Option<String>,
    #[serde(default)]
    pub line: Option<String>,
    #[serde(default)]
    pub est_prob: Option<f64>,
    #[serde(default)]
    pub pinnacle_prob: Option<f64>,
    #[serde(default)]
    pub book_odds: Option<f64>,
    #[serde(default)]
    pub book: Option<String>,
    #[serde(default)]
    pub ev: Option<f64>,
    #[serde(default)]
    pub ev_source: Option<String>,
    /// Raw engine probability before calibration shrink (see Candidate::raw_prob).
    #[serde(default)]
    pub raw_prob: Option<f64>,
}

/// One ticket: a single bet or a multi-leg SGP / SGP+, produced by the model and
/// re-grounded numerically in Rust.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticket {
    #[serde(default, rename = "type")]
    pub kind: String, // "Single" | "SGP" | "SGP+"
    #[serde(default)]
    pub title: String,
    pub confidence: String,
    #[serde(default)]
    pub legs: Vec<TicketLeg>,
    #[serde(default)]
    pub combined_prob: Option<f64>,
    #[serde(default)]
    pub combined_odds: Option<f64>,
    #[serde(default)]
    pub combined_ev: Option<f64>,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub why: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForecastLine {
    pub label: String,
    pub pct: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForecastSection {
    pub title: String,
    pub lines: Vec<ForecastLine>,
}
/// A deterministic single-match forecast — likely result, scorelines, goals,
/// cards/corners and key players, each with a % — read straight off our computed
/// candidate probabilities. Surfaced by the Match Predictor mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchForecast {
    pub home: String,
    pub away: String,
    pub headline: String,
    pub sections: Vec<ForecastSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BuildResult {
    pub tickets: Vec<Ticket>,
    /// Single-match forecast (Match Predictor mode only).
    #[serde(default)]
    pub forecast: Option<MatchForecast>,
    /// Per-fixture forecasts (Simple mode — one per selected match).
    #[serde(default)]
    pub forecasts: Vec<MatchForecast>,
    #[serde(default)]
    pub data_quality_notes: Vec<String>,
    /// Match context fed to the model (predictions, standings, H2H, weather,
    /// referee) — surfaced so the user can see what informed the build.
    #[serde(default)]
    pub context_notes: Vec<String>,
    #[serde(default)]
    pub from_cache: bool,
    #[serde(default)]
    pub grok_used: bool,
    #[serde(default)]
    pub grok_digest: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildResponse {
    pub result: BuildResult,
    pub meter: RequestMeter,
    pub usage: BuildUsage,
}

/// Fixture context passed from the UI so the backend needn't re-fetch fixtures.
#[derive(Debug, Clone, Deserialize)]
pub struct FixtureInput {
    pub fixture_id: i64,
    pub league_id: i64,
    pub season: i64,
    pub home_team_id: i64,
    pub home_team: String,
    pub away_team_id: i64,
    pub away_team: String,
    /// Kickoff (RFC3339). Lets the backend detect an in-play fixture and pull
    /// live state instead of reasoning off pre-match season rates.
    #[serde(default)]
    pub date_utc: Option<String>,
    #[serde(default)]
    pub venue_city: Option<String>,
    #[serde(default)]
    pub referee: Option<String>,
}

/// A selection coming from the UI: which players in which fixtures + the markets.
#[derive(Debug, Clone, Deserialize)]
pub struct BuildSelection {
    // Legacy manual picks — auto mode ignores these (kept for back-compat).
    #[serde(default)]
    #[allow(dead_code)]
    pub picks: Vec<SelectionPick>,
    pub fixtures: Vec<FixtureInput>,
    pub markets: Vec<String>,
    pub reasoning: bool,
    // Sent by the UI; odds aren't fetched in this build so the backend doesn't read it.
    #[serde(default)]
    #[allow(dead_code)]
    pub implied_prob: bool,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub model: String,
    /// Target number of tickets (default 10).
    #[serde(default)]
    pub ticket_count: Option<u32>,
    /// Allowed ticket types, e.g. ["Single","SGP","SGP+"] (default all).
    #[serde(default)]
    pub ticket_types: Vec<String>,
    /// Variation seed — >0 forces a fresh, different slate (bypasses the cache).
    #[serde(default)]
    pub variation: u32,
    /// Ticket signatures to avoid when generating a new variation.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Subjects (players/teams) the user VOIDED on the results screen — dropped
    /// from the candidate pool so they can't return in the next set.
    #[serde(default)]
    pub exclude_subjects: Vec<String>,
    /// Bias bet builders toward priced markets (so combined odds/EV are real).
    #[serde(default)]
    pub bias_builders: bool,
    /// Rank by pure likelihood (a wide data scan) instead of +EV. Legacy — use
    /// `strategy` when present.
    #[serde(default)]
    pub most_likely: bool,
    /// Build strategy: "value" (+EV), "favorites" (in-form favourites at useful
    /// odds), or "likely" (most-probable / secret picks).
    #[serde(default)]
    pub strategy: Option<String>,
    /// Drop legs MORE likely than this (1.0 = off) — caps how "safe"/chalky legs
    /// can be, pushing toward less-obvious picks.
    #[serde(default)]
    pub max_leg_prob: Option<f64>,
    /// Deterministic forecasts ONLY — skip the model call entirely (0 tokens).
    /// Used by the Live screen's match-predict, which only shows the forecast.
    #[serde(default)]
    pub forecast_only: bool,
    /// Run the Grok X/news sentiment precursor for this query.
    #[serde(default)]
    pub use_grok: bool,
    /// Which Grok sections to fetch (empty = injuries+news). Fewer = cheaper.
    #[serde(default)]
    pub grok_categories: Vec<String>,
    /// Context signals to fetch (None = on). Turn off to speed up the build.
    #[serde(default)]
    pub use_weather: Option<bool>,
    #[serde(default)]
    pub use_standings: Option<bool>,
    #[serde(default)]
    pub use_h2h: Option<bool>,
    #[serde(default)]
    pub use_lineups: Option<bool>,
    /// Feed API-Football's own model predictions (win% / advice) to the model.
    #[serde(default)]
    pub use_predictions: Option<bool>,
    /// Use real xG (recent form) for team goal models — extra requests, cached.
    #[serde(default)]
    pub use_xg: Option<bool>,
    /// Add coach/formation tactical play-style context (cheap Haiku, cached).
    #[serde(default)]
    pub use_tactics: Option<bool>,
    /// Hard rule: drop legs for players Grok flags as out/suspended.
    #[serde(default)]
    pub grok_veto: bool,
    /// "Feeling Lucky" tiers — counts per band (>~75% / ~40% / >~10%).
    #[serde(default)]
    pub lucky_safe: u32,
    #[serde(default)]
    pub lucky_moderate: u32,
    #[serde(default)]
    pub lucky_risky: u32,
    /// Minimum legs per ticket (e.g. 4 = only 4-folds and up). 1 = off.
    #[serde(default)]
    pub min_legs: Option<u32>,
    /// Per-leg odds band — drop priced legs cheaper than min or longer than max
    /// (the "sweet spot", e.g. 1.3–7.0). None = no floor / no ceiling.
    #[serde(default)]
    pub min_odds: Option<f64>,
    #[serde(default)]
    pub max_odds: Option<f64>,
    /// Diversity: max times one player/team may appear across the slate (0/None = model default).
    #[serde(default)]
    pub max_per_subject: Option<u32>,
    /// Run the per-fixture Haiku plausibility pre-score (cached) and blend it into ranking.
    #[serde(default)]
    pub use_plausibility: Option<bool>,
    /// Minimum plausibility (1-5) to keep a candidate. None/1 = no filter.
    #[serde(default)]
    pub min_plausibility: Option<u8>,
    /// Feed matched browser-ingested page data into the build as labeled context.
    #[serde(default)]
    pub use_ingest: Option<bool>,
    /// Simple mode: compute a forecast for EVERY selected fixture (not just one).
    #[serde(default)]
    pub simple: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct SelectionPick {
    pub fixture_id: i64,
    pub league_id: i64,
    pub season: i64,
    pub player_id: i64,
    pub team_id: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SavedTicket {
    pub id: i64,
    pub created_at: i64,
    pub result_json: String,
    pub user_notes: String,
}

// ---------- cost breakdown ----------

#[derive(Debug, Clone, Serialize)]
pub struct UsageBreakdown {
    pub today: f64,
    pub week: f64,
    pub month: f64,
    pub lifetime: f64,
    pub today_tokens: i64,
    pub week_tokens: i64,
    pub lifetime_tokens: i64,
    // Grok (x.ai) spend, tracked separately (different provider/pricing).
    pub grok_today: f64,
    pub grok_week: f64,
    pub grok_month: f64,
    pub grok_lifetime: f64,
}

// ---------- bet tracking ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegResult {
    pub won: Option<bool>, // None = not yet gradeable (or void — see `void`)
    pub detail: String,
    /// For O/U-style legs: signed gap to the line in the bet's favour (actual −
    /// line for an over, line − actual for an under). +0.5 = won by half a unit;
    /// −0.5 = a near-miss loss. None for non-O/U markets.
    #[serde(default)]
    pub margin: Option<f64>,
    /// Books refund a leg that can't action (player never featured, match
    /// postponed/abandoned). A void leg is settled — it just doesn't count
    /// toward win/lose; an all-void ticket returns the stake.
    #[serde(default)]
    pub void: bool,
}

// ---------- in-play / live ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveFixture {
    pub fixture_id: i64,
    pub league_id: i64,
    pub league_name: String,
    pub season: i64,
    pub home_team: String,
    pub away_team: String,
    pub home_team_id: i64,
    pub away_team_id: i64,
    pub status: String, // 1H | HT | 2H | ET | …
    pub elapsed: i64,
    pub home_goals: i64,
    pub away_goals: i64,
    pub has_stats: bool, // live in-match stats available (big leagues)
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveStatKV {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveTeamStat {
    pub team: String,
    pub stats: Vec<LiveStatKV>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveEvent {
    pub minute: i64,
    pub team: String,
    pub kind: String, // Goal | subst | Card | Var
    pub player: String,
    pub detail: String,
}

/// Our estimate of a still-live outcome over the remaining time.
#[derive(Debug, Clone, Serialize)]
pub struct LiveEstimate {
    pub label: String,
    pub prob: f64,
    pub basis: String, // "model (rate × time left)" | "+ live momentum" | "pace"
    /// Edge vs the matching in-play odd (our prob − implied), when we found one.
    pub edge: Option<f64>,
    pub book: Option<String>, // the matched market/price, for display
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveOdd {
    pub market: String,
    pub selection: String,
    pub odds: f64,
    pub implied: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveSnapshot {
    pub fixture: LiveFixture,
    pub stats: Vec<LiveTeamStat>,
    pub events: Vec<LiveEvent>,
    pub estimates: Vec<LiveEstimate>,
    pub odds: Vec<LiveOdd>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveLeg {
    pub label: String,
    pub prob: f64,
    pub odds: Option<f64>,
    pub source: String, // "model" | "book"
    pub why: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct LiveTicket {
    pub fixture: LiveFixture,
    pub legs: Vec<LiveLeg>,
    pub combined_prob: f64,
    pub combined_odds: Option<f64>,
    pub rationale: String,
    pub confidence: String, // low | medium | high
    pub model: String,
    pub cached: bool,
    pub note: String,
}

/// A page the user ingested via the browser extension (raw → Haiku-structured).
#[derive(Debug, Clone, Serialize)]
pub struct IngestItem {
    pub id: i64,
    pub created_at: i64, // when ingested
    pub url: String,
    pub title: String,
    pub note: String,
    pub status: String, // new | processed
    pub fixture_label: Option<String>,
    pub fixture_date: Option<String>, // date of the fixture the page is about
    pub summary: String,
    pub data: Vec<IngestKV>, // the structured extraction, viewable to verify it isn't garbage
    pub model: Option<String>,
    pub used: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct IngestKV {
    pub label: String,
    pub value: String,
}

/// Token usage + cost grouped by (model, purpose) — what each model contributed.
#[derive(Debug, Clone, Serialize)]
pub struct ModelPurposeRow {
    pub model: String,
    pub purpose: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
}

/// Local ingest endpoint info (for Settings + the extension to connect).
#[derive(Debug, Clone, Serialize)]
pub struct IngestInfo {
    pub enabled: bool,
    pub port: u16,
    pub token: String,
    pub new_count: i64,
}

/// Per-market (per-pick) settlement stats from the generated ledger — the model's
/// predicted hit-rate vs the ACTUAL hit-rate, so biases per market are visible.
#[derive(Debug, Clone, Serialize)]
pub struct MarketReportRow {
    pub market: String,
    pub settled: i64,
    pub won: i64,
    pub hit_rate: f64,  // actual
    pub predicted: f64, // mean model est_prob for this market
    /// O/U markets only: mean signed gap to the line (+ = cleared comfortably,
    /// − = lost/landed tight). None for non-O/U markets.
    #[serde(default)]
    pub avg_margin: Option<f64>,
    /// O/U losses that missed the line by less than 1 unit (near-misses).
    #[serde(default)]
    pub near_misses: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlacedBet {
    pub id: i64,
    pub created_at: i64,
    pub day: String,
    pub ticket: Ticket,
    pub stake: f64,
    pub status: String, // open | won | lost | partial | void
    pub returns: f64,
    pub leg_results: Vec<LegResult>,
    pub settled: bool,
    pub grok_used: bool,
    pub strategy: String, // strategy key (value/oracle/scout/… or board/live/custom)
    /// Closing-line value: avg (placed / close − 1) across priced legs.
    /// Positive = beat the close — the fastest-converging proof of edge.
    #[serde(default)]
    pub clv: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketEval {
    #[serde(default)]
    pub analysis: String,
    /// Per-leg assessment — clear structured data, one entry per leg.
    #[serde(default)]
    pub leg_notes: Vec<LegNote>,
    #[serde(default)]
    pub risks: Vec<String>,
    /// Concrete suggested changes to improve the ticket (swap/drop/add a leg, etc).
    #[serde(default)]
    pub recommendations: Vec<String>,
    #[serde(default)]
    pub verdict: String, // Strong | Fair | Thin
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegNote {
    #[serde(default)]
    pub leg: String, // which leg (player/team + market)
    #[serde(default)]
    pub rating: String, // solid | ok | risky | trap
    #[serde(default)]
    pub note: String, // 4-10 word real-world reason
}

#[derive(Debug, Clone, Serialize)]
pub struct GrokLogEntry {
    pub id: i64,
    pub created_at: i64,
    pub matches: String,
    pub digest: String,
}

/// One reliability bin: how our predicted probability compared to reality.
#[derive(Debug, Clone, Serialize)]
pub struct CalBin {
    pub lo: f64,
    pub hi: f64,
    pub predicted_avg: f64,
    pub actual_rate: f64,
    pub n: i64,
}

/// Calibration of our `est_prob` vs settled outcomes — drives an optional shrink.
#[derive(Debug, Clone, Serialize)]
pub struct CalibrationReport {
    pub bins: Vec<CalBin>,
    /// Slope of (outcome−0.5) on (pred−0.5): <1 overconfident, >1 underconfident.
    pub lambda: f64,
    pub n: i64,
    pub verdict: String,
    /// Whether the shrink is being applied to builds (n above the threshold).
    pub applied: bool,
}

/// One row of the generated-tickets ledger report (per strategy + grok flag).
#[derive(Debug, Clone, Serialize)]
pub struct GenReportRow {
    pub strategy: String,
    pub grok_used: bool,
    pub total: i64,
    pub settled: i64,
    pub won: i64,
    pub hit_rate: f64,
    pub roi: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BankrollView {
    pub bankroll: f64,
    pub staked_open: f64,
    pub pnl: f64,
    pub current: f64,
    pub open_count: i64,
    pub settled_count: i64,
}

// ---------- data inspector (reads cache only — 0 requests) ----------

#[derive(Debug, Clone, Serialize)]
pub struct TeamStatsView {
    pub played: f64,
    pub gf_avg: f64,
    pub ga_avg: f64,
    pub ppg: f64,
    pub first_half_share: f64,
    pub fts_rate: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlayerLite {
    pub player_id: i64,
    pub name: String,
    pub position: String,
    pub availability: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InspectTeam {
    pub team_id: i64,
    pub team_name: String,
    pub loaded: bool, // whether the squad was found in cache
    pub stats: Option<TeamStatsView>,
    pub players: Vec<PlayerLite>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InspectFixture {
    pub fixture_id: i64,
    pub league_id: i64,
    pub season: i64,
    pub fixture_label: String,
    pub teams: Vec<InspectTeam>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlayerRates {
    pub goals: f64,
    pub sot: f64,
    pub shots: f64,
    pub tackles: f64,
    pub fouls: f64,
    pub cards: f64,
    pub passes: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlayerInspect {
    pub name: String,
    pub position: String,
    pub apps: f64,
    pub minutes: f64,
    pub goals: f64,
    pub shots: f64,
    pub sot: f64,
    pub tackles: f64,
    pub fouls_committed: f64,
    pub fouls_drawn: f64,
    pub cards: f64,
    pub passes: f64,
    pub per90: PlayerRates,
}
