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
  InspectFixture,
  IngestInfo,
  IngestItem,
  LeagueOption,
  MarketReportRow,
  ModelPurposeRow,
  PlacedBet,
  PlayerInspect,
  RequestMeter,
  SavedTicket,
  SettingsView,
  TeamSquad,
  TeamStatsView,
  Ticket,
  TicketEval,
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
    parlayKey: string | null,
    model: string | null,
    dailyLimit: number | null,
    books: string[] | null,
    kellyFraction: number | null,
    timezone: string | null,
    proxyUrl: string | null,
    proxyToken: string | null,
    ingestEnabled: boolean | null
  ) =>
    invoke<SettingsView>("save_settings", {
      apiFootballKey,
      anthropicKey,
      grokKey,
      openaiKey,
      parlayKey,
      model,
      dailyLimit,
      books,
      kellyFraction,
      timezone,
      proxyUrl,
      proxyToken,
      ingestEnabled,
    }),

  calibration: () => invoke<CalibrationReport>("calibration"),

  listGrokLog: () => invoke<GrokLogEntry[]>("list_grok_log"),

  getMeter: () => invoke<RequestMeter>("get_meter"),

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
    maxOdds: number | null
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
    }),

  prewarmPlausibility: (fixture: FixtureInput, markets: string[]) =>
    invoke<number>("prewarm_plausibility", { fixture, markets }),

  settleGenerated: () => invoke<GenReportRow[]>("settle_generated"),
  generatedReport: () => invoke<GenReportRow[]>("generated_report"),
  generatedReportByKind: () => invoke<GenReportRow[]>("generated_report_by_kind"),
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

  exportData: () => invoke<string>("export_data"),
  importData: (json: string) => invoke<number>("import_data", { json }),
  resetData: () => invoke<void>("reset_data"),

  usageByPurpose: () => invoke<ModelPurposeRow[]>("usage_by_purpose"),
  exportExtension: () => invoke<string>("export_extension"),

  ingestInfo: () => invoke<IngestInfo>("ingest_info"),
  listIngested: () => invoke<IngestItem[]>("list_ingested"),
  processIngested: (id: number, model?: string) =>
    invoke<IngestItem>("process_ingested", { id, model: model ?? null }),
  deleteIngested: (id: number) => invoke<void>("delete_ingested", { id }),
  updateIngestNote: (id: number, note: string) => invoke<void>("update_ingest_note", { id, note }),

  getBankroll: () => invoke<BankrollView>("get_bankroll"),
  setBankroll: (amount: number) => invoke<BankrollView>("set_bankroll", { amount }),

  placeBet: (
    ticket: Ticket,
    stake: number,
    odds: number | null,
    grokUsed: boolean,
    strategy: string
  ) => invoke<number>("place_bet", { ticket, stake, odds, grokUsed, strategy }),
  listBets: () => invoke<PlacedBet[]>("list_bets"),
  deleteBet: (id: number) => invoke<void>("delete_bet", { id }),
  settleBet: (id: number) => invoke<PlacedBet>("settle_bet", { id }),
  settleAll: () => invoke<PlacedBet[]>("settle_all"),
};
