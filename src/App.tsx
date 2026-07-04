import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "./api";
import {
  GROK_CATEGORIES,
  MARKETS,
  TICKET_TYPES,
  type BuildResult,
  type BuildUsage,
  type Fixture,
  type FixtureInput,
  type LeagueOption,
  type RequestMeter,
  type SettingsView,
  type Ticket,
  type TicketLeg,
  type UsageBreakdown,
  legKey,
} from "./types";
import Results from "./components/Results";
import CustomSlip from "./components/CustomSlip";
import Spinner from "./components/Spinner";
import Settings from "./components/Settings";
import History from "./components/History";
import Inspector from "./components/Inspector";
import Lineups from "./components/Lineups";
import MyPicks from "./components/MyPicks";
import IngestedStats from "./components/IngestedStats";
import Tracker from "./components/Tracker";
import Newsfeed from "./components/Newsfeed";
import Ledger from "./components/Ledger";
import Ingest from "./components/Ingest";
import Live from "./components/Live";
import PicksBoard from "./components/PicksBoard";
import { Toaster, toast } from "./toast";
import ErrorBoundary from "./components/ErrorBoundary";

export default function App() {
  return (
    <>
      <ErrorBoundary>
        <AppInner />
      </ErrorBoundary>
      <Toaster />
    </>
  );
}

type Step = "date" | "matches" | "markets" | "results";
type Overlay = "settings" | "history" | "tracker" | "newsfeed" | "ledger" | "ingest" | "live" | null;

function fmtDate(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}
function todayStr(): string {
  return fmtDate(new Date()); // local calendar date, not UTC
}
function addDays(date: string, n: number): string {
  const d = new Date(`${date}T00:00:00`);
  d.setDate(d.getDate() + n);
  return fmtDate(d);
}
// The fixture's calendar date in a given timezone (YYYY-MM-DD).
function tzDate(iso: string, tz: string): string {
  try {
    return new Intl.DateTimeFormat("en-CA", {
      timeZone: tz,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
    }).format(new Date(iso));
  } catch {
    return iso.slice(0, 10);
  }
}

const playerMarketKeys = MARKETS.filter((m) => m.group === "player").map((m) => m.key);
const allMarketKeys = MARKETS.map((m) => m.key);

// Derive status from kickoff: "live" for ~the 90 (+ stoppage) window after
// kickoff, "ended" once finished (by feed status or >130' elapsed).
function liveInfo(f: Fixture): { state: "scheduled" | "live" | "ended"; label: string } {
  const mins = Math.floor((Date.now() - new Date(f.date_utc).getTime()) / 60000);
  if (["FT", "AET", "PEN"].includes(f.status) || mins > 130) {
    return { state: "ended", label: "Ended" };
  }
  if (mins >= 0 && mins <= 130) {
    return { state: "live", label: mins >= 90 ? "LIVE 90'+" : `LIVE ${mins + 1}'` };
  }
  return { state: "scheduled", label: "" };
}

