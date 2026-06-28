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
  has_parlay_key: boolean;
  model: string;
  books: string[];
  kelly_fraction: number;
  timezone: string;
  proxy_url: string;
  has_proxy_token: boolean;
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
  { id: "claude-opus-4-8", label: "Opus 4.8", note: "$5 / $25 per 1M — sharpest" },
  { id: "claude-sonnet-4-6", label: "Sonnet 4.6", note: "$3 / $15 per 1M — balanced" },
  { id: "claude-haiku-4-5", label: "Haiku 4.5", note: "$1 / $5 per 1M — cheapest" },
];

// Models for the per-ticket quick analysis (a second angle). GPT needs an OpenAI key.
export const ANALYSIS_MODELS = [
  { id: "claude-haiku-4-5", label: "Haiku", provider: "claude" },
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

export interface BuildResult {
  tickets: Ticket[];
  data_quality_notes: string[];
  context_notes?: string[];
  from_cache: boolean;
  grok_used?: boolean;
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
}

export interface MarketReportRow {
  market: string;
  settled: number;
  won: number;
  hit_rate: number; // actual
  predicted: number; // model's mean predicted prob
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
  strategy: string; // value | likely | board
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
  min_odds: number | null;
  max_odds: number | null;
  max_per_subject: number | null;
  use_plausibility: boolean;
}

export const TICKET_TYPES = ["Single", "SGP", "SGP+"];

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
  { key: "assists", label: "Anytime Assist", group: "player", sub: "Attacking" },
  { key: "sot", label: "Shots on Target", group: "player", sub: "Attacking" },
  { key: "pshots", label: "Player Shots", group: "player", sub: "Attacking" },
  { key: "tackles", label: "Tackles", group: "player", sub: "Involvement" },
  { key: "fouls", label: "Fouls", group: "player", sub: "Involvement" },
  { key: "cards", label: "Cards", group: "player", sub: "Involvement" },
  { key: "passes", label: "Passes", group: "player", sub: "Involvement" },
  { key: "win", label: "Match Result", group: "team", sub: "Result" },
  { key: "dc", label: "Double Chance", group: "team", sub: "Result" },
  { key: "ahandicap", label: "Asian Handicap", group: "team", sub: "Result" },
  { key: "half1", label: "1st-half result", group: "team", sub: "Result" },
  { key: "half2", label: "2nd-half result", group: "team", sub: "Result" },
  { key: "ou25", label: "Goals O/U (1.5 / 2.5 / 3.5)", group: "team", sub: "Goals" },
  { key: "h1goals", label: "1st-half Goals O/U", group: "team", sub: "Goals" },
  { key: "h2goals", label: "2nd-half Goals O/U", group: "team", sub: "Goals" },
  { key: "exactscore", label: "Correct Score", group: "team", sub: "Goals" },
  { key: "goalsrange", label: "Goals Range (0-1 / 2-3 / 4-6)", group: "team", sub: "Goals" },
  { key: "tgoals", label: "Team Total Goals", group: "team", sub: "Goals" },
  { key: "btts", label: "BTTS", group: "team", sub: "Goals" },
  { key: "tcorners", label: "Team Corners (recent form)", group: "team", sub: "Goals" },
  { key: "tshots", label: "Team Shots (recent form)", group: "team", sub: "Goals" },
  { key: "toffsides", label: "Team Offsides (recent form)", group: "team", sub: "Involvement" },
  { key: "saves", label: "Goalkeeper Saves", group: "player", sub: "Involvement" },
];
