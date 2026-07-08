// Mirrors the serde structs in src-tauri/src/models.rs.

export interface Fixture {
  fixture_id: number;
  league_id: number;
  league_name: string;
  season: number;
  date_utc: string;
  home_team_id: number;
  home_team: string;
  away_team_id: number;
  away_team: string;
  status: string;
  venue_city?: string | null;
  venue_name?: string | null;
  referee?: string | null;
}

export interface SquadPlayer {
  player_id: number;
  name: string;
  position: string;
  team_id: number;
  team_name: string;
  availability: string;
}

export interface TeamSquad {
  team_id: number;
  team_name: string;
  fixture_id: number;
  players: SquadPlayer[];
}

export interface RequestMeter {
  day: string;
  count: number;
  limit: number;
}

export interface UsageTotal {
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
}

export interface BuildUsage {
  model: string;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
  from_cache: boolean;
}

export interface SettingsView {
  has_api_football_key: boolean;
  has_anthropic_key: boolean;
  has_grok_key: boolean;
  has_openai_key: boolean;
  has_deepseek_key: boolean;
  has_parlay_key: boolean;
  has_propline_key: boolean;
  model: string;
  books: string[];
  kelly_fraction: number;
  default_stake: number;
  timezone: string;
  proxy_url: string;
  has_proxy_token: boolean;
  use_team_index: boolean;
  excluded_markets: string[];
  deepseek_thinking: boolean;
  meter: RequestMeter;
  usage: UsageTotal;
}

export interface CalBin {
  lo: number;
  hi: number;
  predicted_avg: number;
  actual_rate: number;
  n: number;
}

export interface CalibrationReport {
  bins: CalBin[];
  lambda: number;
  n: number;
  verdict: string;
  applied: boolean;
}

export interface GrokLogEntry {
  id: number;
  created_at: number;
  matches: string;
  digest: string;
}

// Common bookmaker names to line-shop (matched case-insensitively against the feed).
export const COMMON_BOOKS = [
  "Bet365",
  "William Hill",
  "1xBet",
  "Bwin",
  "Unibet",
  "Betfair",
  "Marathonbet",
  "888sport",
  "Betano",
];

export const MODEL_OPTIONS = [
  { id: "claude-haiku-4-5", label: "Haiku 4.5", note: "$1 / $5 per 1M — default, sharp + cheap" },
  { id: "deepseek-v4-pro", label: "DeepSeek v4 Pro", note: "~$0.30 / $1.20 — data crunching, weaker prose" },
  { id: "claude-sonnet-5", label: "Sonnet 5", note: "$3 / $15 — premium, higher cost" },
  { id: "claude-opus-4-8", label: "Opus 4.8", note: "$5 / $25 — premium, sharpest" },
];

// Models for the per-ticket quick analysis (a second angle). GPT needs an OpenAI key.
export const ANALYSIS_MODELS = [
  { id: "claude-haiku-4-5", label: "Haiku", provider: "claude" },
  { id: "deepseek-v4-pro", label: "DeepSeek", provider: "deepseek" },
  { id: "gpt-5-nano", label: "GPT-5 nano", provider: "openai" },
  { id: "gpt-5-mini", label: "GPT-5 mini", provider: "openai" },
];

export interface TicketLeg {
  match: string;
  fixture_id?: number;
  market: string;
  selection: string;
  team?: string | null;
  line?: string | null;
  est_prob?: number | null;
  pinnacle_prob?: number | null;
  book_odds?: number | null;
  book?: string | null;
  ev?: number | null;
  ev_source?: string | null;
  raw_prob?: number | null; // engine prob before the calibration shrink
}

export interface Ticket {
  type: string; // Single | SGP | SGP+
  title: string;
  confidence: string;
  legs: TicketLeg[];
  combined_prob?: number | null;
  combined_odds?: number | null;
  combined_ev?: number | null;
  flags: string[];
  why?: string | null;
}

export interface ForecastLine { label: string; pct: number }
export interface ForecastSection { title: string; lines: ForecastLine[] }
export interface MatchForecast {
  home: string;
  away: string;
  headline: string;
  sections: ForecastSection[];
}
export interface BuildResult {
  tickets: Ticket[];
  forecast?: MatchForecast | null;
  forecasts?: MatchForecast[];
  data_quality_notes: string[];
  context_notes?: string[];
  from_cache: boolean;
  grok_used?: boolean;
  /// Ingested page data actually fed this build — carried to bets/ledger (A/B).
  ingest_used?: boolean;
  /// Opponent-strength index adjusted this build (A/B, like ingest/grok).
  index_used?: boolean;
  grok_digest?: string | null;
}