// Build local YYYY-MM-DD strings (NOT via toISOString, which shifts to UTC and
// can move the date a day for timezones ahead of/behind UTC).
function dateRange(start: string, days: number): string[] {
  const fmt = (d: Date) =>
    `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  const out: string[] = [];
  for (let i = 0; i < days; i++) {
    const d = new Date(`${start}T00:00:00`);
    d.setDate(d.getDate() + i);
    out.push(fmt(d));
  }
  return out;
}

function localTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
}

function MeterBar({ meter }: { meter: RequestMeter | null }) {
  if (!meter) return null;
  const pct = Math.min(100, Math.round((meter.count / meter.limit) * 100));
  const near = pct >= 80;
  const tone = pct >= 100 ? "text-bad" : near ? "text-warn" : "text-slate-400";
  return (
    <span
      className={`text-xs ${tone} ${near ? "font-semibold" : ""}`}
      title={`API requests today: ${meter.count} / ${meter.limit}${pct >= 100 ? " — fresh calls blocked until tomorrow" : near ? " — approaching the daily budget" : ""}`}
    >
      {near ? `${pct >= 100 ? "🚫" : "⚠"} ${meter.count}/${meter.limit}` : `${pct}%`}
    </span>
  );
}

function Coins({ claude, grok, onClick }: { claude: number; grok: number; onClick?: () => void }) {
  return (
    <button
      className="text-xs hover:text-slate-100 flex items-center gap-2"
      title="Claude + Grok spend — click for today / week / month / lifetime"
      onClick={onClick}
    >
      <span className="text-slate-400">🪙 ${claude.toFixed(2)}</span>
      <span className="text-slate-500">🔍 ${grok.toFixed(2)}</span>
    </button>
  );
}

function CostRow({ label, claude }: { label: string; claude: number }) {
  return (
    <div className="flex justify-between">
      <span className="text-slate-400">{label}</span>
      <span className="text-slate-100">${claude.toFixed(2)}</span>
    </div>
  );
}

function AppInner() {
  const [step, setStep] = useState<Step>("date");
  const [overlay, setOverlay] = useState<Overlay>(null);
  const warnedMeter = useRef(false);

  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [meter, setMeter] = useState<RequestMeter | null>(null);

  // Count of ingested pages not yet processed by AI — for the 🧲 nav badge.
  const [ingestPending, setIngestPending] = useState(0);
  useEffect(() => {
    const tick = () =>
      api
        .listIngested()
        .then((items) => setIngestPending(items.filter((i) => i.status !== "processed").length))
        .catch(() => {});
    tick();
    const h = setInterval(tick, 15000);
    return () => clearInterval(h);
  }, [overlay]);

  // One-time nudge when the daily request budget crosses 80%.
  useEffect(() => {
    if (!meter || meter.limit <= 0) return;
    const frac = meter.count / meter.limit;
    if (frac >= 0.8 && frac < 1 && !warnedMeter.current) {
      warnedMeter.current = true;
      toast.info(`Request budget at ${Math.round(frac * 100)}% (${meter.count}/${meter.limit}) — fresh data calls stop at the limit.`);
    }
    if (frac < 0.8) warnedMeter.current = false;
  }, [meter]);

  const [date, setDate] = useState(todayStr());
  const [days, setDays] = useState(1);
  const [leagues, setLeagues] = useState<LeagueOption[]>([]);
  const [leagueSearch, setLeagueSearch] = useState("");
  const [selLeagues, setSelLeagues] = useState<Set<number>>(new Set());
  const [fixtures, setFixtures] = useState<Fixture[]>([]);
  const [selFixtureIds, setSelFixtureIds] = useState<Set<number>>(new Set());

  const [selMarkets, setSelMarkets] = useState<Set<string>>(new Set(["scorer"]));
  // Named MARKET presets — persisted locally; tap to apply, × to delete,
  // "＋ Save current…" stores the current selection under a name. Ships with
  // the old Quick-mode combos as starter presets (deletable).
  type MarketPreset = { name: string; markets: string[] };
  const DEFAULT_PRESETS: MarketPreset[] = [
    { name: "⚽ Scorers only", markets: ["scorer"] },
    { name: "🎯 Score + Assist + Result", markets: ["scorer", "assists", "win"] },
    { name: "Player props", markets: [...playerMarketKeys] },
  ];
  const [marketPresets, setMarketPresets] = useState<MarketPreset[]>(() => {
    try {
      const raw = localStorage.getItem("powabet.marketPresets");
      if (raw) return JSON.parse(raw) as MarketPreset[];
    } catch {
      // corrupted → fall back to defaults
    }
    return DEFAULT_PRESETS;
  });
  function persistPresets(next: MarketPreset[]) {
    setMarketPresets(next);
    try {
      localStorage.setItem("powabet.marketPresets", JSON.stringify(next));
    } catch {
      // storage full/unavailable — presets stay for this session only
    }
  }
  const sameMarkets = (mk: string[]) => mk.length === selMarkets.size && mk.every((k) => selMarkets.has(k));
  // Inline naming UI — window.prompt() does NOT exist in Tauri's webview (the
  // native dialogs are disabled), so the save button reveals an input instead.
  const [presetNaming, setPresetNaming] = useState(false);
  const [presetName, setPresetName] = useState("");
  function savePreset() {
    const name = presetName.trim();
    if (!name || selMarkets.size === 0) return;
    const next = marketPresets.filter((p) => p.name !== name); // same name = overwrite
    persistPresets([...next, { name, markets: [...selMarkets] }]);
    toast.success(`Preset “${name}” saved (${selMarkets.size} market${selMarkets.size > 1 ? "s" : ""}).`);
    setPresetName("");
    setPresetNaming(false);
  }
  function deletePreset(name: string) {
    persistPresets(marketPresets.filter((p) => p.name !== name));
  }
  const [reasoning, setReasoning] = useState(true);
  const [impliedProb, setImpliedProb] = useState(true);
  const [notes, setNotes] = useState("");
  const [ticketCount, setTicketCount] = useState(10);
  // Singles off by default — stacking is where the value is.
  const [ticketTypes, setTicketTypes] = useState<Set<string>>(new Set(TICKET_TYPES.filter((t) => t !== "Single")));
  const [biasBuilders, setBiasBuilders] = useState(false);
  // Simple vs Advanced mode. Remembered across launches only if "remember" is on.
  const [mode, setMode] = useState<"simple" | "advanced">(() => (localStorage.getItem("pb_mode") as "simple" | "advanced") || "simple");
  const [rememberMode, setRememberMode] = useState(() => localStorage.getItem("pb_mode") != null);
  function chooseMode(m: "simple" | "advanced") {
    setMode(m);
    if (rememberMode) localStorage.setItem("pb_mode", m);
  }
  function toggleRemember(on: boolean) {
    setRememberMode(on);
    if (on) localStorage.setItem("pb_mode", mode);
    else localStorage.removeItem("pb_mode");
  }
  // Simple-mode risk dial — best-in-class presets so it stays one-tap.
  const [simpleRisk, setSimpleRisk] = useState<"safe" | "balanced" | "bold" | "scout">("balanced");
  const [strategy, setStrategy] = useState("value");
  // Match Predictor only works with one fixture — fall back if that changes.
  useEffect(() => {
    if (strategy === "predictor" && selFixtureIds.size !== 1) setStrategy("value");
  }, [selFixtureIds, strategy]);
  const [maxLegProb, setMaxLegProb] = useState(1);
  const [useGrok, setUseGrok] = useState(false);
  const [grokVeto, setGrokVeto] = useState(true);
  const [grokCats, setGrokCats] = useState<Set<string>>(new Set(["injuries", "news"]));
  // Weather defaults OFF — low-value token clutter; Grok's digest mentions
  // weather when it actually matters. Still toggleable in Advanced.
  const [useWeather, setUseWeather] = useState(false);
  const [useStandings, setUseStandings] = useState(true);
  const [useH2h, setUseH2h] = useState(true);
  const [useLineups, setUseLineups] = useState(true);
  const [usePredictions, setUsePredictions] = useState(true);
  const [useXg, setUseXg] = useState(true);
  const [useTactics, setUseTactics] = useState(true);
  // Min plausibility (1-5) filter — 1 = show everything, higher = only realistic lines.
  const [minPlausibility, setMinPlausibility] = useState(1);
  // Advanced expander for the redundant strategies + data-source knobs.
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [ladderCount, setLadderCount] = useState(5);
  const [ladderMinProb, setLadderMinProb] = useState(0.55);
  const [ladderScope, setLadderScope] = useState("mixed");
  const [ladderMaxLegs, setLadderMaxLegs] = useState(8);
  const [ladderMinHit, setLadderMinHit] = useState(0.05);
  const [ladderMaxPerSubject, setLadderMaxPerSubject] = useState(2);
  const [ladderOuSide, setLadderOuSide] = useState("auto");
  const [ladderMinLegs, setLadderMinLegs] = useState(2);
  // Diversified cross-game acca: max ONE leg per match (truly independent legs).
  const [ladderOnePerFixture, setLadderOnePerFixture] = useState(false);
  // 🚀 Mega acca: ONE giant ticket — best 2-3 legs from every match, each leg
  // in the 1.2-2.0 sweet spot (or the odds band when set).
  const [ladderMega, setLadderMega] = useState(false);
  // 🧩 Cover-all: every ticket spans EVERY selected match (≥ N legs each);
  // the min-hit target is ignored while on (it would be deceiving).
  const [coverAll, setCoverAll] = useState(false);
  const [coverLegs, setCoverLegs] = useState(1);
  // ↻ Manual data refresh: odds/lineups/injuries go stale near kickoff.
  const [refreshing, setRefreshing] = useState(false);
  const [darwinBusy, setDarwinBusy] = useState(false);
  const darwinRef = useRef(false); // sync double-click guard (state updates lag a fast second click)
  // Darwin sweep report — stays on screen until dismissed (a toast vanished
  // before anyone could read what the sweep actually recorded).
  const [darwinReport, setDarwinReport] = useState<string[] | null>(null);
  const [ladderDiversityReset, setLadderDiversityReset] = useState(true);
  const [ladderVariation, setLadderVariation] = useState(0);
  // Shared per-leg odds sweet-spot (applies to regular + acca builds). 1 / 1000 = off.
  const [oddsMin, setOddsMin] = useState(1.2); // skip near-certain short legs by default
  const [oddsMax, setOddsMax] = useState(1000);
  // Regular build: minimum legs per ticket (4-fold etc.) and diversity cap.
  const [regMinLegs, setRegMinLegs] = useState(1);
  const [regMaxPerSubject, setRegMaxPerSubject] = useState(0); // 0 = model default
  const [usePlausibility] = useState(true); // Haiku pre-score — always on (filter via slider)
  const [useIngest, setUseIngest] = useState(true); // feed ingested page data into builds
  const [prewarmBusy, setPrewarmBusy] = useState(false);
  const [prewarmProgress, setPrewarmProgress] = useState<{ done: number; total: number } | null>(null);
  const prewarmedRef = useRef<string>("");
  // Players the user voided in the current results — kept out of "add more" tickets.
  const [voidedSubjects, setVoidedSubjects] = useState<Map<string, number>>(new Map());
  // 📌 My Picks board — personal shortlist, persisted across restarts.
  const [myPicks, setMyPicks] = useState<TicketLeg[]>(() => {
    try { return JSON.parse(localStorage.getItem("pbz-mypicks") || "[]"); } catch { return []; }
  });
  useEffect(() => {
    try { localStorage.setItem("pbz-mypicks", JSON.stringify(myPicks)); } catch {}
  }, [myPicks]);
  // Cherry-picked legs pulled from across different tickets into one custom slip.
  const [cart, setCart] = useState<TicketLeg[]>([]);
  const [buildTab, setBuildTab] = useState<"regular" | "acca" | "board" | "bankers">("regular");
  // 🍀 Feeling Lucky: one switch — on adds 2 extra parlays of EACH risk band
  // (safe ~75%+, moderate ~40%, risky ~10%+) on top of the main slate.
  const [feelingLucky, setFeelingLucky] = useState(false);
  const [variation, setVariation] = useState(0);
  // What kind of build produced the current result — so "Generate a new set"
  // replays the same thing (Simple stays Simple; a ladder re-ladders).
  const lastBuildRef = useRef<{ kind: "regular" | "simple" | "ladder" }>({ kind: "regular" });
  const [showCost, setShowCost] = useState(false);
  const [costBreak, setCostBreak] = useState<UsageBreakdown | null>(null);

  const [result, setResult] = useState<BuildResult | null>(null);
  const [usage, setUsage] = useState<BuildUsage | null>(null);
  const [model, setModel] = useState("claude-haiku-4-5");
  const [showBoard, setShowBoard] = useState(false);
  const [dataTab, setDataTab] = useState<"picks" | "bankers" | "inspector" | "ingested" | "lineups" | "mypicks">("picks");

  const [bankroll, setBankroll] = useState(0);
  const [buildStrategy, setBuildStrategy] = useState("value");
  const [saved, setSaved] = useState(false);

  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function loadLeagues() {
    // 1 cached request; refreshes the most-picked-first ordering.
    api
      .fetchLeagues()
      .then(setLeagues)
      .catch(() => {
        /* needs an API key; the date screen shows a hint */
      });
  }

  useEffect(() => {
    api
      .getSettings()
      .then((s) => {
        setSettings(s);
        setMeter(s.meter);
        setModel(s.model);
        if (s.has_api_football_key || s.proxy_url) loadLeagues();
      })
      .catch((e) => setError(String(e)));
    api.getBankroll().then((b) => setBankroll(b.current)).catch(() => {});
    api.usageBreakdown().then(setCostBreak).catch(() => {});
    // Auto-settle placed bets whose matches have ended (backend only grades
    // finished fixtures), then refresh the bankroll.
    api
      .settleAll()
      .then(() => api.getBankroll().then((b) => setBankroll(b.current)))
      .catch(() => {});
  }, []);

  const selectedFixtures: Fixture[] = useMemo(
    () => fixtures.filter((f) => selFixtureIds.has(f.fixture_id)),
    [fixtures, selFixtureIds]
  );

  // If a selected match has ENDED (e.g. you left the app open across kickoff),
  // drop it from the selection so the next build can't silently include it.
  useEffect(() => {
    const ended = fixtures.filter((f) => selFixtureIds.has(f.fixture_id) && liveInfo(f).state === "ended");
    if (ended.length === 0) return;
    setSelFixtureIds((prev) => {
      const next = new Set(prev);
      ended.forEach((f) => next.delete(f.fixture_id));
      return next;
    });
    toast.info(`Dropped ${ended.length} ended match${ended.length > 1 ? "es" : ""} from your selection`);
  }, [fixtures]);

  function fail(e: unknown) {
    setError(String(e));
    setBusy(false);
  }

  async function loadFixtures() {
    if (busy) return; // guard against double-fire / request spam
    if (selLeagues.size === 0) {
      toast.info("Pick at least one league first — that keeps the load fast and within budget.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      if (selLeagues.size > 0) {
        await api.bumpLeagues([...selLeagues]).catch(() => {});
      }
      const tz = settings?.timezone || localTimezone();
      const wanted = new Set(dateRange(date, days)); // tz-local dates the user asked for
      // Fetch a ±1-day padded window (the API's date boundary doesn't always
      // honour our tz), then keep only fixtures whose kickoff — in OUR tz —
      // lands on a requested date. Fixes day-boundary leakage/misses.
      const fetchDates = dateRange(addDays(date, -1), days + 2);
      let all: Fixture[] = [];
      const seen = new Set<number>();
      for (const d of fetchDates) {
        const f = await api.fetchFixtures(d, tz);
        for (const fx of f) {
          if (!seen.has(fx.fixture_id)) {
            seen.add(fx.fixture_id);
            all.push(fx);
          }
        }
      }
      all = all.filter((x) => wanted.has(tzDate(x.date_utc, tz)));
      if (selLeagues.size > 0) all = all.filter((x) => selLeagues.has(x.league_id));
      all.sort((a, b) => a.date_utc.localeCompare(b.date_utc));
      setFixtures(all);
      await refreshMeter();
      loadLeagues(); // refresh most-picked ordering for next time
      setStep("matches");
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  async function refreshMeter() {
    try {
      setMeter(await api.getMeter());
    } catch {
      /* ignore */
    }
  }

  function toFixtureInputs(list: Fixture[]): FixtureInput[] {
    return list.map((f) => ({
      fixture_id: f.fixture_id,
      league_id: f.league_id,
      season: f.season,
      home_team_id: f.home_team_id,
      home_team: f.home_team,
      away_team_id: f.away_team_id,
      away_team: f.away_team,
      date_utc: f.date_utc,
      venue_city: f.venue_city,
      referee: f.referee,
    }));
  }

  function toggleGroup(keys: string[]) {
    setSelMarkets((prev) => {
      const allOn = keys.every((k) => prev.has(k));
      const n = new Set(prev);
      keys.forEach((k) => (allOn ? n.delete(k) : n.add(k)));
      return n;
    });
  }

  function ticketSig(t: Ticket): string {
    return `[${t.type}] ${t.legs
      .map((l) => `${l.market}:${l.selection}`)
      .sort()
      .join(" + ")}`;
  }

  // Matches the backend ladder signature (market|subject|line, sorted, "##").
  function ladderSig(t: Ticket): string {
    return t.legs
      .map((l) => `${l.market}|${l.selection}|${l.line ?? ""}`)
      .sort()
      .join("##");
  }

  // fixture_id → competition name, so the Haiku analysis knows it's e.g. the World Cup.
  const leagueByFixtureId = useMemo(() => {
    const m: Record<number, string> = {};
    for (const f of selectedFixtures) m[f.fixture_id] = f.league_name;
    return m;
  }, [selectedFixtures]);

  function onVoidSubject(subject: string, voided: boolean) {
    setVoidedSubjects((prev) => {
      const next = new Map(prev);
      const v = Math.max(0, (next.get(subject) ?? 0) + (voided ? 1 : -1));
      if (v === 0) next.delete(subject);
      else next.set(subject, v);
      return next;
    });
  }

  const cartKeys = useMemo(() => new Set(cart.map(legKey)), [cart]);
  function toggleCartLeg(l: TicketLeg) {
    const k = legKey(l);
    setCart((prev) => (prev.some((x) => legKey(x) === k) ? prev.filter((x) => legKey(x) !== k) : [...prev, l]));
  }
  function removeCartLeg(key: string) {
    setCart((prev) => prev.filter((x) => legKey(x) !== key));
  }

  async function build(opts?: { variation?: number; exclude?: string[]; simple?: boolean }) {
    if (busy || prewarmBusy) return; // guard against double-fire / request spam
    const v = opts?.variation ?? 0;
    const simple = opts?.simple ?? false;
    lastBuildRef.current = { kind: simple ? "simple" : "regular" };
    // Best-in-class presets per Simple risk dial.
    const RISK: Record<string, { strategy: string; safe: number; mod: number; risky: number }> = {
      safe: { strategy: "favorites", safe: 1, mod: 0, risky: 0 },
      balanced: { strategy: "value", safe: 0, mod: 1, risky: 1 },
      bold: { strategy: "value", safe: 0, mod: 1, risky: 3 },
      scout: { strategy: "scout", safe: 0, mod: 1, risky: 1 },
    };
    const rk = RISK[simpleRisk] ?? RISK.balanced;
    setBusy(true);
    setError(null);
    setSaved(false);
    setVariation(v);
    setBuildTab("regular");
    try {
      const selection = {
        fixtures: toFixtureInputs(selectedFixtures),
        // Simple = the whole market for every match, a varied set of tickets,
        // tuned by the risk dial, all the data + ingests — zero setup.
        markets: simple ? [...allMarketKeys] : [...selMarkets],
        reasoning: simple ? true : reasoning,
        implied_prob: impliedProb,
        notes,
        model,
        ticket_count: simple ? 25 : ticketCount,
        ticket_types: simple ? [...TICKET_TYPES] : [...ticketTypes],
        variation: v,
        exclude: opts?.exclude ?? [],
        // Voided subjects persist into rebuilds (they used to only affect ladders).
        exclude_subjects: [...voidedSubjects.entries()].filter(([, n]) => n > 0).map(([s]) => s),
        // Simple advertises "zero setup" — hidden Advanced knobs (leg-prob
        // ceiling, builder bias, data toggles) must NOT leak into it.
        bias_builders: simple ? false : biasBuilders,
        most_likely: simple ? false : strategy === "likely",
        strategy: simple ? rk.strategy : strategy,
        max_leg_prob: simple ? 1 : maxLegProb,
        use_grok: simple ? false : useGrok,
        grok_veto: simple ? false : grokVeto,
        grok_categories: [...grokCats],
        use_weather: simple ? false : useWeather,
        use_standings: simple ? true : useStandings,
        use_h2h: simple ? true : useH2h,
        use_lineups: simple ? true : useLineups,
        use_predictions: simple ? true : usePredictions,
        use_xg: simple ? true : useXg,
        use_tactics: simple ? false : useTactics,
        lucky_safe: simple ? rk.safe : feelingLucky ? 2 : 0,
        lucky_moderate: simple ? rk.mod : feelingLucky ? 2 : 0,
        lucky_risky: simple ? rk.risky : feelingLucky ? 2 : 0,
        use_ingest: simple ? true : useIngest,
        min_legs: simple ? null : regMinLegs > 1 ? regMinLegs : null,
        cover_all: simple ? false : coverAll,
        min_odds: simple ? null : oddsMin > 1.01 ? oddsMin : null,
        max_odds: simple ? null : oddsMax < 999 ? oddsMax : null,
        max_per_subject: simple ? null : regMaxPerSubject > 0 ? regMaxPerSubject : null,
        use_plausibility: usePlausibility,
        min_plausibility: !simple && minPlausibility > 1 ? minPlausibility : null,
        simple,
      };
      const resp = await api.buildTickets(selection);
      setResult(resp.result);
      setBuildStrategy(simple ? rk.strategy : strategy);
      setUsage(resp.usage);
      setMeter(resp.meter);
      setStep("results");
      // Refresh the lifetime cost meter in the header.
      api.getSettings().then(setSettings).catch(() => {});
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  function newSet() {
    // Replay the SAME kind of build the user last ran. Regenerating a Simple
    // slate used to silently rebuild with the (hidden) Advanced settings, and
    // regenerating after a ladder switched it to a model build.
    if (lastBuildRef.current.kind === "ladder") {
      buildLadder(true);
      return;
    }
    const exclude = result ? result.tickets.map(ticketSig) : [];
    build({ variation: variation + 1, exclude, simple: lastBuildRef.current.kind === "simple" });
  }

  // Background one-time plausibility pre-score for the selected slate. Runs one
  // fixture at a time so we can show a 1/x progress bar; results are cached so the
  // later build/ladder read them instantly.
  const fixturesSig = useMemo(
    () => selectedFixtures.map((f) => f.fixture_id).sort().join(","),
    [selectedFixtures]
  );
  async function runPrewarm() {
    if (prewarmBusy) return;
    const fixtures = selectedFixtures;
    const markets = [...selMarkets];
    if (fixtures.length === 0 || markets.length === 0) return;
    setPrewarmBusy(true);
    setPrewarmProgress({ done: 0, total: fixtures.length });
    try {
      for (let i = 0; i < fixtures.length; i++) {
        await api.prewarmPlausibility(toFixtureInputs([fixtures[i]])[0], markets).catch(() => {});
        setPrewarmProgress({ done: i + 1, total: fixtures.length });
      }
      prewarmedRef.current = fixturesSig;
    } finally {
      setPrewarmBusy(false);
      setPrewarmProgress(null);
    }
  }
  useEffect(() => {
    if (
      step === "markets" &&
      usePlausibility &&
      selectedFixtures.length > 0 &&
      selMarkets.size > 0 &&
      prewarmedRef.current !== fixturesSig &&
      !prewarmBusy
    ) {
      runPrewarm();
    }
    // `prewarmBusy` is a dep on purpose: if the fixture selection changes while
    // a prewarm is mid-flight, the busy guard skips the new sig — when the run
    // finishes and busy flips false, this re-fires and prewarms the NEW slate
    // (it used to wedge: "✓ ready" never appeared for the changed selection).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step, usePlausibility, fixturesSig, prewarmBusy]);

  async function buildLadder(append = false) {
    if (busy || prewarmBusy) return; // guard against double-fire / request spam
    lastBuildRef.current = { kind: "ladder" };
    setBusy(true);
    setError(null);
    try {
      const existing = append && result ? result.tickets : [];
      const excludeSigs = existing.map(ladderSig);
      const voided = [...voidedSubjects.entries()].filter(([, n]) => n > 0).map(([s]) => s);
      // Continue the diversity pool across "add more" (seed with the subjects
      // already used) UNLESS the user chose to reset it for the new batch.
      const seed =
        append && !ladderDiversityReset ? existing.flatMap((t) => t.legs.map((l) => l.selection)) : [];
      const variation = append ? ladderVariation + 1 : 0;
      const res = await api.buildLadder(
        toFixtureInputs(selectedFixtures),
        [...selMarkets],
        ladderCount,
        ladderMinProb,
        ladderScope,
        ladderMaxLegs,
        ladderMinHit,
        ladderMaxPerSubject,
        ladderOuSide,
        ladderMinLegs,
        excludeSigs,
        voided,
        seed,
        variation,
        oddsMin > 1.01 ? oddsMin : null,
        oddsMax < 999 ? oddsMax : null,
        ladderOnePerFixture,
        ladderMega,
        coverAll,
        coverLegs
      );
      if (append && result) {
        if (res.tickets.length === 0) {
          setError("No more distinct tickets at these settings — loosen the range, raise the count, or un-void some players.");
        } else {
          setResult({ ...res, tickets: [...result.tickets, ...res.tickets] });
          setLadderVariation(variation);
        }
      } else {
        setResult(res);
        setLadderVariation(0);
        setVoidedSubjects(new Map()); // a fresh ladder clears prior voids
        setUsage(null);
        setBuildStrategy("ladder");
        setSaved(false);
        setStep("results");
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function placeTicket(t: Ticket, stake: number, odds: number | null, strategyOverride?: string) {
    const strat = strategyOverride ?? buildStrategy;
    try {
      await api.placeBet(t, stake, odds, result?.grok_used ?? false, result?.ingest_used ?? false, strat, result?.index_used ?? false);
      const n = t.legs.length;
      toast.success(`Bet placed — $${stake.toFixed(2)} · ${n} leg${n > 1 ? "s" : ""}`);
    } catch (e) {
      toast.error(e);
      return;
    }
    api.getBankroll().then((b) => setBankroll(b.current)).catch(() => {});
  }

  function openCost() {
    setShowCost((v) => !v);
    api.usageBreakdown().then(setCostBreak).catch(() => {});
  }

  function exportCsv() {
    if (!result) return;
    const head = [
      "ticket",
      "type",
      "title",
      "confidence",
      "combined_odds",
      "combined_prob",
      "combined_ev",
      "match",
      "market",
      "selection",
      "line",
      "book_odds",
      "pinnacle_prob",
      "model_prob",
      "leg_ev",
    ];
    const esc = (v: unknown) => {
      const s = v == null ? "" : String(v);
      return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
    };
    const rows = [head.join(",")];
    result.tickets.forEach((t, i) => {
      t.legs.forEach((l) => {
        rows.push(
          [
            i + 1,
            t.type,
            t.title,
            t.confidence,
            t.combined_odds ?? "",
            t.combined_prob ?? "",
            t.combined_ev ?? "",
            l.match,
            l.market,
            l.selection,
            l.line ?? "",
            l.book_odds ?? "",
            l.pinnacle_prob ?? "",
            l.est_prob ?? "",
            l.ev ?? "",
          ]
            .map(esc)
            .join(",")
        );
      });
    });
    const blob = new Blob([rows.join("\n")], { type: "text/csv" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `powabet-tickets-${new Date().toISOString().slice(0, 10)}.csv`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
  }

  async function saveCurrent() {
    if (!result) return;
    try {
      await api.saveTicket(JSON.stringify({ markets: [...selMarkets] }), JSON.stringify(result), notes);
      setSaved(true);
    } catch (e) {
      setError(String(e));
    }
  }

  async function copyCurrent() {
    if (!result) return;
    const text = result.tickets
      .map((t) => {
        const legs = t.legs
          .map((l) => `  • ${l.selection} — ${l.market}${l.line ? " " + l.line : ""}${l.book_odds != null ? ` @${l.book_odds.toFixed(2)}` : ""}`)
          .join("\n");
        const odds = t.combined_odds != null ? ` @${t.combined_odds.toFixed(2)}` : "";
        return `[${t.type}] ${t.title}${odds}\n${legs}`;
      })
      .join("\n\n");
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      /* ignore */
    }
  }

  // ----- overlays -----
  if (overlay === "settings" && settings) {
    return (
      <Shell meter={meter} cost={settings?.usage.cost_usd ?? 0} grokCost={costBreak?.grok_lifetime ?? 0} onCoins={openCost} onNav={setOverlay} ingestBadge={ingestPending} current="settings">
        <Settings
          settings={settings}
          onSaved={(s) => {
            setSettings(s);
            setMeter(s.meter);
            setModel(s.model);
            if (s.has_api_football_key || s.proxy_url) loadLeagues();
          }}
          onClose={() => setOverlay(null)}
        />
      </Shell>
    );
  }
  if (overlay === "history") {
    return (
      <Shell meter={meter} cost={settings?.usage.cost_usd ?? 0} grokCost={costBreak?.grok_lifetime ?? 0} onCoins={openCost} onNav={setOverlay} ingestBadge={ingestPending} current="history">
        <History onClose={() => setOverlay(null)} />
      </Shell>
    );
  }
  if (overlay === "tracker") {
    return (
      <Shell meter={meter} cost={settings?.usage.cost_usd ?? 0} grokCost={costBreak?.grok_lifetime ?? 0} onCoins={openCost} onNav={setOverlay} ingestBadge={ingestPending} current="tracker">
        <Tracker onClose={() => setOverlay(null)} />
      </Shell>
    );
  }
  if (overlay === "newsfeed") {
    return (
      <Shell meter={meter} cost={settings?.usage.cost_usd ?? 0} grokCost={costBreak?.grok_lifetime ?? 0} onCoins={openCost} onNav={setOverlay} ingestBadge={ingestPending} current="newsfeed">
        <Newsfeed onClose={() => setOverlay(null)} />
      </Shell>
    );
  }
  if (overlay === "ledger") {
    return (
      <Shell meter={meter} cost={settings?.usage.cost_usd ?? 0} grokCost={costBreak?.grok_lifetime ?? 0} onCoins={openCost} onNav={setOverlay} ingestBadge={ingestPending} current="ledger">
        <Ledger onClose={() => setOverlay(null)} />
      </Shell>
    );
  }

  if (overlay === "ingest") {
    return (
      <Shell meter={meter} cost={settings?.usage.cost_usd ?? 0} grokCost={costBreak?.grok_lifetime ?? 0} onCoins={openCost} onNav={setOverlay} ingestBadge={ingestPending} current="ingest">
        <Ingest onClose={() => setOverlay(null)} fixtures={toFixtureInputs(fixtures)} />
      </Shell>
    );
  }

  if (overlay === "live") {
    return (
      <Shell meter={meter} cost={settings?.usage.cost_usd ?? 0} grokCost={costBreak?.grok_lifetime ?? 0} onCoins={openCost} onNav={setOverlay} ingestBadge={ingestPending} current="live">
        <Live
          onClose={() => setOverlay(null)}
          defaultStake={settings?.default_stake ?? 0.5}
          buildModel={settings?.model ?? "claude-opus-4-8"}
          onPlaced={() => api.getBankroll().then((b) => setBankroll(b.current)).catch(() => {})}
        />
      </Shell>
    );
  }

  return (
    <Shell
      meter={meter}
      cost={settings?.usage.cost_usd ?? 0}
      grokCost={costBreak?.grok_lifetime ?? 0}
      onCoins={openCost}
      onNav={setOverlay}
      ingestBadge={ingestPending}
      current={null}
      onInspect={() => { setDataTab("picks"); setShowBoard(true); }}
      canInspect={selectedFixtures.length > 0}
    >
      {showCost && (
        <div className="fixed top-16 right-3 z-50 card w-60 shadow-2xl text-xs space-y-1">
          <div className="flex items-center justify-between">
            <span className="font-semibold text-slate-300">Spend (calendar)</span>
            <button className="text-slate-500" onClick={() => setShowCost(false)}>✕</button>
          </div>
          {costBreak ? (
            <>
              <div className="text-[10px] text-slate-500 mt-1">🪙 Claude (Anthropic)</div>
              <CostRow label="Today" claude={costBreak.today} />
              <CostRow label="This week" claude={costBreak.week} />
              <CostRow label="This month" claude={costBreak.month} />
              <CostRow label="Lifetime" claude={costBreak.lifetime} />
              <div className="border-t border-edge my-1" />
              <div className="text-[10px] text-slate-500">🔍 Grok (x.ai) — actual billed</div>
              <CostRow label="Today" claude={costBreak.grok_today} />
              <CostRow label="This week" claude={costBreak.grok_week} />
              <CostRow label="This month" claude={costBreak.grok_month} />
              <CostRow label="Lifetime" claude={costBreak.grok_lifetime} />
            </>
          ) : (
            <div className="text-slate-500">loading…</div>
          )}
        </div>
      )}
      {showBoard && selectedFixtures.length > 0 && (
        <div className="fixed inset-0 z-40 bg-ink overflow-y-auto">
          <div className="max-w-md mx-auto p-4">
            {/* Data viewer — ONE header, four tabs, one close. Tabs never close
                the panel; only ✕ does, so back-and-forth browsing is free. */}
            <div className="sticky top-0 z-10 bg-ink pb-2 -mx-4 px-4 pt-1 border-b border-edge mb-3">
              <div className="flex items-center justify-between mb-2">
                <h2 className="text-lg font-bold">📊 Data</h2>
                <button className="btn btn-ghost text-sm py-2" onClick={() => setShowBoard(false)}>
                  ✕ Done
                </button>
              </div>
              <div className="flex gap-1.5">
                {([
                  ["picks", "📊 Picks"],
                  ["mypicks", "📌 Mine"],
                  ["bankers", "🏦 Bankers"],
                  ["lineups", "👥 XIs"],
                  ["inspector", "🔍 Inspector"],
                  ["ingested", "🧲 Ingested"],
                ] as const).map(([id, label]) => (
                  <button
                    key={id}
                    className={`flex-1 text-center text-xs rounded-lg py-2 ${
                      dataTab === id ? "bg-accent text-ink font-semibold" : "bg-panel border border-edge text-slate-300"
                    }`}
                    onClick={() => setDataTab(id)}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </div>
            {(dataTab === "picks" || dataTab === "bankers") && (
              <PicksBoard
                key={dataTab}
                fixtures={toFixtureInputs(selectedFixtures)}
                markets={[...selMarkets]}
                bankroll={bankroll}
                kellyFraction={settings?.kelly_fraction ?? 0}
                defaultStake={settings?.default_stake ?? 0.5}
                mode={dataTab === "bankers" ? "bankers" : "all"}
                onPlaced={() => api.getBankroll().then((b) => setBankroll(b.current)).catch(() => {})}
              />
            )}
            {dataTab === "mypicks" && (
              <MyPicks
                picks={myPicks}
                selectedFixtureIds={selectedFixtures.map((f) => f.fixture_id)}
                onRemove={(k) => setMyPicks((prev) => prev.filter((l) => legKey(l) !== k))}
                onClear={() => setMyPicks([])}
                onPlace={(t, stake, odds) => placeTicket(t, stake, odds, "custom")}
              />
            )}
            {dataTab === "lineups" && <Lineups fixtures={toFixtureInputs(selectedFixtures)} />}
            {dataTab === "inspector" && <Inspector fixtures={toFixtureInputs(selectedFixtures)} />}
            {dataTab === "ingested" && <IngestedStats fixtures={toFixtureInputs(selectedFixtures)} />}
          </div>
        </div>
      )}
      {error && (
        <div className="card border-bad/60 text-sm text-bad mb-3">
          {error}
          <button className="ml-2 underline" onClick={() => setError(null)}>
            dismiss
          </button>
        </div>
      )}

      <StepDots step={step} onJump={setStep} hasResult={result != null} />

      {/* DATE */}
      {step === "date" && (
        <div className="space-y-4">
          <div>
            <h2 className="text-lg font-bold">Pick a date &amp; leagues</h2>
            <p className="text-xs text-slate-400">Step 1 of 4 — choose when, and which competitions to pull matches from.</p>
          </div>
          <input
            type="date"
            className="w-full rounded-xl bg-panel border border-edge px-4 py-3 text-lg"
            value={date}
            onChange={(e) => setDate(e.target.value)}
          />

          <div>
            <div className="text-xs font-semibold text-slate-400 mb-2">Days to load</div>
            <div className="flex gap-1.5">
              {[1, 2, 3, 4, 5, 6, 7].map((d) => (
                <button
                  key={d}
                  className={`chip flex-1 text-center ${days === d ? "chip-on" : ""}`}
                  onClick={() => setDays(d)}
                >
                  {d}
                </button>
              ))}
            </div>
            <p className="text-[11px] text-slate-500 mt-1">
              {days === 1 ? "Just this date (≤3 requests, tz-padded)." : `${days} days from this date (~${days + 2} requests, cached after).`}
            </p>
          </div>

          <div>
            <div className="flex items-center justify-between mb-2">
              <span className="text-xs font-semibold text-slate-400">
                Leagues {selLeagues.size === 0 ? "(pick at least one)" : `(${selLeagues.size})`}
              </span>
              {selLeagues.size > 0 && (
                <button
                  className="text-xs text-slate-400 underline"
                  onClick={() => setSelLeagues(new Set())}
                >
                  clear
                </button>
              )}
            </div>
            <input
              className="w-full rounded-lg bg-ink border border-edge px-3 py-2 text-sm mb-2"
              placeholder="Search leagues…"
              value={leagueSearch}
              onChange={(e) => setLeagueSearch(e.target.value)}
            />
            <LeaguePicker
              leagues={leagues}
              search={leagueSearch}
              selected={selLeagues}
              onToggle={(id) => toggle(setSelLeagues, id)}
            />
            <p className="text-[11px] text-slate-500 mt-2">
              Pick the leagues you care about — we load only their matches (loading every league at
              once is slow and burns your request budget). Your most-picked leagues rise to the top.
            </p>
          </div>

          <button className="btn btn-primary w-full" onClick={loadFixtures} disabled={busy || selLeagues.size === 0}>
            {busy ? (
              <span className="inline-flex items-center gap-2"><Spinner /> Loading fixtures…</span>
            ) : selLeagues.size === 0 ? (
              "Pick a league to load matches"
            ) : (
              `Load matches · ${selLeagues.size} league${selLeagues.size > 1 ? "s" : ""}`
            )}
          </button>
          {settings && !settings.has_api_football_key && !settings.proxy_url && (
            <div className="text-xs text-warn">
              Add your API-Football key (or a proxy URL) in Settings first.
            </div>
          )}
        </div>
      )}

      {/* MATCHES */}
      {step === "matches" && (
        <div className="space-y-3 pb-28">
          <Header title={`Matches · ${date}`} onBack={() => setStep("date")} />
          <p className="text-xs text-slate-400 -mt-1">Step 2 of 4 — tap the matches you want to research (up to ~10).</p>
          <div
            className={`text-[11px] rounded-lg px-2.5 py-1.5 ${
              selFixtureIds.size > 10
                ? "bg-warn/15 text-warn"
                : "bg-ink text-slate-500 border border-edge"
            }`}
          >
            {selFixtureIds.size > 10
              ? `${selFixtureIds.size} matches selected — the table and context get thin past ~10. For big cross-game accas, the Ladder's one-leg-per-match mode handles any count for free.`
              : "Pick up to ~10 matches — the pool and context scale with your selection (past 6, low-value context is auto-trimmed)."}
          </div>
          {fixtures.length === 0 && (
            <div className="card text-sm text-slate-400 space-y-2">
              <div>No matches for these leagues on {date}.</div>
              <button className="btn btn-ghost text-xs py-1.5" onClick={() => setStep("date")}>
                ← Try another date or add leagues
              </button>
            </div>
          )}
          <div className="space-y-2">
            {fixtures.map((f) => {
              const on = selFixtureIds.has(f.fixture_id);
              const info = liveInfo(f);
              const ended = info.state === "ended";
              return (
                <button
                  key={f.fixture_id}
                  disabled={ended && !on}
                  className={`card w-full text-left ${on ? "border-accent bg-accent/10" : ""} ${
                    ended && !on ? "opacity-50 cursor-not-allowed" : ""
                  }`}
                  onClick={() => (!ended || on) && toggle(setSelFixtureIds, f.fixture_id)}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="font-semibold">
                      {f.home_team} <span className="text-slate-500">vs</span> {f.away_team}
                    </span>
                    <div className="flex items-center gap-2 shrink-0">
                      {info.state === "live" && (
                        <span className="text-[10px] font-bold text-bad bg-bad/15 rounded px-1.5 py-0.5 animate-pulse">
                          ● {info.label}
                        </span>
                      )}
                      {ended && (
                        <span className="text-[10px] font-semibold text-slate-400 bg-edge rounded px-1.5 py-0.5">
                          {on ? "Ended — tap to remove ✕" : "Ended"}
                        </span>
                      )}
                      {on && !ended && <span className="text-accent text-sm">✓</span>}
                    </div>
                  </div>
                  <div className="text-xs text-slate-400">
                    {f.league_name} ·{" "}
                    {ended ? (
                      <span className="text-slate-500">Match ended — not available to bet</span>
                    ) : info.state === "live" ? (
                      <span className="text-bad">in play</span>
                    ) : (
                      new Date(f.date_utc).toLocaleString([], {
                        month: "short",
                        day: "numeric",
                        hour: "2-digit",
                        minute: "2-digit",
                        timeZone: settings?.timezone || undefined,
                      })
                    )}
                  </div>
                  {(f.venue_name || f.venue_city || f.referee) && (
                    <div className="text-[11px] text-slate-500 mt-0.5 truncate">
                      {[f.venue_name, f.venue_city].filter(Boolean).join(", ")}
                      {f.referee ? ` · ref ${f.referee}` : ""}
                    </div>
                  )}
                </button>
              );
            })}
          </div>
          <Sticky>
            <button
              className="btn btn-primary w-full"
              disabled={selFixtureIds.size === 0 || busy}
              onClick={() => setStep("markets")}
            >
              {`Next · ${selFixtureIds.size} selected`}
            </button>
          </Sticky>
        </div>
      )}

      {/* MARKETS + OPTIONS + BUILD */}
      {step === "markets" && (
        <div className="space-y-4 pb-44">
          <Header title={mode === "simple" ? "Build" : "Markets"} onBack={() => setStep("matches")} />

          {/* Simple ⇄ Advanced toggle */}
          <div className="flex items-center justify-between -mt-1">
            <div className="flex rounded-lg overflow-hidden border border-edge text-xs">
              {(["simple", "advanced"] as const).map((m) => (
                <button
                  key={m}
                  className={`px-3 py-1.5 ${mode === m ? "bg-accent text-ink font-semibold" : "text-slate-300"}`}
                  onClick={() => chooseMode(m)}
                >
                  {m === "simple" ? "🟢 Simple" : "⚙️ Advanced"}
                </button>
              ))}
            </div>
            <label className="text-[10px] text-slate-500 flex items-center gap-1 cursor-pointer">
              <input type="checkbox" checked={rememberMode} onChange={(e) => toggleRemember(e.target.checked)} />
              remember until next launch
            </label>
          </div>

          <button
            className="btn btn-ghost w-full text-xs py-2 disabled:opacity-40"
            disabled={refreshing || busy || selFixtureIds.size === 0}
            title={`Force-fresh odds, lineups and injuries for the ${selFixtureIds.size} selected match(es) — cached copies go stale near kickoff (lines move, lineups drop ~1h before). Costs ~${selFixtureIds.size * 3} requests.`}
            onClick={async () => {
              setRefreshing(true);
              try {
                const n = await api.refreshFixtureData(toFixtureInputs(selectedFixtures));
                setMeter(await api.getMeter());
                toast.success(`Refreshed ${n} data pull(s) — odds, lineups & injuries are now live-fresh.`);
              } catch (e) {
                toast.error(e);
              } finally {
                setRefreshing(false);
              }
            }}
          >
            {refreshing ? "↻ Refreshing…" : `↻ Refresh data (odds · lineups · injuries) — ${selFixtureIds.size} match${selFixtureIds.size === 1 ? "" : "es"}`}
          </button>

          {mode === "simple" && (
            <div className="card space-y-2 border-accent/40">
              <div className="text-sm font-bold">🟢 Simple build</div>
              <p className="text-xs text-slate-400">
                We'll read <b>every market</b> for your {selFixtureIds.size} match{selFixtureIds.size === 1 ? "" : "es"}, forecast each game, and build a
                varied set using all the data. Pick a risk level — that's the only choice. The per-match forecast shows under the tickets.
              </p>
              {/* Risk dial — best-in-class presets, one tap. */}
              <div className="grid grid-cols-4 gap-1">
                {([
                  ["safe", "🛡️ Safe", "Form favourites, no longshots"],
                  ["balanced", "⚖️ Balanced", "Value picks + a couple of longshots"],
                  ["bold", "🔥 Bold", "More jackpot-style longshots"],
                  ["scout", "📡 Scout", "Fuse your ingested pages with our data"],
                ] as [typeof simpleRisk, string, string][]).map(([id, label, hint]) => (
                  <button
                    key={id}
                    title={hint}
                    className={`chip text-center text-[11px] py-2 ${simpleRisk === id ? "chip-on" : ""}`}
                    onClick={() => setSimpleRisk(id)}
                  >
                    {label}
                  </button>
                ))}
              </div>
              <p className="text-[10px] text-slate-500">
                {simpleRisk === "safe"
                  ? "🛡️ Safe — in-form favourites at fair odds, no lottery legs. Bankable."
                  : simpleRisk === "balanced"
                    ? "⚖️ Balanced — +EV value picks across markets plus a couple of longshots. The all-rounder."
                    : simpleRisk === "bold"
                      ? "🔥 Bold — same value core but more jackpot-style longshots stacked for big payouts."
                      : "📡 Scout — builds from the stats you've ingested fused with our data (needs a processed page matching your fixtures)."}
              </p>
              <button className="btn btn-primary w-full text-sm py-2.5" disabled={busy || prewarmBusy} onClick={() => build({ simple: true })}>
                {busy ? <span className="inline-flex items-center gap-2"><Spinner /> Building…</span> : prewarmBusy ? "🔒 Pre-scoring…" : "✨ Build my tickets"}
              </button>
              <p className="text-[10px] text-slate-500">Want the knobs (markets, strategies, ladders)? Switch to ⚙️ Advanced above.</p>
            </div>
          )}

          {mode === "advanced" && (<>
          <p className="text-xs text-slate-400 -mt-1">Step 3 of 4 — pick what to bet on (or tap a preset), then choose how to build below.</p>

          <div>
            <div className="flex items-center justify-between mb-2">
              <div className="text-xs font-semibold text-slate-400">Presets</div>
              <div className="flex gap-2">
                <button className="text-[11px] text-accent underline" onClick={() => setSelMarkets(new Set(allMarketKeys))}>
                  select all
                </button>
                <button className="text-[11px] text-slate-400 underline" onClick={() => setSelMarkets(new Set())}>
                  deselect all
                </button>
              </div>
            </div>
            <div className="flex flex-wrap gap-2">
              {marketPresets.map((p) => (
                <span key={p.name} className="inline-flex items-center">
                  <button
                    className={`chip rounded-r-none ${sameMarkets(p.markets) ? "chip-on" : ""}`}
                    title={p.markets.join(", ")}
                    onClick={() => setSelMarkets(new Set(p.markets))}
                  >
                    {p.name}
                  </button>
                  <button
                    className="chip rounded-l-none border-l-0 px-1.5 text-slate-500"
                    title={`Delete preset “${p.name}”`}
                    onClick={() => deletePreset(p.name)}
                  >
                    ×
                  </button>
                </span>
              ))}
              {presetNaming ? (
                <span className="inline-flex items-center gap-1">
                  <input
                    className="input text-xs w-36 py-1.5 rounded-full px-3"
                    placeholder="Preset name…"
                    autoFocus
                    value={presetName}
                    onChange={(e) => setPresetName(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") savePreset();
                      if (e.key === "Escape") {
                        setPresetNaming(false);
                        setPresetName("");
                      }
                    }}
                  />
                  <button
                    className="chip text-accent"
                    disabled={!presetName.trim() || selMarkets.size === 0}
                    onClick={savePreset}
                  >
                    Save
                  </button>
                  <button
                    className="chip text-slate-500"
                    onClick={() => {
                      setPresetNaming(false);
                      setPresetName("");
                    }}
                  >
                    ✕
                  </button>
                </span>
              ) : (
                <button
                  className="chip border-dashed text-accent"
                  title="Save the currently selected markets as a named preset"
                  onClick={() => setPresetNaming(true)}
                  disabled={selMarkets.size === 0}
                >
                  ＋ Save current…
                </button>
              )}
            </div>
          </div>

          <div className="space-y-3">
            {(["Attacking", "Involvement"] as const).map((sub) => (
              <div key={sub}>
                <button
                  className="text-xs font-semibold text-slate-400 mb-2 underline-offset-2 hover:underline"
                  onClick={() =>
                    toggleGroup(MARKETS.filter((m) => m.group === "player" && m.sub === sub).map((m) => m.key))
                  }
                >
                  {sub === "Attacking" ? "Goal-scoring props" : "Passes / fouls / tackles / cards"}{" "}
                  <span className="text-slate-500">— tap to toggle all</span>
                </button>
                <div className="flex flex-wrap gap-2">
                  {MARKETS.filter((m) => m.group === "player" && m.sub === sub).map((m) => (
                    <MarketChip key={m.key} k={m.key} label={m.label} sel={selMarkets} setSel={setSelMarkets} />
                  ))}
                </div>
              </div>
            ))}
          </div>

          <div className="space-y-3">
            {(["Result", "Goals"] as const).map((sub) => (
              <div key={sub}>
                <button
                  className="text-xs font-semibold text-slate-400 mb-2 underline-offset-2 hover:underline"
                  onClick={() => toggleGroup(MARKETS.filter((m) => m.group === "team" && m.sub === sub).map((m) => m.key))}
                >
                  {sub === "Result" ? "Match result (full-time & by half)" : "Goals"}{" "}
                  <span className="text-slate-500">— tap to toggle all</span>
                </button>
                <div className="flex flex-wrap gap-2">
                  {MARKETS.filter((m) => m.group === "team" && m.sub === sub).map((m) => (
                    <MarketChip key={m.key} k={m.key} label={m.label} sel={selMarkets} setSel={setSelMarkets} />
                  ))}
                </div>
              </div>
            ))}
          </div>

          <p className="text-[11px] text-slate-500">
            Players are auto-selected from each team — no need to pick them.
          </p>

          {/* mode tabs */}
          <div className="flex gap-1.5">
            {(
              [
                ["regular", "🎯 Regular"],
                ["acca", "🪜 Acca ladder"],
              ] as const
            ).map(([id, label]) => (
              <button
                key={id}
                className={`flex-1 text-center text-sm rounded-lg py-2 ${
                  buildTab === id ? "bg-accent text-ink font-semibold" : "bg-ink border border-edge text-slate-300"
                }`}
                onClick={() => setBuildTab(id)}
              >
                {label}
              </button>
            ))}
          </div>

          {/* Plausibility FILTER — a fast AI rates every pick 1–5 on how realistic it
              is (likely to start, fits the game). Slide up to hide traps/fringe lines. */}
          <div className="card space-y-1.5">
            <div className="flex items-center justify-between">
              <div className="text-xs font-semibold text-slate-200">🧠 Plausibility filter</div>
              <span className="text-[11px] text-accent font-mono">
                {minPlausibility === 1 ? "off" : `${minPlausibility}+/5`}
                {prewarmBusy ? " · scoring…" : prewarmedRef.current === fixturesSig ? " · ✓ ready" : ""}
              </span>
            </div>
            <input
              type="range"
              min={1}
              max={5}
              step={1}
              value={minPlausibility}
              onChange={(e) => setMinPlausibility(parseInt(e.target.value, 10))}
              className="w-full accent-accent"
            />
            <p className="text-[10px] text-slate-500">
              {[
                "", // index 0 unused
                "1 · Off — every candidate, including fringe & unlikely lines. Widest, noisiest.",
                "2 · Drops the clearest traps (benched/injured, wrong role). Light clean-up.",
                "3 · Solid picks only — neutral-or-better realism. Good default for quality.",
                "4 · Strong picks — fits the player's role and the matchup well. Tighter, fewer options.",
                "5 · Only the most bankable, textbook-realistic lines. Very strict — can thin the slate a lot.",
              ][minPlausibility]}
            </p>
          </div>

          {buildTab === "regular" && (
          <>
          <div className="card space-y-3">
            <div>
              <div className="text-xs font-semibold text-slate-400 mb-2">Strategy</div>
              <div className="flex flex-wrap gap-1.5">
                {(([
                  // Primary strategies always shown; the redundant/niche ones live
                  // behind the ⚙ Advanced expander.
                  ["apex", "Apex 🎯"],
                  ["stacker", "Stacker 🧗"],
                  ["scout", "Scout 📡"],
                  ["value", "Value +EV"],
                  ["bankers", "Anchors ⚓"],
                  ["jackpot", "Jackpot 🎰"],
                  ["predictor", "Match Predictor 🔮"],
                  ...(showAdvanced
                    ? [
                        ["favorites", "Form faves"],
                        ["likely", "Secret picks"],
                        ["oracle", "Oracle ✦"],
                        ["power", "Power Stacker ⚡"],
                      ]
                    : []),
                ] as [string, string][])).map(([id, label]) => {
                  // Match Predictor needs exactly one fixture — show it but disable
                  // it (with a hint) otherwise, so it's discoverable.
                  const dis = id === "predictor" && selFixtureIds.size !== 1;
                  return (
                    <button
                      key={id}
                      disabled={dis}
                      title={dis ? "Select exactly ONE match to use Match Predictor" : undefined}
                      className={`chip flex-1 text-center whitespace-nowrap ${strategy === id ? "chip-on" : ""} ${
                        id === "apex" || id === "oracle" || id === "power" || id === "bankers" || id === "jackpot" || id === "predictor" || id === "scout" ? "border-accent/60" : ""
                      } ${dis ? "opacity-40 cursor-not-allowed" : ""}`}
                      onClick={() => !dis && setStrategy(id)}
                    >
                      {label}
                    </button>
                  );
                })}
                <button
                  className="chip text-center text-slate-400"
                  onClick={() => setShowAdvanced((v) => !v)}
                  title="Show/hide the extra strategies and data-source knobs"
                >
                  {showAdvanced ? "▴ Less" : "⚙ Advanced"}
                </button>
              </div>
              {selFixtureIds.size !== 1 && (
                <p className="text-[10px] text-slate-500 mt-1">🔮 Match Predictor needs exactly one match selected (you have {selFixtureIds.size}).</p>
              )}
              <p className="text-[11px] text-slate-500 mt-1">
                {strategy === "apex"
                  ? "🎯 Apex — the discipline strategy: bets ONLY where a proven edge mechanism exists. (1) Sharp-edge singles: the takeable price beats Pinnacle's de-vigged truth by 2%+ AND our model agrees. (2) Correlated SGPs: combos our Monte-Carlo copula prices as reinforcing each other more than the book charges for (lift ×1.08+, corr-EV 2%+) — found deterministically, the model just copies them. No padding, no longshots; a short slate means Apex is passing, which IS the strategy. Judge it by CLV in the Tracker."
                  : strategy === "favorites"
                  ? "In-form favourites at useful odds (~1.5–2.5): known scorers, strong wins — bankable parlays, not chalk or longshots."
                  : strategy === "likely"
                    ? "Wide scan for motivated underdogs & value picks with a reason (must-win, momentum) — leans on Grok/standings. Pairs best with Grok on."
                    : strategy === "oracle"
                      ? "✦ Claude's own read. Bets only where the sharp price, my model and a real edge all AGREE — at under-the-radar odds (~1.7–3.2). Deliberately fades chalk, lottery longshots, and any leg where my model fights the market. Each 'why' names the confluence."
                      : strategy === "power"
                        ? "⚡ Power Stacker — cross-game DOUBLES (rarely a treble) only. Stacks two high-likelihood 'must-happen' picks the book prices generously (~2.0) into 4x–10x for fewer things to connect. Low variance, lottery-like payout. Auto-builds 2-leg parlays regardless of the type toggles."
                        : strategy === "bankers"
                          ? "⚓ Anchors — auto-builds a ticket from the 'this basically always happens' picks: regular bookers, reliable shooters, high-volume passers, corner-heavy teams. Leans on the measured recent hit-rate (carded N of last M). (For a browsable, cherry-pick version of the same idea, use the 🏦 Bankers board tab.)"
                          : strategy === "jackpot"
                            ? "🎰 Jackpot — deliberate lottery tickets: 5-8 plausible longshots stacked for ~1-5% hit chance at 20x-150x+. Every leg is reasonable on its own; it's a longshot only because they must ALL land. Prefers correlated same-game legs so the true chance beats the multiply. Stake tiny (your 50¢), build a couple daily, wait for one to hit."
                            : strategy === "predictor"
                              ? "🔮 Match Predictor — a deep read of THIS one game. Forces every market, shows a forecast (likely result, scores, goals, cards, key players with %), then builds several same-game SGP variations. If the match is already in-play, it pulls the LIVE score/stats and the model adjusts every suggestion for the time remaining. (Single fixture only.)"
                              : strategy === "scout"
                                ? "📡 Scout — FUSES your ingested pages with our data through the model. It builds our table for the markets you pick above (or all of them if you pick none), pulls in the WHOLE ingested page (corners, cards, shots, form, xG, injuries, predictions, analyst reads — whatever you scraped), and the model cross-references the two: leaning in where they agree, judging where they differ, using the page's extra angles. Needs at least one processed page matching your fixtures (a stats/preview page → 🧲 Ingest → process). Your hand-fed edge, run through the model."
                                : strategy === "stacker"
                                  ? "🧗 Stacker — risk-controlled, STAT-LED stacks (not EV-hunting). Canvasses every match and stacks 5-8 measured picks into 6x-25x parlays. The trick: when the obvious line is chalk (corners over 5.5 @ 1.10), it STEPS UP to the still-plausible next line (over 6.5/7.5), and a near-certain scorer becomes Multi Scorer (2+). Safe stepped legs + 1-2 measured risks per ticket — decent multiplier, genuine fighting chance, no lottery junk and no useless 1.10 legs."
                                  : "Ranks by +EV — value/longshots where the price beats the true probability."}
              </p>
              <button
                className="btn btn-ghost w-full text-xs py-2 border border-edge mt-2"
                disabled={darwinBusy || selFixtureIds.size === 0}
                title="Paper-trades 8 deterministic micro-strategies (sharp-EV ×2, form-gap, line-room, correlated combos, cross-game shooters, chalk treble, contrarian unders) on this slate into the Ledger. Zero model tokens; the Ledger becomes a survival-of-the-fittest leaderboard."
                onClick={async () => {
                  if (darwinRef.current) return;
                  darwinRef.current = true;
                  setDarwinBusy(true);
                  try {
                    setDarwinReport(await api.darwinSweep(toFixtureInputs(selectedFixtures), [...selMarkets]));
                  } catch (e) {
                    toast.error(e);
                  } finally {
                    darwinRef.current = false;
                    setDarwinBusy(false);
                  }
                }}
              >
                {darwinBusy ? "🧬 Sweeping…" : "🧬 Darwin sweep — paper-trade 8 micro-strategies on this slate (0 tokens)"}
              </button>
              {darwinReport && (
                <div className="mt-2 rounded-lg bg-ink border border-edge px-2.5 py-2 space-y-1">
                  <div className="flex items-center justify-between">
                    <span className="text-[11px] font-semibold text-slate-300">🧬 Darwin sweep — what just happened</span>
                    <button className="text-slate-500 hover:text-slate-200 text-xs" onClick={() => setDarwinReport(null)}>✕</button>
                  </div>
                  <p className="text-[10px] text-slate-500">
                    Nothing was bet. Darwin PAPER-traded these deterministic micro-strategies on your slate
                    (0 model tokens) and recorded their tickets to the Ledger. When the matches finish they
                    settle automatically — over weeks the Ledger's 🧬 rows become a survival-of-the-fittest
                    leaderboard showing which mechanical approach actually wins on YOUR slates.
                  </p>
                  <div className="space-y-0.5">
                    {darwinReport.map((l, i) => (
                      <div key={i} className="text-[11px] text-slate-300">{l}</div>
                    ))}
                  </div>
                  <p className="text-[10px] text-slate-500">→ Open the 🧬 Ledger tab to watch them settle.</p>
                </div>
              )}
            </div>
            <div>
              <div className="text-[11px] text-slate-500 mb-1">
                Safety ceiling —{" "}
                {maxLegProb >= 0.999
                  ? "default (near-certainties >93% always dropped)"
                  : `drop legs over ${Math.round(maxLegProb * 100)}% likely`}
              </div>
              <input
                type="range"
                min={40}
                max={100}
                step={5}
                value={Math.round(maxLegProb * 100)}
                onChange={(e) => setMaxLegProb(parseInt(e.target.value, 10) / 100)}
                className="w-full"
              />
              <p className="text-[10px] text-slate-500">
                Lower = less safe — strips out the obvious chalk so picks lean under-the-radar
                (good with Secret picks).
              </p>
            </div>
            <div>
              <div className="text-xs font-semibold text-slate-400 mb-2">Tickets per run</div>
              <div className="flex gap-1.5">
                {[5, 8, 10, 12, 15, 20].map((n) => (
                  <button
                    key={n}
                    className={`chip flex-1 text-center ${ticketCount === n ? "chip-on" : ""}`}
                    onClick={() => setTicketCount(n)}
                  >
                    {n}
                  </button>
                ))}
              </div>
            </div>
            <OddsBand min={oddsMin} max={oddsMax} setMin={setOddsMin} setMax={setOddsMax} />
            <div>
              <div className="text-[11px] text-slate-500 mb-1">
                Min legs per ticket — {regMinLegs <= 1 ? "off (any)" : `${regMinLegs}-fold minimum`}
              </div>
              <div className="flex gap-1.5">
                {[1, 2, 3, 4, 5, 6].map((n) => (
                  <button
                    key={n}
                    className={`chip flex-1 text-center ${regMinLegs === n ? "chip-on" : ""}`}
                    onClick={() => setRegMinLegs(n)}
                  >
                    {n === 1 ? "off" : n}
                  </button>
                ))}
              </div>
            </div>
            <div>
              <div className="text-[11px] text-slate-500 mb-1">
                Diversity — {regMaxPerSubject === 0 ? "auto (≤¼ of tickets)" : `max ${regMaxPerSubject} ticket(s) per player/team`}
              </div>
              <div className="flex gap-1.5">
                {[0, 1, 2, 3, 4].map((n) => (
                  <button
                    key={n}
                    className={`chip flex-1 text-center ${regMaxPerSubject === n ? "chip-on" : ""}`}
                    onClick={() => setRegMaxPerSubject(n)}
                  >
                    {n === 0 ? "auto" : n}
                  </button>
                ))}
              </div>
            </div>
            <div>
              <div className="text-xs font-semibold text-slate-400 mb-2">Ticket types</div>
              <div className="flex gap-2">
                {TICKET_TYPES.map((tt) => {
                  const on = ticketTypes.has(tt);
                  return (
                    <button
                      key={tt}
                      className={`chip flex-1 text-center ${on ? "chip-on" : ""}`}
                      onClick={() =>
                        setTicketTypes((prev) => {
                          const next = new Set(prev);
                          if (next.has(tt)) {
                            if (next.size > 1) next.delete(tt);
                          } else next.add(tt);
                          return next;
                        })
                      }
                    >
                      {tt}
                    </button>
                  );
                })}
              </div>
            </div>
            <div className="space-y-1">
              <Toggle label="🧩 Cover every match" on={coverAll} onChange={setCoverAll} />
              {coverAll && (
                <p className="text-[10px] text-slate-500">
                  Every ticket takes at least one usable leg from EVERY selected match.
                </p>
              )}
            </div>
            <div className="space-y-1">
              <Toggle label="🧲 Use ingested data" on={useIngest} onChange={setUseIngest} />
              {useIngest && (
                <p className="text-[10px] text-slate-500">
                  Feeds your processed pages into the build. Tracked on every ticket (🧲) so the
                  Ledger can show whether your ingested data actually helps.
                </p>
              )}
            </div>
            <div className="space-y-1">
              <Toggle label="🍀 Feeling Lucky" on={feelingLucky} onChange={setFeelingLucky} />
              {feelingLucky && (
                <p className="text-[10px] text-slate-500">
                  Adds 6 extra parlays on top of the main slate: 2 safe (~75%+), 2 moderate (~40%), 2 risky (~10%+ longshots).
                </p>
              )}
            </div>
          </div>

          <div className="card space-y-3">
            <Toggle label="Reasoning (why per ticket)" on={reasoning} onChange={setReasoning} />
            <Toggle label="Implied-prob comparison" on={impliedProb} onChange={setImpliedProb} />
            <Toggle label="Bias builders to priced markets" on={biasBuilders} onChange={setBiasBuilders} />
            {settings?.has_grok_key || settings?.proxy_url ? (
              <>
                <Toggle
                  label="Grok X/news sentiment (injuries, team news)"
                  on={useGrok}
                  onChange={setUseGrok}
                />
                {useGrok && (
                  <div className="pl-3 border-l border-edge space-y-2">
                    <Toggle
                      label="Hard rule: drop players Grok flags OUT"
                      on={grokVeto}
                      onChange={setGrokVeto}
                    />
                    <div>
                      <div className="text-[11px] text-slate-500 mb-1">
                        Grok fetches (fewer = cheaper &amp; faster)
                      </div>
                      <div className="flex flex-wrap gap-1.5">
                        {GROK_CATEGORIES.map((c) => {
                          const on = grokCats.has(c.id);
                          return (
                            <button
                              key={c.id}
                              className={`chip text-xs ${on ? "chip-on" : ""}`}
                              onClick={() =>
                                setGrokCats((prev) => {
                                  const n = new Set(prev);
                                  n.has(c.id) ? n.delete(c.id) : n.add(c.id);
                                  return n;
                                })
                              }
                            >
                              {c.label}
                            </button>
                          );
                        })}
                      </div>
                    </div>
                  </div>
                )}
              </>
            ) : (
              <div className="flex items-center justify-between opacity-50">
                <span className="text-sm">Grok X/news sentiment</span>
                <span className="text-[11px] text-slate-500">add key in Settings</span>
              </div>
            )}
            <textarea
              className="w-full rounded-lg bg-ink border border-edge px-3 py-2 text-sm"
              rows={2}
              placeholder="My notes (optional)"
              value={notes}
              onChange={(e) => setNotes(e.target.value)}
            />
          </div>
          </>
          )}

          {buildTab === "board" && (
            <div className="card text-sm text-slate-300 space-y-1">
              <div className="font-semibold">🧮 Build your own</div>
              <p className="text-[11px] text-slate-500">
                Every data-backed pick across your matches as a board — tap to compose your own
                tickets, evaluate them, and place. Uses your selected markets.
              </p>
            </div>
          )}

          {buildTab === "bankers" && (
            <div className="card text-sm text-slate-300 space-y-1">
              <div className="font-semibold">🏦 Bankers board</div>
              <p className="text-[11px] text-slate-500">
                Only the safest, most repeatable legs — high likelihood, recurring events (cards,
                shots, corners…), strong recent form, must-play. The picks you anchor an acca on.
                Deterministic, no model call.
              </p>
            </div>
          )}

          {buildTab === "acca" && (
          <div className="card space-y-3">
            <div className="text-xs font-semibold text-slate-400">🪜 Acca ladder settings</div>
            <div>
              <div className="text-[11px] text-slate-500 mb-1">Tickets — {ladderCount}</div>
              <input
                type="range"
                min={2}
                max={20}
                step={1}
                value={ladderCount}
                onChange={(e) => setLadderCount(parseInt(e.target.value, 10))}
                className="w-full"
              />
            </div>
            <div>
              <div className="text-[11px] text-slate-500 mb-1">
                Max appearances per player — {ladderMaxPerSubject}{" "}
                <span className="text-slate-500">(diversity; 1 = each star in just one ticket)</span>
              </div>
              <div className="flex gap-1.5">
                {[1, 2, 3, 4, 5].map((n) => (
                  <button
                    key={n}
                    className={`chip flex-1 text-center ${ladderMaxPerSubject === n ? "chip-on" : ""}`}
                    onClick={() => setLadderMaxPerSubject(n)}
                  >
                    {n}
                  </button>
                ))}
              </div>
            </div>
            <div>
              <div className="text-[11px] text-slate-500 mb-1">
                Legs per ticket — {ladderMinLegs}–{ladderMaxLegs === 0 ? "∞" : ladderMaxLegs}
              </div>
              <div className="flex gap-1.5 mb-1">
                <span className="text-[10px] text-slate-500 w-7 shrink-0 self-center">min</span>
                {[2, 3, 4, 5, 6].map((n) => (
                  <button
                    key={n}
                    className={`chip flex-1 text-center ${ladderMinLegs === n ? "chip-on" : ""}`}
                    onClick={() => {
                      setLadderMinLegs(n);
                      if (ladderMaxLegs !== 0 && n > ladderMaxLegs) setLadderMaxLegs(n);
                    }}
                  >
                    {n}
                  </button>
                ))}
              </div>
              <div className="flex gap-1.5">
                <span className="text-[10px] text-slate-500 w-7 shrink-0 self-center">max</span>
                {[3, 4, 6, 8, 10, 12, 0].map((n) => (
                  <button
                    key={n}
                    className={`chip flex-1 text-center ${ladderMaxLegs === n ? "chip-on" : ""}`}
                    title={n === 0 ? "No cap — legs keep stacking until the hit-target floor or the pool runs out" : undefined}
                    onClick={() => {
                      setLadderMaxLegs(n);
                      if (n !== 0 && n < ladderMinLegs) setLadderMinLegs(n);
                    }}
                  >
                    {n === 0 ? "∞" : n}
                  </button>
                ))}
              </div>
            </div>
            <OddsBand min={oddsMin} max={oddsMax} setMin={setOddsMin} setMax={setOddsMax} />
            <Toggle
              label="🚀 Mega acca"
              on={ladderMega}
              onChange={(on: boolean) => {
                setLadderMega(on);
                if (on) setLadderOnePerFixture(false); // mutually exclusive shapes
              }}
            />
            {ladderMega && (
              <p className="text-[10px] text-slate-500 -mt-1">
                ONE giant accumulator: the best 2–3 legs from EVERY selected match, each leg priced
                in the 1.2–2.0 sweet spot (set the odds band above to override). All plausible,
                all stacked — watch it grow. Deterministic — no model tokens.
              </p>
            )}
            <Toggle
              label="🧩 Cover every match"
              on={coverAll}
              onChange={setCoverAll}
            />
            {coverAll && (
              <div className="-mt-1 space-y-1">
                <div className="flex gap-1.5 items-center">
                  <span className="text-[10px] text-slate-500 shrink-0">legs per match</span>
                  {[1, 2, 3].map((n) => (
                    <button
                      key={n}
                      className={`chip flex-1 text-center ${coverLegs === n ? "chip-on" : ""}`}
                      onClick={() => setCoverLegs(n)}
                    >
                      {n}
                    </button>
                  ))}
                </div>
                <p className="text-[10px] text-slate-500">
                  Every ticket includes at least {coverLegs} leg{coverLegs > 1 ? "s" : ""} from EVERY
                  selected match. The min-hit target is ignored in this mode — it would stop the fill
                  before coverage and be deceiving.
                </p>
              </div>
            )}
            <Toggle
              label="🎯 One leg per match"
              on={ladderOnePerFixture}
              onChange={(on: boolean) => {
                setLadderOnePerFixture(on);
                if (on) setLadderMega(false);
              }}
            />
            {ladderOnePerFixture && (
              <p className="text-[10px] text-slate-500 -mt-1">
                Max one leg from each match, so legs are truly independent — e.g. select 10 matches +
                the SOT market, min prob ~60%: the 10 best shooters, one per game, priced at the
                product of their odds. Deterministic — no model tokens.
              </p>
            )}
            <Toggle
              label="Reset diversity on “Add more”"
              on={ladderDiversityReset}
              onChange={setLadderDiversityReset}
            />
            <div>
              <div className="text-[11px] text-slate-500 mb-1">
                Min hit chance — {Math.round(ladderMinHit * 100)}% (riskiest ticket floor)
              </div>
              <input
                type="range"
                min={1}
                max={60}
                step={1}
                value={Math.round(ladderMinHit * 100)}
                onChange={(e) => setLadderMinHit(parseInt(e.target.value, 10) / 100)}
                className="w-full"
              />
            </div>
            <div>
              <div className="text-[11px] text-slate-500 mb-1">
                Min leg probability — {Math.round(ladderMinProb * 100)}% (lower = more value/longshot)
              </div>
              <input
                type="range"
                min={10}
                max={90}
                step={5}
                value={Math.round(ladderMinProb * 100)}
                onChange={(e) => setLadderMinProb(parseInt(e.target.value, 10) / 100)}
                className="w-full"
              />
            </div>
            <div>
              <div className="text-[11px] text-slate-500 mb-1">
                Over/Under side (goals, corners, shots)
              </div>
              <div className="flex gap-1.5">
                {[
                  ["auto", "Auto"],
                  ["over", "Over only"],
                  ["under", "Under only"],
                ].map(([id, label]) => (
                  <button
                    key={id}
                    className={`chip flex-1 text-center ${ladderOuSide === id ? "chip-on" : ""}`}
                    onClick={() => setLadderOuSide(id)}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </div>
            <div>
              <div className="text-[11px] text-slate-500 mb-1">Markets in the ladder</div>
              <div className="flex gap-1.5">
                {[
                  ["team", "Team / match"],
                  ["props", "Player props"],
                  ["mixed", "Mixed"],
                ].map(([id, label]) => (
                  <button
                    key={id}
                    className={`chip flex-1 text-center ${ladderScope === id ? "chip-on" : ""}`}
                    onClick={() => setLadderScope(id)}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </div>
          </div>
          )}

          {showAdvanced && (
          <div className="card space-y-2">
            <div className="text-xs font-semibold text-slate-400">Data inputs — all on by default</div>
            <p className="text-[11px] text-slate-500">
              Everything that feeds the engine is on by default. Turn off anything you find clouds
              judgement (also speeds up the build). The markets you pick above already decide which
              player/team stats are used.
            </p>
            <Toggle label="Real xG — recent form (slower, more requests)" on={useXg} onChange={setUseXg} />
            <Toggle label="Coach & tactics play-style (Haiku, cached)" on={useTactics} onChange={setUseTactics} />
            <Toggle label="Confirmed lineups (starting XI only)" on={useLineups} onChange={setUseLineups} />
            <Toggle label="API-Football predictions (win% / advice)" on={usePredictions} onChange={setUsePredictions} />
            <Toggle label="League standings (motivation)" on={useStandings} onChange={setUseStandings} />
            <Toggle label="Head-to-head history" on={useH2h} onChange={setUseH2h} />
            <Toggle label="Weather at venue" on={useWeather} onChange={setUseWeather} />
            <details className="text-[11px] text-slate-500">
              <summary className="cursor-pointer">What we pull from API-Football</summary>
              <ul className="list-disc pl-4 mt-1 space-y-0.5">
                <li>Team season stats: goals for/against avg, PPG, 1st-half share, failed-to-score</li>
                <li>Real xG (opt-in): expected_goals from each team's last 8 fixtures</li>
                <li>Player season stats: goals, shots, SOT, tackles, fouls, cards, passes, minutes</li>
                <li>Odds: Pinnacle (de-vigged true prob) + line-shopped book prices</li>
                <li>Injuries, confirmed lineups, standings, head-to-head, predictions, live score</li>
              </ul>
            </details>
          </div>
          )}

          <Sticky>
            {prewarmBusy && prewarmProgress && (
              <div className="card space-y-1 mb-2">
                <div className="text-xs text-slate-300 inline-flex items-center gap-2">
                  <Spinner /> Pre-scoring plausibility… {prewarmProgress.done}/{prewarmProgress.total} fixtures
                </div>
                <div className="h-2 rounded-full bg-edge overflow-hidden">
                  <div
                    className="h-full bg-accent transition-all"
                    style={{ width: `${(prewarmProgress.done / Math.max(1, prewarmProgress.total)) * 100}%` }}
                  />
                </div>
                <div className="text-[10px] text-slate-500">One-time per slate — generate unlocks when done. Cached after.</div>
              </div>
            )}
            {buildTab === "regular" && (
              <button className="btn btn-primary w-full" disabled={(selMarkets.size === 0 && strategy !== "predictor" && strategy !== "scout") || busy || prewarmBusy} onClick={() => build()}>
                {busy ? (
                  <span className="inline-flex items-center gap-2"><Spinner /> Building…</span>
                ) : prewarmBusy ? (
                  "🔒 Pre-scoring…"
                ) : (
                  `🎯 Build ${ticketCount} tickets`
                )}
              </button>
            )}
            {buildTab === "acca" && (
              <button
                className="btn btn-primary w-full"
                disabled={selMarkets.size === 0 || busy || prewarmBusy}
                onClick={() => buildLadder()}
              >
                {busy ? (
                  <span className="inline-flex items-center gap-2"><Spinner /> Building…</span>
                ) : prewarmBusy ? (
                  "🔒 Pre-scoring…"
                ) : (
                  `🪜 Build acca ladder (${ladderCount})`
                )}
              </button>
            )}
            {buildTab === "board" && (
              <button className="btn btn-primary w-full" disabled={busy || prewarmBusy} onClick={() => { setDataTab("picks"); setShowBoard(true); }}>
                {prewarmBusy ? "🔒 Pre-scoring…" : "🧮 Open picks board"}
              </button>
            )}
            {buildTab === "bankers" && (
              <button className="btn btn-primary w-full" disabled={busy || prewarmBusy} onClick={() => { setDataTab("bankers"); setShowBoard(true); }}>
                {prewarmBusy ? "🔒 Pre-scoring…" : "🏦 Open bankers board"}
              </button>
            )}
            {busy && (
              <div className="text-xs text-slate-400 mt-2 text-center inline-flex items-center justify-center gap-2 w-full">
                <Spinner />
                Fetching… Computing features…{buildTab === "regular" ? " Asking the model…" : ""}
                <button
                  className="text-bad underline hover:brightness-125"
                  title="Stop this build — it aborts at the next checkpoint (no model tokens are billed for an aborted call)"
                  onClick={() => api.cancelBuild().catch(() => {})}
                >
                  ✕ Cancel
                </button>
              </div>
            )}
          </Sticky>
          </>)}
        </div>
      )}

      {/* RESULTS */}
      {step === "results" && result && (
        <div className="space-y-3">
          <Header title="Tickets" onBack={() => setStep("markets")} />
          <div className="flex items-center justify-between -mt-1">
            <p className="text-xs text-slate-400">Step 4 of 4 — review, tweak the stake, and place. Tap a ticket to expand it.</p>
            <button
              className="text-xs text-slate-400 underline hover:text-slate-200 shrink-0 ml-2"
              title="Jump back to the start for a fresh slate — clears the match selection; these tickets stay reachable via the step dots until you build again"
              onClick={() => {
                setSelFixtureIds(new Set());
                setStep("date");
              }}
            >
              ↺ Start over
            </button>
          </div>
          {busy && (
            <div className="text-xs text-accent inline-flex items-center gap-2">
              <Spinner /> Generating a fresh set…
              <button
                className="text-bad underline hover:brightness-125"
                title="Stop this build"
                onClick={() => api.cancelBuild().catch(() => {})}
              >
                ✕ Cancel
              </button>
            </div>
          )}
          <Results
            result={result}
            usage={usage}
            saved={saved}
            busy={busy}
            bankroll={bankroll}
            kellyFraction={settings?.kelly_fraction ?? 0}
            defaultStake={settings?.default_stake ?? 0.5}
            leagues={leagueByFixtureId}
            onSave={saveCurrent}
            onCopy={copyCurrent}
            onNewSet={newSet}
            onExport={exportCsv}
            onPlace={placeTicket}
            onVoidSubject={onVoidSubject}
            cartKeys={cartKeys}
            onToggleCartLeg={toggleCartLeg}
          />
          <CustomSlip
            legs={cart}
            bankroll={bankroll}
            kellyFraction={settings?.kelly_fraction ?? 0}
            defaultStake={settings?.default_stake ?? 0.5}
            leagues={leagueByFixtureId}
            onRemove={removeCartLeg}
            onClear={() => setCart([])}
            onPlace={(t, stake, odds) => placeTicket(t, stake, odds, "custom")}
            onSaveToPicks={(legs) => {
              setMyPicks((prev) => {
                const seen = new Set(prev.map(legKey));
                return [...prev, ...legs.filter((l) => !seen.has(legKey(l)))];
              });
              setCart([]);
              toast.success("Saved to 📌 My Picks (Data → Mine).");
            }}
          />
          {buildStrategy === "ladder" && (
            <button
              className="btn btn-ghost w-full"
              disabled={busy}
              onClick={() => buildLadder(true)}
            >
              {busy ? (
                <span className="inline-flex items-center gap-2"><Spinner /> Adding…</span>
              ) : (
                `➕ Add ${ladderCount} more ${ladderDiversityReset ? "(fresh pool)" : "(keep diversity)"}`
              )}
            </button>
          )}
        </div>
      )}
    </Shell>
  );
}

// ---------- small helpers / subcomponents ----------

function OddsBand({
  min,
  max,
  setMin,
  setMax,
}: {
  min: number;
  max: number;
  setMin: (v: number) => void;
  setMax: (v: number) => void;
}) {
  const active = min > 1.01 || max < 999;
  return (
    <div>
      <div className="text-[11px] text-slate-500 mb-1">
        Per-leg odds band —{" "}
        {active ? `${min.toFixed(2)} – ${max >= 999 ? "∞" : max.toFixed(2)}` : "off (any price)"}
        <span className="text-slate-500"> · skips chalk &amp; lottery prices</span>
      </div>
      <div className="flex items-center gap-1.5 flex-wrap">
        <input
          type="number"
          step="0.05"
          inputMode="decimal"
          className="w-16 rounded-lg bg-ink border border-edge px-2 py-1.5 text-sm"
          value={min}
          onChange={(e) => setMin(Math.max(1, parseFloat(e.target.value) || 1))}
        />
        <span className="text-slate-500 text-xs">to</span>
        <input
          type="number"
          step="0.5"
          inputMode="decimal"
          placeholder="∞"
          className="w-16 rounded-lg bg-ink border border-edge px-2 py-1.5 text-sm"
          value={max >= 999 ? "" : max}
          onChange={(e) => setMax(parseFloat(e.target.value) || 1000)}
        />
        <button className={`chip ${min === 1.3 && max === 7 ? "chip-on" : ""}`} onClick={() => { setMin(1.3); setMax(7); }}>
          1.3–7 sweet spot
        </button>
        <button className={`chip ${!active ? "chip-on" : ""}`} onClick={() => { setMin(1); setMax(1000); }}>
          off
        </button>
      </div>
    </div>
  );
}

function toggle(setter: React.Dispatch<React.SetStateAction<Set<number>>>, id: number) {
  setter((prev) => {
    const next = new Set(prev);
    next.has(id) ? next.delete(id) : next.add(id);
    return next;
  });
}

const NAV_ITEMS: { id: Overlay; icon: string; label: string }[] = [
  { id: null, icon: "🏠", label: "Build" },
  { id: "tracker", icon: "💰", label: "Tracker" },
  { id: "ledger", icon: "📊", label: "Ledger" },
  { id: "live", icon: "🔴", label: "Live" },
  { id: "newsfeed", icon: "📰", label: "News" },
  { id: "ingest", icon: "🧲", label: "Ingest" },
  { id: "history", icon: "🗂", label: "History" },
  { id: "settings", icon: "⚙", label: "Settings" },
];

function Shell({
  children,
  meter,
  cost,
  grokCost,
  onCoins,
  onNav,
  current,
  onInspect,
  canInspect,
  ingestBadge = 0,
}: {
  children: React.ReactNode;
  meter: RequestMeter | null;
  cost?: number;
  grokCost?: number;
  onCoins?: () => void;
  onNav?: (o: Overlay) => void;
  current?: Overlay;
  onInspect?: () => void;
  canInspect?: boolean;
  ingestBadge?: number;
}) {
  const navBtn = (active: boolean) =>
    `flex items-center gap-1 px-2.5 py-1 rounded-lg text-xs whitespace-nowrap transition ${
      active ? "bg-accent/15 text-accent" : "text-slate-400 hover:text-slate-100 hover:bg-edge"
    }`;
  return (
    <div className="min-h-full max-w-2xl mx-auto flex flex-col">
      <header className="border-b border-edge">
        <div className="flex items-center justify-between px-4 py-3">
          <img src="/powabetz-name.png" alt="POWABETZ" className="h-6 w-auto select-none" draggable={false} />
          <div className="flex items-center gap-3">
            <MeterBar meter={meter} />
            <Coins claude={cost ?? 0} grok={grokCost ?? 0} onClick={onCoins} />
          </div>
        </div>
        <nav className="flex flex-wrap items-center gap-1 px-2 pb-2">
          {NAV_ITEMS.map((it) => (
            <button key={it.label} className={`relative ${navBtn(current === it.id)}`} onClick={() => onNav?.(it.id)}>
              <span>{it.icon}</span>
              <span>{it.label}</span>
              {it.id === "ingest" && ingestBadge > 0 && (
                <span
                  className="absolute -top-1 -right-1 min-w-[15px] h-[15px] px-1 rounded-full bg-bad text-white text-[9px] font-bold flex items-center justify-center"
                  title={`${ingestBadge} ingested page${ingestBadge > 1 ? "s" : ""} not yet processed by AI`}
                >
                  {ingestBadge}
                </span>
              )}
            </button>
          ))}
          {onInspect && (
            <button
              className={
                canInspect
                  ? navBtn(false)
                  : "flex items-center gap-1 px-2.5 py-1 rounded-lg text-xs whitespace-nowrap text-slate-700 cursor-not-allowed"
              }
              onClick={() => canInspect && onInspect()}
              title={canInspect ? "All picks — data board (every prop, our true probability)" : "Select matches first"}
            >
              <span>📊</span>
              <span>Data</span>
            </button>
          )}
        </nav>
      </header>
      <main className="flex-1 p-4 pb-28">{children}</main>
    </div>
  );
}

const STEP_LABELS: Record<Step, string> = { date: "Date", matches: "Matches", markets: "Markets", results: "Tickets" };
function StepDots({ step, onJump, hasResult }: { step: Step; onJump?: (s: Step) => void; hasResult?: boolean }) {
  const order: Step[] = ["date", "matches", "markets", "results"];
  const i = order.indexOf(step);
  return (
    <div className="flex gap-1.5 mb-4">
      {order.map((s, k) => {
        // Backward jumps always allowed (state is kept). Forward only to an
        // EXISTING result — Back from Results used to be a one-way door: the
        // tickets stayed in memory but the only way to see them again was to
        // build again.
        const jumpable = k < i || (s === "results" && !!hasResult && k > i);
        return (
          <button
            key={k}
            disabled={!jumpable || !onJump}
            onClick={() => jumpable && onJump?.(s)}
            title={jumpable ? (k < i ? `Back to ${STEP_LABELS[s]}` : "View your tickets") : STEP_LABELS[s]}
            className={`h-1.5 flex-1 rounded-full transition ${k <= i || (s === "results" && hasResult) ? "bg-accent" : "bg-edge"} ${jumpable ? "cursor-pointer hover:opacity-80" : "cursor-default"}`}
          />
        );
      })}
    </div>
  );
}

function Header({ title, onBack }: { title: string; onBack: () => void }) {
  return (
    <div className="flex items-center gap-2">
      <button className="btn btn-ghost text-sm py-2 px-3" onClick={onBack}>
        ←
      </button>
      <h2 className="text-lg font-bold">{title}</h2>
    </div>
  );
}

function Sticky({ children }: { children: React.ReactNode }) {
  return (
    <div className="fixed bottom-0 left-0 right-0 max-w-md mx-auto p-4 bg-ink/95 border-t border-edge">
      {children}
    </div>
  );
}

function Toggle({ label, on, onChange }: { label: string; on: boolean; onChange: (v: boolean) => void }) {
  return (
    <button className="flex items-center justify-between w-full" onClick={() => onChange(!on)}>
      <span className="text-sm">{label}</span>
      <span className={`w-11 h-6 rounded-full p-0.5 transition ${on ? "bg-accent" : "bg-edge"}`}>
        <span
          className={`block w-5 h-5 rounded-full bg-white transition ${on ? "translate-x-5" : ""}`}
        />
      </span>
    </button>
  );
}

function LeaguePicker({
  leagues,
  search,
  selected,
  onToggle,
}: {
  leagues: LeagueOption[];
  search: string;
  selected: Set<number>;
  onToggle: (id: number) => void;
}) {
  if (leagues.length === 0) {
    return (
      <div className="text-xs text-slate-500 py-2">
        No leagues loaded yet — add an API-Football key (or a working proxy URL) in Settings,
        then reopen this screen.
      </div>
    );
  }

  const q = search.trim().toLowerCase();
  const sel = leagues.filter((l) => selected.has(l.id));
  const rest = leagues.filter(
    (l) =>
      !selected.has(l.id) &&
      (q === "" || l.name.toLowerCase().includes(q) || l.country.toLowerCase().includes(q))
  );
  const CAP = 60;
  const shown = rest.slice(0, CAP);
  const more = rest.length - shown.length;

  const chip = (l: LeagueOption) => {
    const on = selected.has(l.id);
    return (
      <button
        key={l.id}
        className={`chip ${on ? "chip-on" : ""}`}
        title={l.country}
        onClick={() => onToggle(l.id)}
      >
        {l.name}
        {l.country && <span className="ml-1 text-[10px] text-slate-500">{l.country}</span>}
        {l.picks > 0 && <span className="ml-1 text-[10px] text-accent">×{l.picks}</span>}
      </button>
    );
  };

  return (
    <div className="max-h-64 overflow-y-auto pr-1">
      <div className="flex flex-wrap gap-2">
        {sel.map(chip)}
        {shown.map(chip)}
      </div>
      {more > 0 && (
        <div className="text-[11px] text-slate-500 mt-2">+{more} more — search to narrow.</div>
      )}
    </div>
  );
}

function MarketChip({
  k,
  label,
  sel,
  setSel,
}: {
  k: string;
  label: string;
  sel: Set<string>;
  setSel: React.Dispatch<React.SetStateAction<Set<string>>>;
}) {
  const on = sel.has(k);
  return (
    <button
      className={`chip ${on ? "chip-on" : ""}`}
      onClick={() =>
        setSel((prev) => {
          const next = new Set(prev);
          next.has(k) ? next.delete(k) : next.add(k);
          return next;
        })
      }
    >
      {label}
    </button>
  );
}
