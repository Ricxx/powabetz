import { useEffect, useState } from "react";
import { errMsg } from "../toast";
import { api } from "../api";
import Spinner from "./Spinner";
import Hint from "./Hint";
import { ANALYSIS_MODELS, type LiveFixture, type LiveSnapshot, type LiveTicket } from "../types";

export default function Live({ onClose }: { onClose: () => void }) {
  const [fixtures, setFixtures] = useState<LiveFixture[] | null>(null);
  const [snap, setSnap] = useState<LiveSnapshot | null>(null);
  const [sel, setSel] = useState<LiveFixture | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

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
        <SnapshotView snap={snap} busy={busy} err={err} fallback={sel} />
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
      {!fixtures && busy && <div className="text-sm text-slate-400 inline-flex items-center gap-2"><Spinner /> Loading live matches…</div>}
      {fixtures && fixtures.length === 0 && <div className="card text-sm text-slate-400">No matches in play right now.</div>}
      {fixtures?.map((f) => (
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

function SnapshotView({ snap, busy, err, fallback }: { snap: LiveSnapshot | null; busy: boolean; err: string | null; fallback: LiveFixture }) {
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

          <TicketBuilder fixture={snap.fixture} />

          <p className="text-[10px] text-slate-500">{snap.note}</p>
        </>
      )}
    </div>
  );
}

function TicketBuilder({ fixture }: { fixture: LiveFixture }) {
  const [model, setModel] = useState(ANALYSIS_MODELS[0].id);
  const [ticket, setTicket] = useState<LiveTicket | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function build() {
    setBusy(true);
    setErr(null);
    try {
      setTicket(await api.liveTicket(fixture, model));
    } catch (e) {
      setErr(errMsg(e));
    } finally {
      setBusy(false);
    }
  }

  const confColor = (c: string) => (c === "high" ? "text-accent" : c === "low" ? "text-bad" : "text-warn");

  return (
    <div className="card space-y-2 border-accent/30">
      <div className="flex items-center justify-between gap-2">
        <div className="text-xs font-semibold text-accent">
          🎟️ Build an in-play ticket
          <Hint text="Spends one cached model call per game state (rebuild is free until the score/minute changes). Cost order: GPT-5 nano cheapest, then Haiku, then GPT-5 mini. GPT needs an OpenAI key." />
        </div>
        <select
          className="rounded-lg bg-ink border border-edge px-2 py-1 text-[11px]"
          value={model}
          onChange={(e) => setModel(e.target.value)}
        >
          {ANALYSIS_MODELS.map((m) => (
            <option key={m.id} value={m.id}>{m.label}</option>
          ))}
        </select>
      </div>
      <p className="text-[10px] text-slate-500">
        Feeds the live score, stats, events and your ingested notes to the model — it assembles a coherent ticket from
        our estimates + the live odds. One cached call per game state.
      </p>
      <button className="btn btn-primary w-full text-sm py-2" onClick={build} disabled={busy}>
        {busy ? <Spinner /> : ticket ? "Rebuild" : "Build ticket"}
      </button>
      {err && <div className="text-xs text-bad">{err}</div>}
      {ticket && (
        <div className="space-y-1.5 pt-1">
          <div className="flex items-center justify-between text-[11px]">
            <span className={`font-semibold ${confColor(ticket.confidence)}`}>{ticket.confidence.toUpperCase()} confidence</span>
            <span className="text-slate-400">
              {ticket.combined_odds ? `~${ticket.combined_odds.toFixed(2)} · ` : ""}
              {Math.round(ticket.combined_prob * 100)}% combined
            </span>
          </div>
          {ticket.legs.map((l, i) => (
            <div key={i} className="text-xs border-t border-edge pt-1">
              <div className="flex items-center justify-between gap-2">
                <span className="text-slate-200 min-w-0">{l.label}</span>
                <span className="shrink-0 text-slate-400">
                  {l.odds ? `${l.odds.toFixed(2)} · ` : ""}{Math.round(l.prob * 100)}%
                  <span className="text-slate-500"> {l.source === "book" ? "book" : "model"}</span>
                </span>
              </div>
              {l.why && <div className="text-[10px] text-slate-500">{l.why}</div>}
            </div>
          ))}
          {ticket.rationale && <p className="text-[11px] text-slate-300 pt-1">{ticket.rationale}</p>}
          <p className="text-[10px] text-slate-500">{ticket.note}{ticket.cached ? " · cached" : ""}</p>
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