export interface Candidate {
  subject: string;
  subject_kind: string;
  team: string;
  opponent: string;
  fixture: string;
  fixture_id: number;
  market: string;
  market_group: string;
  line: string;
  base_rate: number;
  est_prob: number;
  pinnacle_prob?: number | null;
  book_odds?: number | null;
  book?: string | null;
  ev?: number | null;
  ev_source?: string | null;
  form_state?: string | null;
  xg_source?: string | null;
  plausibility?: number | null;
  support: string[];
  flags: string[];
}

export interface LegNote {
  leg: string;
  rating: string; // solid | ok | risky | trap
  note: string;
}

export interface TicketEval {
  analysis: string;
  leg_notes: LegNote[];
  risks: string[];
  recommendations: string[];
  verdict: string;
}

export interface GenReportRow {
  strategy: string;
  grok_used: boolean;
  total: number;
  settled: number;
  won: number;
  hit_rate: number;
  roi: number | null;
  /// All-void tickets (pushes) — settled but excluded from hit/ROI.
  voided?: number;
  /// Avg PREDICTED combined hit chance of the settled tickets — vs hit_rate =
  /// the strategy's honesty (per-strategy calibration).
  predicted_hit?: number | null;
}

export interface MarketReportRow {
  market: string;
  settled: number;
  won: number;
  hit_rate: number; // actual
  predicted: number; // model's mean predicted prob
  avg_margin?: number | null; // O/U: mean signed gap to the line
  near_misses?: number; // O/U losses within 1 of the line
}

export interface IngestKV {
  label: string;
  value: string;
}

export interface IngestItem {
  id: number;
  created_at: number;
  url: string;
  title: string;
  note: string;
  status: string; // new | processed
  fixture_label?: string | null;
  fixture_date?: string | null;
  /// "fixture" = real kickoff in YOUR timezone; "page" = date as the site printed it.
  date_source?: string;
  summary: string;
  data: IngestKV[];
  model?: string | null;
  used: boolean;
}

export interface IngestInfo {
  enabled: boolean;
  port: number;
  token: string;
  new_count: number;
}

export interface LiveFixture {
  fixture_id: number;
  league_id: number;
  league_name: string;
  season: number;
  home_team: string;
  away_team: string;
  home_team_id: number;
  away_team_id: number;
  status: string;
  elapsed: number;
  home_goals: number;
  away_goals: number;
  has_stats: boolean;
  date_utc?: string | null;
}
export interface LiveStatKV { label: string; value: string }
export interface LiveTeamStat { team: string; stats: LiveStatKV[] }
export interface LiveEvent { minute: number; team: string; kind: string; player: string; detail: string }
export interface LiveEstimate { label: string; prob: number; basis: string; edge?: number | null; book?: string | null }
export interface LiveOdd { market: string; selection: string; odds: number; implied: number }
export interface LiveSnapshot {
  fixture: LiveFixture;
  stats: LiveTeamStat[];
  events: LiveEvent[];
  estimates: LiveEstimate[];
  odds: LiveOdd[];
  note: string;
}
export interface LiveLeg { label: string; prob: number; odds: number | null; source: string; why: string }
export interface LiveTicket {
  fixture: LiveFixture;
  legs: LiveLeg[];
  combined_prob: number;
  combined_odds: number | null;
  rationale: string;
  confidence: string;
  model: string;
  cached: boolean;
  note: string;
}

export interface SgpPrice {
  correlated: number;
  independent: number;
  lift: number;
  fair_odds: number;
  legs: number;
  sims: number;
}

export interface ModelPurposeRow {
  model: string;
  purpose: string;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
}

export interface BuildResponse {
  result: BuildResult;
  meter: RequestMeter;
  usage: BuildUsage;
}

export interface UsageBreakdown {
  today: number;
  week: number;
  month: number;
  lifetime: number;
  today_tokens: number;
  week_tokens: number;
  lifetime_tokens: number;
  grok_today: number;
  grok_week: number;
  grok_month: number;
  grok_lifetime: number;
}

export interface LegResult {
  won: boolean | null;
  detail: string;
  margin?: number | null; // O/U: signed gap to the line (+ = cleared, − = missed by)
  void?: boolean; // book-refunded (didn't feature / postponed) — settled, counts neither way
}

