import { useState } from "react";
import TicketAnalysis from "./TicketAnalysis";
import { kellyStake, legKey, shortTeam, type Ticket, type TicketLeg } from "../types";

// A cherry-picked slip: legs the user pulled from across different generated
// tickets. Recomputes combined prob/odds, suggests a Kelly stake, and places it
// under the "custom" strategy so its conversion tracks separately in the Tracker.
export default function CustomSlip({
  legs,
  bankroll = 0,
  kellyFraction = 0,
  leagues,
  onRemove,
  onClear,
  onPlace,
}: {
  legs: TicketLeg[];
  bankroll?: number;
  kellyFraction?: number;
  leagues?: Record<number, string>;
  onRemove: (key: string) => void;
  onClear: () => void;
  onPlace: (t: Ticket, stake: number, odds: number | null) => Promise<void>;
}) {
  const [open, setOpen] = useState(true);
  const [placing, setPlacing] = useState(false);
  const [placed, setPlaced] = useState(false);
  const [stake, setStake] = useState("");
  const [odds, setOdds] = useState("");

  const allEst = legs.length > 0 && legs.every((l) => l.est_prob != null);
  const combinedProb = allEst ? legs.reduce((a, l) => a * (l.est_prob as number), 1) : null;
  const allPriced = legs.length > 0 && legs.every((l) => l.book_odds != null);
  const combinedOdds = allPriced ? legs.reduce((a, l) => a * (l.book_odds as number), 1) : null;
  const recStake = kellyStake(legs, combinedOdds, bankroll, kellyFraction);

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
    setPlacing(false);
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
                  <div className="font-medium truncate">
                    {l.selection}
                    {l.team && (
                      <span className="ml-1.5 text-[9px] font-semibold text-slate-400 bg-edge rounded px-1 py-0.5 align-middle">
                        {shortTeam(l.team)}
                      </span>
                    )}
                  </div>
                  <div className="text-slate-500 truncate">
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

          <div className="flex items-center justify-between text-[11px] text-slate-400">
            <span>{kind === "Custom" ? "cross-game parlay" : kind}</span>
            <button className="underline hover:text-slate-200" onClick={onClear}>
              clear all
            </button>
          </div>

          <TicketAnalysis key={legs.map(legKey).join("|")} ticket={ticket()} leagues={leagues} />

          {placing ? (
            <div className="space-y-1.5">
              <div className="flex items-center gap-2">
                <input
                  className="w-20 rounded-lg bg-ink border border-edge px-2 py-2 text-sm"
                  placeholder="stake $"
                  inputMode="decimal"
                  value={stake}
                  onChange={(e) => setStake(e.target.value)}
                />
                <input
                  className="w-20 rounded-lg bg-ink border border-edge px-2 py-2 text-sm"
                  placeholder="odds"
                  inputMode="decimal"
                  value={odds || (combinedOdds != null ? combinedOdds.toFixed(2) : "")}
                  onChange={(e) => setOdds(e.target.value)}
                />
                <button className="btn btn-primary text-sm px-3 py-2" onClick={confirm}>
                  Place
                </button>
                <button className="text-xs text-slate-400 underline" onClick={() => setPlacing(false)}>
                  cancel
                </button>
              </div>
              {recStake > 0 && (
                <button className="text-[11px] text-accent underline" onClick={() => setStake(String(recStake))}>
                  use Kelly stake ${recStake.toFixed(2)}
                </button>
              )}
            </div>
          ) : (
            <button className="btn btn-primary w-full text-sm" onClick={() => setPlacing(true)}>
              Place custom slip
            </button>
          )}
        </>
      )}
    </div>
  );
}
