import { useEffect, useMemo, useRef, useState } from "react";
import { errMsg, toast } from "../toast";
import { api } from "../api";
import Spinner from "./Spinner";
import Hint from "./Hint";
import StakeBumps from "./StakeBumps";
import ForecastPanel from "./ForecastPanel";
import { ANALYSIS_MODELS, type LiveFixture, type LiveSnapshot, type LiveTicket, type MatchForecast } from "../types";

export default function Live({ onClose, defaultStake = 0.5, buildModel = "claude-opus-4-8", onPlaced }: { onClose: () => void; defaultStake?: number; buildModel?: string; onPlaced?: () => void }) {
  const [fixtures, setFixtures] = useState<LiveFixture[] | null>(null);
  const [snap, setSnap] = useState<LiveSnapshot | null>(null);
  const [sel, setSel] = useState<LiveFixture | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [query, setQuery] = useState("");

  const shown = useMemo(() => {
    if (!fixtures) return null;
    const q = query.trim().toLowerCase();
    if (!q) return fixtures;
    return fixtures.filter(
      (f) =>
        f.home_team.toLowerCase().includes(q) ||
        f.away_team.toLowerCase().includes(q) ||
        f.league_name.toLowerCase().includes(q)
    );
  }, [fixtures, query]);

  async function loadList() {
    setBusy(true);
    setErr(null);
    try {
      setFixtures(await api.liveFixtures());
    } catch (e) {
      setErr(errMsg(e));
    } finally {
      setBusy(false);
    }
  }
  useEffect(() => {
    loadList();
  }, []);

  async function open(f: LiveFixture) {
    setSel(f);
    setSnap(null);
    setBusy(true);
    setErr(null);
    try {
      setSnap(await api.liveSnapshot(f));
    } catch (e) {
      setErr(errMsg(e));
    } finally {
      setBusy(false);
    }
  }

  if (sel) {
    return (
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <button className="btn btn-ghost text-sm py-2" onClick={() => { setSel(null); setSnap(null); }}>
            ← Live
          </button>
          <button className="btn btn-ghost text-sm py-2" onClick={() => open(sel)} disabled={busy}>
            {busy ? <Spinner /> : "↻ refresh"}
          </button>
        </div>
        <SnapshotView snap={snap} busy={busy} err={err} fallback={sel} defaultStake={defaultStake} buildModel={buildModel} onPlaced={onPlaced} />
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-bold">🔴 In-play</h2>
        <div className="flex gap-2">
          <button className="btn btn-ghost text-sm py-2" onClick={loadList} disabled={busy}>
            {busy ? <Spinner /> : "↻"}
          </button>
          <button className="btn btn-ghost text-sm py-2" onClick={onClose}>Done</button>
        </div>
      </div>
      <p className="text-[11px] text-slate-500">
        On-demand only — each open/refresh costs a few requests. Tap a match (half-time games are listed first) to
        see current stats + our estimate for the time remaining.
      </p>
      {err && <div className="text-xs text-bad">{err}</div>}
      {fixtures && fixtures.length > 0 && (
        <input
          className="w-full rounded-lg bg-ink border border-edge px-3 py-2 text-sm"
          placeholder="Filter by team or league…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      )}
      {!fixtures && busy && <div className="text-sm text-slate-400 inline-flex items-center gap-2"><Spinner /> Loading live matches…</div>}
      {fixtures && fixtures.length === 0 && <div className="card text-sm text-slate-400">No matches in play right now.</div>}
      {shown && shown.length === 0 && <div className="text-xs text-slate-500">No live matches match "{query}".</div>}
      {shown?.map((f) => (
        <button key={f.fixture_id} className="card w-full text-left hover:border-accent/50" onClick={() => open(f)}>
          <div className="flex items-center justify-between gap-2">
            <div className="min-w-0">
              <div className="text-sm font-semibold truncate">
                {f.home_team} <span className="text-accent">{f.home_goals}–{f.away_goals}</span> {f.away_team}
              </div>
              <div className="text-[11px] text-slate-500 truncate">{f.league_name}</div>
            </div>
            <span className={`badge shrink-0 ${f.status === "HT" ? "bg-warn/25 text-warn" : "bg-bad/20 text-bad"}`}>
              {f.status === "HT" ? "HT" : `${f.elapsed}'`}
            </span>
          </div>
        </button>
      ))}
    </div>
  );
}

function SnapshotView({ snap, busy, err, fallback, defaultStake, buildModel, onPlaced }: { snap: LiveSnapshot | null; busy: boolean; err: string | null; fallback: LiveFixture; defaultStake: number; buildModel: string; onPlaced?: () => void }) {
  const f = snap?.fixture ?? fallback;
  return (
    <div className="space-y-3">
      <div className="card">
        <div className="flex items-center justify-between">
          <div className="text-base font-bold">
            {f.home_team} <span className="text-accent">{f.home_goals}–{f.away_goals}</span> {f.away_team}
          </div>
          <span className={`badge ${f.status === "HT" ? "bg-warn/25 text-warn" : "bg-bad/20 text-bad"}`}>
            {f.status === "HT" ? "Half-time" : `${f.elapsed}'`}
          </span>
        </div>
        <div className="text-[11px] text-slate-500">{f.league_name}</div>
      </div>

      <LivePredict fixture={f} buildModel={buildModel} />

      {busy && !snap && <div className="text-sm text-slate-400 inline-flex items-center gap-2"><Spinner /> Reading live data…</div>}
      {err && <div className="text-xs text-bad">{err}</div>}

      {snap && (
        <>
          {snap.estimates.length > 0 && (
            <div className="card space-y-1">
              <div className="text-xs font-semibold text-accent">📈 Likely in the time remaining (our estimate)</div>
              {snap.estimates.map((e, i) => (
                <div key={i} className="flex items-center justify-between text-xs">
                  <span className="text-slate-300 min-w-0 truncate">
                    {e.label} <span className="text-slate-500">· {e.basis}</span>
                    {e.edge != null && e.book && (
                      <span className={e.edge > 0 ? "text-accent" : "text-slate-500"}>
                        {" "}· {e.edge > 0 ? "+" : ""}{Math.round(e.edge * 100)}% vs {e.book}
                      </span>
                    )}
                  </span>
                  <b className="text-slate-100 shrink-0 ml-2">{Math.round(e.prob * 100)}%</b>
                </div>
              ))}
            </div>
          )}

          {snap.stats.length > 0 && (
            <div className="card">
              <div className="text-xs font-semibold text-slate-400 mb-1">Live stats</div>
              <StatsTable snap={snap} />
            </div>
          )}

          {snap.odds.length > 0 && (
            <div className="card space-y-1">
              <div className="text-xs font-semibold text-slate-400">In-play odds (implied %)</div>
              {snap.odds.slice(0, 18).map((o, i) => (
                <div key={i} className="flex items-center justify-between text-[11px]">
                  <span className="text-slate-400 min-w-0 truncate">{o.market} — <span className="text-slate-200">{o.selection}</span></span>
                  <span className="shrink-0 ml-2 text-slate-300">{o.odds.toFixed(2)} · {Math.round(o.implied * 100)}%</span>
                </div>
              ))}
              <p className="text-[10px] text-slate-500">Compare an estimate above to the same market's implied % — your edge is the gap.</p>
            </div>
          )}

          {snap.events.length > 0 && (
            <div className="card space-y-0.5">
              <div className="text-xs font-semibold text-slate-400 mb-1">Match events</div>
              {snap.events.slice().reverse().slice(0, 12).map((e, i) => (
                <div key={i} className="text-[11px] text-slate-400">
                  <span className="text-slate-500">{e.minute}'</span> {iconFor(e.kind)} <b className="text-slate-300">{e.player}</b>
                  <span className="text-slate-500"> · {e.team}{e.detail ? ` · ${e.detail}` : ""}</span>
                </div>
              ))}
            </div>
          )}

          <TicketBuilder fixture={snap.fixture} defaultStake={defaultStake} onPlaced={onPlaced} />

          <p className="text-[10px] text-slate-500">{snap.note}</p>
        </>
      )}
    </div>
  );
}

// One-tap: run the full Match Predictor for this fixture (live-aware) and show
// the forecast — no need to leave the Live screen.
function LivePredict({ fixture, buildModel }: { fixture: LiveFixture; buildModel: string }) {
  const [forecast, setForecast] = useState<MatchForecast | null>(null);
  const [busy, setBusy] = useState(false);

  async function predict() {
    setBusy(true);
    try {
      const resp = await api.buildTickets({
        fixtures: [
          {
            fixture_id: fixture.fixture_id,
            league_id: fixture.league_id,
            season: fixture.season,
            home_team_id: fixture.home_team_id,
            home_team: fixture.home_team,
            away_team_id: fixture.away_team_id,
            away_team: fixture.away_team,
          },
        ],
        markets: [],
        reasoning: true,
        implied_prob: false,
        notes: "",
        model: buildModel,
        ticket_types: ["SGP"],
        variation: 0,
        exclude: [],
        bias_builders: false,
        most_likely: false,
        strategy: "predictor",
        max_leg_prob: 1,
        use_grok: false,
        grok_veto: false,
        grok_categories: [],
        use_weather: false,
        use_standings: false,
        use_h2h: false,
        use_lineups: false,
        use_predictions: false,
        use_xg: true,
        use_tactics: false,
        lucky_safe: 0,
        lucky_moderate: 0,
        lucky_risky: 0,
        use_ingest: true,
        min_legs: null,
        min_odds: null,
        max_odds: null,
        max_per_subject: null,
        use_plausibility: false,
        // Only the forecast is shown here — skip the model call entirely
        // (this used to burn a full premium build and discard the tickets).
        forecast_only: true,
      });
      setForecast(resp.result.forecast ?? null);
      if (!resp.result.forecast) toast.info("No forecast available for this match yet.");
    } catch (e) {
      toast.error(e);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-2">
      <button className="btn btn-ghost w-full text-sm py-2 border border-accent/40" onClick={predict} disabled={busy}>
        {busy ? <Spinner /> : forecast ? "🔮 Re-predict (live-adjusted)" : "🔮 Predict full match"}
      </button>
      {forecast && (
        <ForecastPanel
          f={forecast}
          footer={
            forecast.headline.startsWith("⚡")
              ? "Live-adjusted from the current score & time remaining (the minute is in the header — re-predict as the game moves)."
              : "Pre-match forecast — this match isn't in-play yet (or live data was unavailable)."
          }
        />
      )}
    </div>
  );
}

function TicketBuilder({ fixture, defaultStake, onPlaced }: { fixture: LiveFixture; defaultStake: number; onPlaced?: () => void }) {
  const [model, setModel] = useState(ANALYSIS_MODELS[0].id);
  const [pool, setPool] = useState<LiveTicket | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [stake, setStake] = useState(defaultStake > 0 ? defaultStake.toFixed(2) : "");
  const [sel, setSel] = useState<Set<number>>(new Set());

  async function build() {
    setBusy(true);
    setErr(null);
    setSel(new Set());
    try {
      setPool(await api.liveTicket(fixture, model));
    } catch (e) {
      setErr(errMsg(e));
    } finally {
      setBusy(false);
    }
  }

  const picks = pool?.legs ?? [];
  const chosen = [...sel].map((i) => picks[i]).filter(Boolean);
  const allPriced = chosen.length > 0 && chosen.every((p) => p.odds != null);
  const comboOdds = allPriced ? chosen.reduce((a, p) => a * (p.odds as number), 1) : null;
  const placingRef = useRef(false); // sync double-tap guard

  async function place() {
    // Sync guard before the await — double-tap used to place duplicate bets.
    if (placingRef.current) return;
    placingRef.current = true;
    try {
    if (chosen.length === 0) {
      toast.error("Tap one or more picks first.");
      return;
    }
    const s = parseFloat(stake);
    if (!Number.isFinite(s) || s <= 0) {
      toast.error("Enter a stake greater than 0.");
      return;
    }
    const match = `${fixture.home_team} vs ${fixture.away_team}`;
    const ticketObj = {
      type: chosen.length === 1 ? "Live" : "Live combo",
      title: `Live ${fixture.elapsed}': ${chosen.map((p) => p.label).slice(0, 2).join(" + ")}${chosen.length > 2 ? "…" : ""}`,
      confidence: pool?.confidence ?? "",
      legs: chosen.map((p) => ({
        match,
        fixture_id: fixture.fixture_id,
        market: "Live",
        selection: p.label,
        line: null,
        est_prob: p.prob,
        book_odds: p.odds,
      })),
      combined_prob: chosen.reduce((a, p) => a * p.prob, 1),
      combined_odds: comboOdds,
      combined_ev: null,
      flags: ["in-play"],
      why: pool?.rationale ?? null,
    };
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      await api.placeBet(ticketObj as any, s, comboOdds, false, "live");
    } catch (e) {
      toast.error(e);
      return;
    }
    toast.success(`Live bet placed — $${s.toFixed(2)} · ${chosen.length} pick${chosen.length > 1 ? "s" : ""} · Live 🔴`);
    setSel(new Set());
    onPlaced?.();
    } finally {
      placingRef.current = false;
    }
  }

  return (
    <div className="card space-y-2 border-accent/30">
      <div className="flex items-center justify-between gap-2">
        <div className="text-xs font-semibold text-accent">
          🎯 Live picks pool
          <Hint text="The best individual in-play bets right now (incl. live player props — who's actually shooting). One cached model call per game state. Tap picks to select, then Place — one pick = a single, several = your own combo." />
        </div>
        <select className="rounded-lg bg-ink border border-edge px-2 py-1 text-[11px]" value={model} onChange={(e) => setModel(e.target.value)}>
          {ANALYSIS_MODELS.map((m) => (
            <option key={m.id} value={m.id}>{m.label}</option>
          ))}
        </select>
      </div>
      <p className="text-[10px] text-slate-500">
        A pool of standalone singles (team + live player props) for the current game state. Tap the ones you want — place a single or build your own combo.
      </p>
      <button className="btn btn-primary w-full text-sm py-2" onClick={build} disabled={busy}>
        {busy ? <Spinner /> : pool ? "Refresh picks" : "Get live picks"}
      </button>
      {err && <div className="text-xs text-bad">{err}</div>}
      {pool && (
        <div className="space-y-1.5 pt-1">
          {pool.rationale && <p className="text-[11px] text-slate-300">{pool.rationale}</p>}
          {picks.map((l, i) => {
            const on = sel.has(i);
            return (
              <button
                key={i}
                className={`w-full text-left text-xs rounded-lg border px-2.5 py-1.5 ${on ? "border-accent bg-accent/10" : "border-edge bg-ink hover:border-accent/40"}`}
                onClick={() => setSel((p) => { const n = new Set(p); n.has(i) ? n.delete(i) : n.add(i); return n; })}
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="text-slate-100 min-w-0 break-words">{on ? "✓ " : ""}{l.label}</span>
                  <span className="shrink-0 text-slate-400">
                    {l.odds ? `${l.odds.toFixed(2)} · ` : ""}{Math.round(l.prob * 100)}%
                    <span className="text-slate-500"> {l.source === "book" ? "book" : "model"}</span>
                  </span>
                </div>
                {l.why && <div className="text-[10px] text-slate-500">{l.why}</div>}
              </button>
            );
          })}
          <p className="text-[10px] text-slate-500">{pool.note}{pool.cached ? " · cached" : ""}</p>

          <div className="space-y-1 border-t border-edge pt-1.5">
            <div className="flex items-center gap-1.5">
              <span className="text-sm text-slate-500">$</span>
              <input className="w-16 rounded-lg bg-ink border border-edge px-2 py-1.5 text-sm" placeholder="stake" inputMode="decimal" value={stake} onChange={(e) => setStake(e.target.value)} />
              <button className="btn btn-primary text-sm px-3 py-1.5 flex-1" onClick={place} disabled={chosen.length === 0}>
                {chosen.length === 0
                  ? "Select picks"
                  : `Place ${chosen.length} ${chosen.length === 1 ? "single" : "combo"}${comboOdds ? ` @ ${comboOdds.toFixed(2)}` : ""}`}
              </button>
            </div>
            <StakeBumps value={stake} onChange={setStake} />
          </div>
        </div>
      )}
    </div>
  );
}

function StatsTable({ snap }: { snap: LiveSnapshot }) {
  const labels = Array.from(new Set(snap.stats.flatMap((t) => t.stats.map((s) => s.label))));
  const val = (team: string, label: string) => snap.stats.find((t) => t.team === team)?.stats.find((s) => s.label === label)?.value ?? "–";
  const [a, b] = snap.stats;
  if (!a || !b) return null;
  return (
    <table className="w-full text-xs">
      <thead className="text-slate-500">
        <tr>
          <td className="text-right pb-1">{a.team}</td>
          <td className="text-center pb-1"></td>
          <td className="pb-1">{b.team}</td>
        </tr>
      </thead>
      <tbody>
        {labels.map((l) => (
          <tr key={l} className="border-t border-edge">
            <td className="text-right py-0.5 text-slate-200">{val(a.team, l)}</td>
            <td className="text-center py-0.5 text-slate-500 text-[10px]">{l}</td>
            <td className="py-0.5 text-slate-200">{val(b.team, l)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function iconFor(kind: string): string {
  const k = kind.toLowerCase();
  if (k === "goal") return "⚽";
  if (k === "card") return "🟨";
  if (k === "subst") return "🔁";
  if (k === "var") return "📺";
  return "•";
}
