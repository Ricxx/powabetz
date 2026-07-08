import { useEffect, useMemo, useState } from "react";
import { api } from "../api";
import Spinner from "./Spinner";
import { toast } from "../toast";
import type { PlEvent, PlPick } from "../types";

// US sports (MLB / NBA) — SIMPLE evidence flow, organized for big slates:
// games grouped by DATE (tap a date to select/deselect the day), results
// grouped by FIXTURE (collapsible), probability + text filters, per-row void.
const SPORT_MARKETS: Record<string, { key: string; label: string }[]> = {
  baseball_mlb: [
    { key: "h2h", label: "Moneyline" },
    { key: "totals", label: "Totals (runs)" },
    { key: "spreads", label: "Run Line" },
    { key: "pitcher_strikeouts", label: "Pitcher Ks" },
    { key: "batter_total_bases", label: "Total Bases" },
    { key: "batter_hits", label: "Hits" },
    { key: "batter_home_runs", label: "Home Runs" },
    { key: "batter_rbis", label: "RBIs" },
  ],
  basketball_nba: [
    { key: "h2h", label: "Moneyline" },
    { key: "totals", label: "Totals (points)" },
    { key: "spreads", label: "Spread" },
    { key: "player_points", label: "Points" },
    { key: "player_rebounds", label: "Rebounds" },
    { key: "player_assists", label: "Assists" },
    { key: "player_threes", label: "Threes" },
    { key: "player_points_rebounds_assists", label: "PRA" },
    { key: "player_steals", label: "Steals" },
    { key: "player_blocks", label: "Blocks" },
    { key: "player_double_double", label: "Double-Double" },
  ],
};

const pct = (p?: number | null) => (p == null ? "—" : `${Math.round(p * 100)}%`);
const PROB_STEPS = [0, 0.4, 0.5, 0.6, 0.7] as const;

