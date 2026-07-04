// Thin typed wrappers around the Tauri commands. The frontend ONLY talks to
// these — never to an external API directly.

import { invoke } from "@tauri-apps/api/core";
import type {
  BankrollView,
  BuildResponse,
  BuildResult,
  BuildSelection,
  CalibrationReport,
  Candidate,
  Fixture,
  GenReportRow,
  GrokLogEntry,
  IndexLeagueView,
  InspectFixture,
  IngestInfo,
  IngestItem,
  LeagueOption,
  LineupView,
  LiveFixture,
  LiveSnapshot,
  LiveTicket,
  MarketReportRow,
  ModelPurposeRow,
  PlacedBet,
  PlayerInspect,
  RequestMeter,
  SavedTicket,
  SettingsView,
  SgpPrice,
  TeamIndexView,
  TeamPerfRow,
  TeamSquad,
  TeamStatsView,
  Ticket,
  TicketEval,
  TicketLeg,
  UsageBreakdown,
  FixtureInput,
} from "./types";

export const api = {
  getSettings: () => invoke<SettingsView>("get_settings"),

  saveSettings: (
    apiFootballKey: string | null,
    anthropicKey: string | null,
    grokKey: string | null,
    openaiKey: string | null,
    deepseekKey: string | null,
    parlayKey: string | null,
    model: string | null,
    dailyLimit: number | null,
    books: string[] | null,
    kellyFraction: number | null,
    defaultStake: number | null,
    timezone: string | null,
    proxyUrl: string | null,
    proxyToken: string | null,
    ingestEnabled: boolean | null,
    useTeamIndex: boolean | null = null
  ) =>
    invoke<SettingsView>("save_settings", {
      apiFootballKey,
      anthropicKey,
      grokKey,
      openaiKey,
      deepseekKey,
      parlayKey,
      model,
      dailyLimit,
      books,
      kellyFraction,
      defaultStake,
      timezone,
      proxyUrl,
      proxyToken,
      ingestEnabled,
      useTeamIndex,
    }),

  calibration: () => invoke<CalibrationReport>("calibration"),

  listGrokLog: () => invoke<GrokLogEntry[]>("list_grok_log"),

  getMeter: () => invoke<RequestMeter>("get_meter"),
  // Interrupt an in-flight build (stops at the next checkpoint / aborts the model call).
  cancelBuild: () => invoke<void>("cancel_build"),
  // ↻ Force-fresh odds + lineups + injuries for the selected fixtures (~3 req/match).
  refreshFixtureData: (fixtures: FixtureInput[]) => invoke<number>("refresh_fixture_data", { fixtures }),
  // 👥 Starting XIs (API feed → ingested page → none) for the Data tab.
  getLineups: (fixtures: FixtureInput[]) => invoke<LineupView[]>("get_lineups", { fixtures }),

  fetchLeagues: () => invoke<LeagueOption[]>("fetch_leagues"),

  bumpLeagues: (ids: number[]) => invoke<void>("bump_leagues", { ids }),

  fetchFixtures: (date: string, timezone?: string, league?: number, season?: number) =>
    invoke<Fixture[]>("fetch_fixtures", {
      date,
      league: league ?? null,
      season: season ?? null,
      timezone: timezone ?? null,
    }),

  fetchSquads: (fixtures: FixtureInput[]) =>
    invoke<TeamSquad[]>("fetch_squads", { fixtures }),

  buildTickets: (selection: BuildSelection) =>
    invoke<BuildResponse>("build_tickets", { selection }),

  getPicks: (fixtures: FixtureInput[], markets: string[]) =>
    invoke<Candidate[]>("get_picks", { fixtures, markets }),

  buildLadder: (
    fixtures: FixtureInput[],
    markets: string[],
    count: number,
    minProb: number,
    scope: string,
    maxLegs: number,
    minHit: number,
    maxPerSubject: number,
    ouSide: string,
    minLegs: number,
    excludeSigs: string[],
    excludeSubjects: string[],
    seedSubjects: string[],
    variation: number,
    minOdds: number | null,
    maxOdds: number | null,
    onePerFixture = false,
    mega = false,
    coverAll = false,
    coverLegs = 1
  ) =>
    invoke<BuildResult>("build_ladder", {
      fixtures,
      markets,
      count,
      minProb,
      scope,
      maxLegs,
      minHit,
      maxPerSubject,
      ouSide,
      minLegs,
      excludeSigs,
      excludeSubjects,
      seedSubjects,
      variation,
      minOdds,
      maxOdds,
      onePerFixture,
      mega,
      coverAll,
      coverLegs,
    }),

  prewarmPlausibility: (fixture: FixtureInput, markets: string[]) =>
    invoke<number>("prewarm_plausibility", { fixture, markets }),

  settleGenerated: () => invoke<GenReportRow[]>("settle_generated"),
  generatedReport: (sinceDays?: number | null) => invoke<GenReportRow[]>("generated_report", { sinceDays: sinceDays ?? null }),
  // 🧬 Darwin: paper-trade a population of deterministic micro-strategies (0 tokens).
  darwinSweep: (fixtures: FixtureInput[], markets: string[]) =>
    invoke<string[]>("darwin_sweep", { fixtures, markets }),
  generatedReportByKind: () => invoke<GenReportRow[]>("generated_report_by_kind"),
  // A/B: does ingested data help? (paper ledger, void-aware, windowed)
  generatedIngestSplit: (sinceDays?: number | null) =>
    invoke<GenReportRow[]>("generated_ingest_split", { sinceDays: sinceDays ?? null }),
  generatedReportByMarket: () => invoke<MarketReportRow[]>("generated_report_by_market"),

  evaluateTickets: (
    tickets: Ticket[],
    model: string | null,
    leagues?: Record<number, string>
  ) => invoke<TicketEval[]>("evaluate_tickets", { tickets, model, leagues: leagues ?? null }),

  saveTicket: (selectionJson: string, resultJson: string, notes: string) =>
    invoke<number>("save_ticket", { selectionJson, resultJson, notes }),

  listTickets: () => invoke<SavedTicket[]>("list_tickets"),

  inspectFixtures: (fixtures: FixtureInput[]) =>
    invoke<InspectFixture[]>("inspect_fixtures", { fixtures }),

  inspectPlayer: (playerId: number, leagueId: number, season: number) =>
    invoke<PlayerInspect | null>("inspect_player", { playerId, leagueId, season }),

  inspectTeamStats: (teamId: number, leagueId: number, season: number) =>
    invoke<TeamStatsView | null>("inspect_team_stats", { teamId, leagueId, season }),

  usageBreakdown: () => invoke<UsageBreakdown>("usage_breakdown"),

  // Opponent-strength index: manual per-league build, audit, recalibrate.
  buildTeamIndex: (leagueId: number, season: number) =>
    invoke<IndexLeagueView>("build_team_index", { leagueId, season }),
  listTeamIndex: () => invoke<IndexLeagueView[]>("list_team_index"),
  indexLeagueTeams: (leagueId: number, season: number) =>
    invoke<TeamIndexView[]>("index_league_teams", { leagueId, season }),
  resetTeamIndex: (leagueId?: number | null) => invoke<number>("reset_team_index", { leagueId: leagueId ?? null }),
  exportTeamIndex: () => invoke<string>("export_team_index"),
  indexReview: () => invoke<TeamPerfRow[]>("index_review"),
  recalibrateIndex: () => invoke<string>("recalibrate_index"),

  exportData: () => invoke<string>("export_data"),
  importData: (json: string) => invoke<number>("import_data", { json }),
  resetData: () => invoke<void>("reset_data"),

  liveFixtures: () => invoke<LiveFixture[]>("live_fixtures"),
  liveSnapshot: (fixture: LiveFixture) => invoke<LiveSnapshot>("live_snapshot", { fixture }),
  liveTicket: (fixture: LiveFixture, model: string) => invoke<LiveTicket>("live_ticket", { fixture, model }),
  priceSgp: (legs: TicketLeg[]) => invoke<SgpPrice>("price_sgp", { legs }),
  getBankers: (fixtures: FixtureInput[], markets: string[]) =>
    invoke<Candidate[]>("get_bankers", { fixtures, markets }),

  usageByPurpose: () => invoke<ModelPurposeRow[]>("usage_by_purpose"),
  exportExtension: () => invoke<string>("export_extension"),

  ingestInfo: () => invoke<IngestInfo>("ingest_info"),
  listIngested: () => invoke<IngestItem[]>("list_ingested"),
  processIngested: (id: number, model?: string) =>
    invoke<IngestItem>("process_ingested", { id, model: model ?? null }),
  deleteIngested: (id: number) => invoke<void>("delete_ingested", { id }),
  updateIngestNote: (id: number, note: string) => invoke<void>("update_ingest_note", { id, note }),
  assignIngestFixture: (id: number, label: string, date?: string) => invoke<void>("assign_ingest_fixture", { id, label, date }),
  // 🩹 One cached DeepSeek call re-matches badly-extracted page names to real fixtures.
  fixIngestNames: (fixtures: FixtureInput[]) => invoke<number>("fix_ingest_names", { fixtures }),

  getBankroll: () => invoke<BankrollView>("get_bankroll"),
  setBankroll: (amount: number) => invoke<BankrollView>("set_bankroll", { amount }),

  placeBet: (
    ticket: Ticket,
    stake: number,
    odds: number | null,
    grokUsed: boolean,
    ingestUsed: boolean,
    strategy: string,
    indexUsed = false
  ) => invoke<number>("place_bet", { ticket, stake, odds, grokUsed, ingestUsed, indexUsed, strategy }),
  listBets: () => invoke<PlacedBet[]>("list_bets"),
  deleteBet: (id: number) => invoke<void>("delete_bet", { id }),
  settleBet: (id: number) => invoke<PlacedBet>("settle_bet", { id }),
  settleAll: () => invoke<PlacedBet[]>("settle_all"),
  // Add odds to an all-green open bet, then settle it at the real price.
  setBetOdds: (id: number, odds: number) => invoke<PlacedBet>("set_bet_odds", { id, odds }),
};