export interface PlacedBet {
  id: number;
  created_at: number;
  day: string;
  ticket: Ticket;
  stake: number;
  status: string;
  returns: number;
  leg_results: LegResult[];
  settled: boolean;
  grok_used: boolean;
  ingest_used?: boolean;
  strategy: string;
  /// Closing-line value: avg (placed/close − 1) across priced legs. Positive =
  /// beat the close — the fastest-converging proof of real edge.
  clv?: number | null; // value | likely | board
}

export interface BankrollView {
  bankroll: number;
  staked_open: number;
  pnl: number;
  current: number;
  open_count: number;
  settled_count: number;
}

export interface FixtureInput {
  fixture_id: number;
  league_id: number;
  season: number;
  home_team_id: number;
  home_team: string;
  away_team_id: number;
  away_team: string;
  date_utc?: string;
  venue_city?: string | null;
  referee?: string | null;
}

export interface SelectionPick {
  fixture_id: number;
  league_id: number;
  season: number;
  player_id: number;
  team_id: number;
}

export interface BuildSelection {
  picks?: SelectionPick[];
  fixtures: FixtureInput[];
  markets: string[];
  reasoning: boolean;
  implied_prob: boolean;
  notes: string;
  model: string;
  ticket_count?: number;
  ticket_types: string[];
  variation: number;
  exclude: string[];
  bias_builders: boolean;
  most_likely: boolean;
  strategy: string; // value | favorites | likely
  max_leg_prob: number; // safety ceiling (1 = off)
  use_grok: boolean;
  grok_veto: boolean;
  grok_categories: string[];
  use_weather: boolean;
  use_standings: boolean;
  use_h2h: boolean;
  use_lineups: boolean;
  use_predictions: boolean;
  use_xg: boolean;
  use_tactics: boolean;
  lucky_safe: number;
  lucky_moderate: number;
  lucky_risky: number;
  use_ingest: boolean;
  min_legs: number | null;
  /// Every multi-leg ticket must include ≥1 leg from EVERY selected fixture.
  cover_all?: boolean;
  /// "over" | "under" | undefined = both sides.
  ou_side?: string | null;
  min_odds: number | null;
  max_odds: number | null;
  max_per_subject: number | null;
  use_plausibility: boolean;
  min_plausibility?: number | null;
  simple?: boolean;
  /// Deterministic forecast only — backend skips the model call (0 tokens).
  forecast_only?: boolean;
  /// Voided subjects (players/teams) to drop from the candidate pool.
  exclude_subjects?: string[];
}

export const TICKET_TYPES = ["Single", "SGP", "SGP+"];

/// ONE ticket-kind rule for every screen (Results/PicksBoard/CustomSlip used to
/// classify the same legs three different ways, and the label persists into the
/// ledger's by-type report). Matches the backend's reground rule: SGP+ needs a
/// same-game core (some fixture contributing 2+ legs); one leg per fixture is
/// just a cross-game Acca.
export function classifyTicket(matches: string[]): string {
  if (matches.length <= 1) return "Single";
  const counts: Record<string, number> = {};
  for (const m of matches) counts[m] = (counts[m] || 0) + 1;
  const nFix = Object.keys(counts).length;
  if (nFix <= 1) return "SGP";
  return Math.max(...Object.values(counts)) >= 2 ? "SGP+" : "Acca";
}

/// Shared strategy display names (was copy-pasted in Tracker + Ledger).
export function stratLabel(s: string): string {
  if (s.startsWith("dw:")) return `🧬 ${s.slice(3)}`; // Darwin paper variants
  if (s === "apex") return "Apex 🎯";
  if (s === "likely") return "Secret picks";
  if (s === "favorites") return "Form faves";
  if (s === "oracle") return "Oracle ✦";
  if (s === "power") return "Power Stacker ⚡";
  if (s === "bankers") return "Anchors ⚓";
  if (s === "jackpot") return "Jackpot 🎰";
  if (s === "predictor") return "Match Predictor 🔮";
  if (s === "scout") return "Scout 📡";
  if (s === "live") return "Live 🔴";
  if (s === "custom") return "Cherry-picked 🍒";
  if (s === "ladder") return "Acca ladder";
  if (s === "board") return "Board";
  return "Value +EV";
}