export default function PropLine({ sport }: { sport: string }) {
  const [events, setEvents] = useState<PlEvent[] | null>(null);
  const [selEvents, setSelEvents] = useState<Set<string>>(new Set());
  const [selMarkets, setSelMarkets] = useState<Set<string>>(new Set(["h2h", "totals"]));
  const [picks, setPicks] = useState<PlPick[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [filter, setFilter] = useState("");
  const [minProb, setMinProb] = useState<number>(0);
  const [voided, setVoided] = useState<Set<string>>(new Set());
  const [openFx, setOpenFx] = useState<Set<string>>(new Set());
  const [copied, setCopied] = useState(false);
  const [usage, setUsage] = useState<number | null>(null);

  useEffect(() => {
    setEvents(null);
    setPicks(null);
    setSelEvents(new Set());
    api.plEvents(sport).then(setEvents).catch((e) => {
      setEvents([]);
      toast.error(e);
    });
    api.plUsage().then(setUsage).catch(() => {});
  }, [sport]);

  const rowKey = (p: PlPick) => `${p.fixture}|${p.market}|${p.subject}|${p.side}`;

  // Games grouped by LOCAL date, chronological within each day.
  const byDate = useMemo(() => {
    const m = new Map<string, PlEvent[]>();
    for (const e of events ?? []) {
      const d = e.commence_time
        ? new Date(e.commence_time).toLocaleDateString([], { weekday: "short", month: "short", day: "numeric" })
        : "TBD";
      m.set(d, [...(m.get(d) ?? []), e]);
    }
    return m;
  }, [events]);

  const visible = useMemo(() => {
    if (!picks) return [];
    const q = filter.trim().toLowerCase();
    return picks.filter((p) => {
      if (minProb > 0 && (p.probability ?? 0) < minProb) return false;
      if (q && !`${p.fixture} ${p.market} ${p.subject} ${p.side}`.toLowerCase().includes(q)) return false;
      return true;
    });
  }, [picks, filter, minProb]);

  // Results grouped by fixture; rows sorted market → probability desc.
  const byFixture = useMemo(() => {
    const m = new Map<string, PlPick[]>();
    for (const p of visible) m.set(p.fixture, [...(m.get(p.fixture) ?? []), p]);
    for (const rows of m.values()) {
      rows.sort((a, b) => a.market.localeCompare(b.market) || (b.probability ?? 0) - (a.probability ?? 0));
    }
    return m;
  }, [visible]);

  const copyCount = visible.filter((p) => !voided.has(rowKey(p))).length;

  // Honest evaluation estimate: ~1 odds call per game (throttled ~0.4s) plus
  // the first run of the day syncing new completed games (~15 more).
  const estimate = useMemo(() => {
    const n = selEvents.size;
    if (n === 0) return "";
    const lo = Math.ceil(n * 0.5);
    const hi = Math.ceil(n * 0.5 + 12);
    return `≈ ${n}–${n + 15} API calls · ~${lo}s (up to ~${hi}s on the first run of the day)`;
  }, [selEvents]);

  async function evaluate() {
    if (selEvents.size === 0) return toast.error("Select at least one game.");
    if (selMarkets.size === 0) return toast.error("Select at least one market.");
    setBusy(true);
    setPicks(null);
    setVoided(new Set());
    setOpenFx(new Set());
    try {
      setPicks(await api.plPicks(sport, [...selEvents], [...selMarkets]));
      api.plUsage().then(setUsage).catch(() => {});
    } catch (e) {
      toast.error(e);
    } finally {
      setBusy(false);
    }
  }

  function copyOut() {
    const rows = visible.filter((p) => !voided.has(rowKey(p)));
    if (rows.length === 0) return toast.error("Nothing to copy (all filtered/voided).");
    const lines = rows.map((p) =>
      `Fixture: ${p.fixture}, Prop/market: ${p.market}${p.subject && p.subject !== p.side ? ` (${p.subject})` : ""}, over/under: ${p.side}, odds: ${p.odds ?? "—"}${p.book ? ` (${p.book})` : ""}, sharp: ${pct(p.sharp)}, probability: ${pct(p.probability)}, implied prob: ${pct(p.implied)}, hit chance: ${pct(p.hit_chance)}`
    );
    navigator.clipboard.writeText(lines.join("\n")).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
      toast.success(`Copied ${lines.length} pick(s)${voided.size ? ` (${voided.size} voided skipped)` : ""}.`);
    });
  }

  const markets = SPORT_MARKETS[sport] ?? [];

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <p className="text-xs text-slate-400">
          {sport === "basketball_nba" ? "NBA" : "MLB"} — evidence list, not tickets. Void ✕ removes a
          row from the copy.
        </p>
        {usage != null && (
          <span className="text-[10px] text-slate-500 shrink-0 ml-2" title="Fresh PropLine API calls today (cached reads are free)">
            📡 {usage} calls today
          </span>
        )}
      </div>

      {/* games — grouped by date, tap the date to toggle the whole day */}
      {events === null ? (
        <div className="text-xs text-slate-400 flex items-center gap-2"><Spinner /> Loading games…</div>
      ) : events.length === 0 ? (
        <div className="card text-xs text-slate-500">
          No upcoming games. {sport === "basketball_nba" ? "NBA is off-season until October — check back at preseason." : "Check back later."}
        </div>
      ) : (
        <div className="card space-y-2">
          <div className="flex items-center justify-between">
            <div className="text-xs font-semibold text-slate-300">Games ({selEvents.size}/{events.length})</div>
            <button
              className="text-[11px] text-accent underline"
              onClick={() => setSelEvents(selEvents.size === events.length ? new Set() : new Set(events.map((e) => e.id)))}
            >
              {selEvents.size === events.length ? "clear all" : "select all"}
            </button>
          </div>
          <div className="space-y-2 max-h-72 overflow-y-auto">
            {[...byDate.entries()].map(([date, evs]) => {
              const allOn = evs.every((e) => selEvents.has(e.id));
              const someOn = evs.some((e) => selEvents.has(e.id));
              return (
                <div key={date}>
                  <button
                    className={`w-full text-left text-[11px] font-semibold rounded-md px-2 py-1 mb-1 ${
                      allOn ? "bg-accent/20 text-accent" : someOn ? "bg-edge text-slate-200" : "bg-ink text-slate-400"
                    }`}
                    title="Tap to select/deselect every game on this date"
                    onClick={() =>
                      setSelEvents((prev) => {
                        const n = new Set(prev);
                        if (allOn) evs.forEach((e) => n.delete(e.id));
                        else evs.forEach((e) => n.add(e.id));
                        return n;
                      })
                    }
                  >
                    📅 {date} · {evs.length} game{evs.length === 1 ? "" : "s"} {allOn ? "✓" : someOn ? "◐" : ""}
                  </button>
                  <div className="grid grid-cols-1 gap-1">
                    {evs.map((e) => {
                      const on = selEvents.has(e.id);
                      return (
                        <button
                          key={e.id}
                          className={`text-left text-xs rounded-lg border px-2.5 py-1.5 ${on ? "border-accent bg-accent/10" : "border-edge bg-ink"}`}
                          onClick={() =>
                            setSelEvents((prev) => {
                              const n = new Set(prev);
                              n.has(e.id) ? n.delete(e.id) : n.add(e.id);
                              return n;
                            })
                          }
                        >
                          {e.away_team} @ {e.home_team}
                          <span className="text-slate-500 ml-1.5">
                            {e.live ? "· LIVE" : e.commence_time ? `· ${new Date(e.commence_time).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}` : ""}
                          </span>
                        </button>
                      );
                    })}
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      )}

      {/* markets */}
      <div className="card space-y-1.5">
        <div className="text-xs font-semibold text-slate-300">Markets</div>
        <div className="flex flex-wrap gap-1.5">
          {markets.map((m) => {
            const on = selMarkets.has(m.key);
            return (
              <button
                key={m.key}
                className={`chip text-xs ${on ? "chip-on" : ""}`}
                onClick={() =>
                  setSelMarkets((prev) => {
                    const n = new Set(prev);
                    n.has(m.key) ? n.delete(m.key) : n.add(m.key);
                    return n;
                  })
                }
              >
                {m.label}
              </button>
            );
          })}
        </div>
      </div>

      <div>
        <button className="btn btn-primary w-full" disabled={busy || selEvents.size === 0} onClick={evaluate}>
          {busy ? <span className="inline-flex items-center gap-2"><Spinner /> Evaluating…</span> : `📋 Evaluate ${selEvents.size} game${selEvents.size === 1 ? "" : "s"}`}
        </button>
        {estimate && !busy && <p className="text-[10px] text-slate-500 mt-1 text-center">{estimate}</p>}
      </div>

      {/* results — fixture-grouped, filterable */}
      {picks && (
        <div className="card space-y-2">
          <div className="flex items-center gap-2">
            <input
              className="input flex-1 text-xs"
              placeholder="Filter — player / market / game…"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
            />
            <button className="btn btn-ghost text-xs py-2 shrink-0" onClick={copyOut}>
              {copied ? "✓ copied" : `⧉ Copy ${copyCount}`}
            </button>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="text-[10px] text-slate-500 shrink-0">min prob</span>
            {PROB_STEPS.map((p) => (
              <button
                key={p}
                className={`chip flex-1 text-center text-xs ${minProb === p ? "chip-on" : ""}`}
                onClick={() => setMinProb(p)}
              >
                {p === 0 ? "All" : `≥${Math.round(p * 100)}%`}
              </button>
            ))}
          </div>
          <div className="text-[10px] text-slate-500">
            {visible.length.toLocaleString()} of {picks.length.toLocaleString()} rows
            {voided.size > 0 ? ` · ${voided.size} voided` : ""} — tap a game to expand; rows sorted market → probability.
          </div>
          <div className="space-y-1.5 max-h-[62vh] overflow-y-auto">
            {[...byFixture.entries()].map(([fx, rows]) => {
              const open = openFx.has(fx);
              return (
                <div key={fx} className="rounded-lg border border-edge">
                  <button
                    className="w-full flex items-center justify-between px-2.5 py-2 text-left"
                    onClick={() =>
                      setOpenFx((prev) => {
                        const n = new Set(prev);
                        n.has(fx) ? n.delete(fx) : n.add(fx);
                        return n;
                      })
                    }
                  >
                    <span className="text-xs font-semibold truncate">{fx}</span>
                    <span className="text-[10px] text-slate-500 shrink-0 ml-2">{rows.length} rows {open ? "▴" : "▾"}</span>
                  </button>
                  {open && (
                    <div className="px-1.5 pb-1.5 space-y-1">
                      {rows.map((p) => {
                        const k = rowKey(p);
                        const isVoid = voided.has(k);
                        return (
                          <div
                            key={k}
                            className={`flex items-center justify-between gap-2 text-xs rounded-lg bg-ink px-2.5 py-1.5 ${isVoid ? "opacity-40" : ""}`}
                          >
                            <div className="min-w-0">
                              <div className="font-medium break-words">
                                {p.subject && p.subject !== p.side ? `${p.subject} · ` : ""}{p.side}
                                <span className="text-slate-500 ml-1.5">{p.market}</span>
                              </div>
                              <div className="text-slate-500 break-words">
                                {p.odds != null ? `@${p.odds.toFixed(2)}${p.book ? ` (${p.book})` : ""}` : "unpriced"}
                                {` · sharp ${pct(p.sharp)} · prob ${pct(p.probability)} · implied ${pct(p.implied)} · hit ${pct(p.hit_chance)}`}
                              </div>
                            </div>
                            <button
                              className={`shrink-0 ${isVoid ? "text-accent" : "text-slate-500 hover:text-bad"}`}
                              title={isVoid ? "Restore into the copy" : "Void — keep visible, exclude from copy"}
                              onClick={() =>
                                setVoided((prev) => {
                                  const n = new Set(prev);
                                  n.has(k) ? n.delete(k) : n.add(k);
                                  return n;
                                })
                              }
                            >
                              {isVoid ? "↩" : "✕"}
                            </button>
                          </div>
                        );
                      })}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
