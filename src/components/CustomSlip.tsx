import { useEffect, useState } from "react";
import TicketAnalysis from "./TicketAnalysis";
import StakeBumps from "./StakeBumps";
import { api } from "../api";
import { kellyStake, legKey, shortTeam, type SgpPrice, type Ticket, type TicketLeg } from "../types";

// A cherry-picked slip: legs the user pulled from across different generated
// tickets. Recomputes combined prob/odds, suggests a Kelly stake, and places it
// under the "custom" strategy so its conversion tracks separately in the Tracker.
export default function CustomSlip({
  legs,
  bankroll = 0,
  kellyFraction = 0,
  defaultStake = 0,
  leagues,
  onRemove,
  onClear,
  onPlace,
}: {
  legs: TicketLeg[];
  bankroll?: number;
  kellyFraction?: number;
  defaultStake?: number;
  leagues?: Record<number, string>;
  onRemove: (key: string) => void;
  onClear: () => void;
  onPlace: (t: Ticket, stake: number, odds: number | null) => Promise<void>;
}) {
  const [open, setOpen] = useState(true);
  const [placed, setPlaced] = useState(false);
  const [stake, setStake] = useState("");
  const [odds, setOdds] = useState("");

  const allEst = legs.length > 0 && legs.every((l) => l.est_prob != null);
  const combinedProb = allEst ? legs.reduce((a, l) => a * (l.est_prob as number), 1) : null;

  // Correlation-aware joint probability (Monte Carlo) — only meaningful for 2+
  // legs that share a game, where the naive product understates the true chance.
  const [sgp, setSgp] = useState<SgpPrice | null>(null);
  const sameGame = new Set(legs.map((l) => l.match)).size === 1;
  useEffect(() => {
    setSgp(null);
    if (legs.length < 2 || !allEst) return;
    let live = true;
    api.priceSgp(legs).then((p) => { if (live) setSgp(p); }).catch(() => {});
    return () => { live = false; };
  }, [legs.map(legKey).join("|"), allEst]);

  const allPriced = legs.length > 0 && legs.every((l) => l.book_odds != null);
  const combinedOdds = allPriced ? legs.reduce((a, l) => a * (l.book_odds as number), 1) : null;
  const recStake = kellyStake(legs, combinedOdds, bankroll, kellyFraction);

  // Prefill stake (Kelly) and odds inline so placing is one tap, still editable.
  // MUST be before any early return — all hooks run unconditionally every render.
  useEffect(() => {
    const prefill = defaultStake > 0 ? defaultStake : recStake; // flat default wins
    setStake((s) => (s === "" && prefill > 0 ? prefill.toFixed(2) : s));
    setOdds((o) => (o === "" && combinedOdds != null ? combinedOdds.toFixed(2) : o));
  }, [recStake, combinedOdds, defaultStake]);

  if (legs.length === 0) return null;

  const kind = legs.length <= 1 ? "Single" : new Set(legs.map((l) => l.match)).size <= 1 ? "SGP" : "Custom";

  function ticket(): Ticket {
    const title = legs.map((l) => `${shortTeam(l.team) || l.selection}`).slice(0, 4).join(" + ");
    return {
      type: "Custom",
      title: title || "Cherry-picked slip",
      confidence: "",
      legs,
      combined_prob: combinedProb,
      combined_odds: combinedOdds,
      combined_ev: null,
      flags: ["cherry-picked"],
      why: null,
    };
  }

  async function confirm() {
    const s = parseFloat(stake);
    if (!Number.isFinite(s) || s <= 0 || legs.length === 0) return;
    const o = parseFloat(odds);
    await onPlace(ticket(), s, Number.isFinite(o) && o > 0 ? o : null);
    setPlaced(true);
    onClear();
    setStake("");
    setOdds("");
    setTimeout(() => setPlaced(false), 2500);
  }

  return (
    <div className="card border-accent/50 space-y-2">
      <button className="w-full flex items-center justify-between gap-2 text-left" onClick={() => setOpen(!open)}>
        <div className="flex items-center gap-2">
          <span className="badge bg-accent text-ink">🍒 Custom slip</span>
          <span className="text-xs text-slate-400">
            {legs.length} leg{legs.length > 1 ? "s" : ""}
            {combinedOdds != null ? ` · @ ${combinedOdds.toFixed(2)}` : ""}
            {combinedProb != null ? ` · ${Math.round(combinedProb * 100)}%` : ""}
          </span>
        </div>
        <span className="text-xs text-slate-500">{open ? "▴" : "▾"}</span>
      </button>

      {placed && <div className="text-xs text-accent">Placed — tracking under “Cherry-picked”.</div>}

      {open && (
        <>
          <div className="space-y-1">
            {legs.map((l) => (
              <div key={legKey(l)} className="flex items-center justify-between gap-2 text-xs rounded-lg bg-ink border border-edge px-2.5 py-1.5">
                <div className="min-w-0">
                  <div className="font-medium break-words">
                    {l.selection}
                    {l.team && (
                      <span className="ml-1.5 text-[9px] font-semibold text-slate-400 bg-edge rounded px-1 py-0.5 align-middle">
                        {shortTeam(l.team)}
                      </span>
                    )}
                  </div>
                  <div className="text-slate-500 break-words">
                    {l.match} · {l.market}
                    {l.line ? ` ${l.line}` : ""}
                    {l.book_odds != null ? ` · @ ${l.book_odds.toFixed(2)}` : " · no price"}
                  </div>
                </div>
                <button className="text-slate-500 hover:text-bad shrink-0" onClick={() => onRemove(legKey(l))} title="Remove from slip">
                  ✕
                </button>
              </div>
            ))}
          </div>

          {sgp && (
            <div className="rounded-lg bg-ink border border-edge px-2.5 py-2 text-[11px] space-y-0.5">
              <div className="flex items-center justify-between">
                <span className="text-slate-400">
                  {sameGame ? "Correlated price" : "Joint price"} <span className="text-slate-500">(Monte Carlo)</span>
                </span>
                <b className="text-slate-100">
                  {Math.round(sgp.correlated * 100)}% · fair @ {sgp.fair_odds.toFixed(2)}
                </b>
              </div>
              <div className="flex items-center justify-between text-slate-500">
                <span>vs naive {Math.round(sgp.independent * 100)}% (independent)</span>
                <span className={sgp.lift > 1.03 ? "text-accent" : sgp.lift < 0.97 ? "text-bad" : ""}>
                  {sgp.lift >= 1 ? "+" : ""}{Math.round((sgp.lift - 1) * 100)}% correlation
                </span>
              </div>
            </div>
          )}

          <div className="flex items-center justify-between text-[11px] text-slate-400">
            <span>{kind === "Custom" ? "cross-game parlay" : kind}</span>
            <button className="underline hover:text-slate-200" onClick={onClear}>
              clear all
            </button>
          </div>

          <TicketAnalysis key={legs.map(legKey).join("|")} ticket={ticket()} leagues={leagues} />

          <div className="space-y-1">
            <div className="flex items-center gap-1.5">
              <span className="text-sm text-slate-500">$</span>
              <input
                className="w-16 rounded-lg bg-ink border border-edge px-2 py-2 text-sm"
                placeholder="stake"
                inputMode="decimal"
                value={stake}
                onChange={(e) => setStake(e.target.value)}
              />
              <span className="text-xs text-slate-500">@</span>
              <input
                className="w-16 rounded-lg bg-ink border border-edge px-2 py-2 text-sm"
                placeholder="odds"
                inputMode="decimal"
                value={odds}
                onChange={(e) => setOdds(e.target.value)}
              />
              <button className="btn btn-primary text-sm px-3 py-2 flex-1" onClick={confirm}>
                Place{stake ? ` $${stake}` : " slip"}
              </button>
            </div>
            <div className="flex items-center justify-between">
              <StakeBumps value={stake} onChange={setStake} />
              {recStake > 0 && (
                <div className="text-[10px] text-slate-500">
                  Kelly ${recStake.toFixed(2)}
                  {recStake.toFixed(2) !== stake && (
                    <button className="ml-1 text-accent underline" onClick={() => setStake(recStake.toFixed(2))}>use</button>
                  )}
                </div>
              )}
            </div>
          </div>
        </>
      )}
    </div>
  );
}