/// Shared probability formatter. Sub-1% shows "<1%" — Jackpot tickets target
/// ~1-5% hit chances and Math.round showed them as a broken-looking "0%".
export function pct(p?: number | null): string {
  if (p == null) return "—";
  if (p > 0 && p < 0.01) return "<1%";
  if (p < 0.05) return `${(p * 100).toFixed(1)}%`;
  return `${Math.round(p * 100)}%`;
}

export const GROK_CATEGORIES: { id: string; label: string }[] = [
  { id: "injuries", label: "Injuries" },
  { id: "news", label: "Team news" },
  { id: "bets", label: "Recommended bets" },
  { id: "analysis", label: "Analysis" },
  { id: "tactics", label: "Tactics / coach" },
  { id: "opinions", label: "Opinions" },
  { id: "predictions", label: "Predictions" },
];

// IANA zones for fixture times. Etc/GMT+5 == UTC-5 (sign is inverted in POSIX).
export const TIMEZONES: { id: string; label: string }[] = [
  { id: "Etc/GMT+5", label: "UTC−5 (default)" },
  { id: "America/New_York", label: "US Eastern (DST)" },
  { id: "America/Chicago", label: "US Central" },
  { id: "America/Denver", label: "US Mountain" },
  { id: "America/Los_Angeles", label: "US Pacific" },
  { id: "America/Bogota", label: "Bogotá / Lima (−5)" },
  { id: "Europe/London", label: "UK" },
  { id: "Europe/Madrid", label: "Central Europe" },
  { id: "UTC", label: "UTC" },
];

/// Fractional-Kelly recommended stake. Uses sharp (Pinnacle) prob where present,
/// else our model prob. Returns 0 when there's no priced edge to stake on.
export function kellyStake(
  legs: TicketLeg[],
  combinedOdds: number | null | undefined,
  bankroll: number,
  fraction: number
): number {
  if (!combinedOdds || combinedOdds <= 1 || bankroll <= 0 || fraction <= 0) return 0;
  let p = 1;
  for (const l of legs) {
    const lp = l.pinnacle_prob ?? l.est_prob;
    if (lp == null) return 0;
    p *= lp;
  }
  const b = combinedOdds - 1;
  const f = (b * p - (1 - p)) / b;
  if (f <= 0) return 0;
  const stake = Math.min(fraction * f * bankroll, bankroll * 0.25); // cap 25% bankroll
  return Math.round(stake * 100) / 100;
}

/// Stable identity for a single leg — for cherry-picking legs across tickets.
export function legKey(l: TicketLeg): string {
  return `${l.fixture_id ?? l.match}|${l.market}|${l.selection}|${l.line ?? ""}`;
}

/// Short 3-letter team badge, e.g. "Germany" → "GER", "Bayern Munich" → "BAY".
export function shortTeam(name?: string | null): string {
  if (!name) return "";
  const clean = name.replace(/[^A-Za-z ]/g, "").trim();
  if (!clean) return "";
  const words = clean.split(/\s+/);
  if (words.length >= 2) {
    return words.slice(0, 3).map((w) => w[0]).join("").toUpperCase();
  }
  return clean.slice(0, 3).toUpperCase();
}

// ---- data inspector ----
export interface TeamStatsView {
  played: number;
  gf_avg: number;
  ga_avg: number;
  ppg: number;
  first_half_share: number;
  fts_rate: number;
}

export interface PlayerLite {
  player_id: number;
  name: string;
  position: string;
  availability: string;
}

export interface InspectTeam {
  team_id: number;
  team_name: string;
  loaded: boolean;
  stats: TeamStatsView | null;
  players: PlayerLite[];
}

export interface InspectFixture {
  fixture_id: number;
  league_id: number;
  season: number;
  fixture_label: string;
  teams: InspectTeam[];
}

export interface PlayerRates {
  goals: number;
  sot: number;
  shots: number;
  tackles: number;
  fouls: number;
  cards: number;
  passes: number;
}

export interface PlayerInspect {
  name: string;
  position: string;
  apps: number;
  minutes: number;
  goals: number;
  shots: number;
  sot: number;
  tackles: number;
  fouls_committed: number;
  fouls_drawn: number;
  cards: number;
  passes: number;
  per90: PlayerRates;
}

export interface SavedTicket {
  id: number;
  created_at: number;
  result_json: string;
  user_notes: string;
}

export interface MarketDef {
  key: string;
  label: string;
  group: "player" | "team";
  sub?: string;
}

// Fetched once (cached) from /leagues, sorted by the user's pick history.
export interface LeagueOption {
  id: number;
  name: string;
  country: string;
  picks: number;
}

export const MARKETS: MarketDef[] = [
  { key: "scorer", label: "Anytime Scorer", group: "player", sub: "Attacking" },
  { key: "goalassist", label: "Score or Assist", group: "player", sub: "Attacking" },
  { key: "assists", label: "Anytime Assist", group: "player", sub: "Attacking" },
  { key: "sot", label: "Shots on Target", group: "player", sub: "Attacking" },
  { key: "pshots", label: "Player Shots", group: "player", sub: "Attacking" },
  { key: "tackles", label: "Tackles", group: "player", sub: "Involvement" },
  { key: "fouls", label: "Fouls Committed", group: "player", sub: "Involvement" },
  { key: "fdrawn", label: "Fouls Drawn (To Be Fouled)", group: "player", sub: "Involvement" },
  { key: "saves", label: "Goalkeeper Saves", group: "player", sub: "Involvement" },
  { key: "cards", label: "Cards", group: "player", sub: "Involvement" },
  { key: "win", label: "Match Result", group: "team", sub: "Result" },
  { key: "dc", label: "Double Chance", group: "team", sub: "Result" },
  { key: "half1", label: "1st-half result", group: "team", sub: "Result" },
  { key: "half2", label: "2nd-half result", group: "team", sub: "Result" },
  { key: "ou25", label: "Goals O/U (1.5 / 2.5 / 3.5)", group: "team", sub: "Goals" },
  { key: "h1goals", label: "1st-half Goals O/U", group: "team", sub: "Goals" },
  { key: "h2goals", label: "2nd-half Goals O/U", group: "team", sub: "Goals" },
  { key: "exactscore", label: "Correct Score", group: "team", sub: "Goals" },
  { key: "tgoals", label: "Team Total Goals", group: "team", sub: "Goals" },
  { key: "btts", label: "BTTS", group: "team", sub: "Goals" },
  { key: "mcorners", label: "Match Corners — combined", group: "team", sub: "Involvement" },
  { key: "mcards", label: "Match Cards — combined", group: "team", sub: "Involvement" },
  { key: "mshots", label: "Match Shots — combined", group: "team", sub: "Involvement" },
  { key: "msot", label: "Match Shots on Target — combined", group: "team", sub: "Involvement" },
  { key: "tcorners", label: "Team Corners — one team (recent form)", group: "team", sub: "Goals" },
  { key: "tshots", label: "Team Shots — one team (recent form)", group: "team", sub: "Goals" },
  { key: "toffsides", label: "Team Offsides — one team (recent form)", group: "team", sub: "Involvement" },
  { key: "tcards", label: "Team Cards — one team", group: "team", sub: "Involvement" },
  { key: "bothcards", label: "Both Teams Carded", group: "team", sub: "Involvement" },
  { key: "mostcards", label: "Most Cards (which team)", group: "team", sub: "Involvement" },
  { key: "mostcorners", label: "Most Corners (which team)", group: "team", sub: "Involvement" },
  { key: "mostshots", label: "Most Shots (which team)", group: "team", sub: "Involvement" },
];

/// Opponent-strength index — one team's league-relative factors.
export interface TeamFactors {
  atk_goals: number;
  def_goals: number;
  atk_shots: number;
  def_shots: number;
  atk_corners: number;
  def_corners: number;
  sos_atk: number;
  sos_def: number;
  games: number;
}
export interface TeamIndexView {
  team_id: number;
  name: string;
  factors: TeamFactors;
  games: number;
}
export interface IndexLeagueView {
  league_id: number;
  season: number;
  teams: number;
  built_at: number;
}
export interface TeamPerfRow {
  team_id: number;
  team_name: string;
  league_id: number;
  family: string;
  games: number;
  avg_predicted: number;
  avg_actual: number;
  ratio: number;
}

/// One team's starting XI in the Data tab viewer.
export interface LineupSide {
  team: string;
  source: string; // "api" | "ingested" | "none"
  players: string[];
}
export interface LineupView {
  fixture_id: number;
  label: string;
  sides: LineupSide[];
}

/// Ledger market display name → selectable market-group key (best-markets picker).
export function marketNameToKey(market: string): string | null {
  const m = market.toLowerCase();
  if (m.includes("multi scorer") || m.includes("anytime scorer")) return "scorer";
  if (m.includes("score or assist")) return "goalassist";
  if (m.includes("anytime assist")) return "assists";
  if (m.includes("shots on target")) return "sot";
  if (m.includes("player shots")) return "pshots";
  if (m.includes("tackles")) return "tackles";
  if (m.includes("fouls drawn") || m.includes("to be fouled")) return "fdrawn";
  if (m.includes("fouls")) return "fouls";
  if (m.includes("passes")) return "passes";
  if (m.includes("saves")) return "saves";
  if (m.includes("most corners")) return "mostcorners";
  if (m.includes("most shots")) return "mostshots";
  if (m.includes("match corners")) return "mcorners";
  if (m.includes("match cards")) return "mcards";
  if (m.includes("match shots on target")) return "msot";
  if (m.includes("match shots")) return "mshots";
  if (m.includes("to be carded")) return "cards";
  if (m.includes("both teams carded")) return "bothcards";
  if (m.includes("most cards")) return "mostcards";
  if (m.includes("team total cards") || m.includes("team cards")) return "tcards";
  if (m.includes("team corners")) return "tcorners";
  if (m.includes("team shots")) return "tshots";
  if (m.includes("team offsides")) return "toffsides";
  if (m.includes("team total goals")) return "tgoals";
  if (m.includes("correct score")) return "exactscore";
  if (m.includes("btts")) return "btts";
  if (m.includes("match result") || m.includes("double chance")) return "win";
  if (m.includes("1st half")) return "h1goals";
  if (m.includes("2nd half")) return "h2goals";
  if (m.includes("goals")) return "ou25"; // Over/Under X.5 Goals
  return null;
}

/// Plain-English one-liners for every strategy in the Ledger — what the rows
/// actually MEAN (written from the real selection rules in the code).
export function stratDescription(s: string): string {
  const m: Record<string, string> = {
    "dw:sharp2": "Singles where the takeable price beats Pinnacle's de-vigged truth by ≥2% — the classic sharp edge at a low bar.",
    "dw:sharp5": "Same sharp edge but stricter: only ≥5% EV singles. Compares with sharp2 to find which EV bar survives the vig.",
    "dw:formgap": "Players whose RECENT hit-rate runs far above their season rate (role change the book hasn't repriced yet), ≥50% picks.",
    "dw:lineroom": "Overs on count markets (corners/shots/cards) whose own settled history clears the line ≥75% of the time — mining lines set too low.",
    "dw:corrlift": "The Monte-Carlo copula's best correlated same-game combos (legs that reinforce each other more than the book charges for).",
    "dw:shooters": "One shots/SOT leg (≥55%) from EACH match — a cross-game acca of truly independent legs at the product price.",
    "dw:chalk3": "A treble of short-priced favourites (1.25-1.60) from different games — tests whether chalk out-earns its drag.",
    "dw:contra-under": "Unders where OUR model sees less scoring than the sharp line implies (fading the public's over-bias).",
    value: "Model +EV picks — price beats our estimated true probability.",
    likely: "Highest-probability picks with real-world context, ignoring price.",
    favorites: "In-form favourites at useful odds (1.5-2.5), no chalk, no longshots.",
    oracle: "Only picks where sharp price, our model AND a real edge all agree.",
    power: "Cross-game doubles of likely-but-generously-priced legs (4x+ combined).",
    bankers: "High-likelihood recurring events (booked regulars, reliable shooters) — reliability over price.",
    jackpot: "Deliberate 20-150x lotteries built from individually-plausible legs.",
    predictor: "Deep single-match read — themed same-game builds from every market.",
    scout: "Your ingested pages fused with our data — picks both sources support.",
    stacker: "Stat-led 5-8 leg stacks; steps chalk up to the next plausible line.",
    powerof3: "Per-game ~3x blocks of very likely legs, compounded across all matches.",
    mega: "One giant acca — best 2-3 sweet-spot legs from every match.",
    ladder: "Deterministic accumulator ladder built by hit-chance bands (no AI).",
    custom: "Legs you cherry-picked by hand.",
    live: "In-play builds from live state.",
  };
  return m[s] ?? "";
}

/// PropLine (US sports) event + evidence row.
export interface PlEvent {
  id: string;
  sport_key: string;
  home_team: string;
  away_team: string;
  commence_time: string;
  live: boolean;
}
export interface PlPick {
  fixture: string;
  market: string;
  subject: string;
  side: string;
  odds?: number | null;
  book?: string | null;
  sharp?: number | null;
  probability?: number | null;
  implied?: number | null;
  hit_chance?: number | null;
}
