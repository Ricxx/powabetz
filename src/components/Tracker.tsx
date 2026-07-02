import { useEffect, useState } from "react";
import { errMsg, toast } from "../toast";
import { api } from "../api";
import Spinner from "./Spinner";
import Hint from "./Hint";
import { stratLabel } from "../types";
import type { BankrollView, CalibrationReport, PlacedBet } from "../types";

function statusBadge(s: string): string {
  if (s === "won") return "bg-accent text-ink";
  if (s === "lost") return "bg-bad/30 text-bad";
  if (s === "partial") return "bg-warn/25 text-warn";
  if (s === "void") return "bg-edge text-slate-400"; // pushed — stake refunded
  return "bg-edge text-slate-300";
}

function money(n: number): string {
  return `${n < 0 ? "-" : ""}$${Math.abs(n).toFixed(2)}`;
}

function BetCard({ bet, onSettle, onDelete, onUpdated }: { bet: PlacedBet; onSettle: () => void; onDelete: () => void; onUpdated?: (b: PlacedBet) => void }) {
  const t = bet.ticket;
  const pnl = bet.settled ? bet.returns - bet.stake : null;
  const [oddsIn, setOddsIn] = useState("");
  // All legs graded green/void but no price recorded → the backend keeps it open
  // (never fabricates a break-even payout); prompt for the real odds here.
  const needsOdds =
    !bet.settled &&
    t.combined_odds == null &&
    bet.leg_results.length === t.legs.length &&
    bet.leg_results.length > 0 &&
    bet.leg_results.every((r) => r.won === true || r.void === true);
  async function addOdds() {
    const o = parseFloat(oddsIn);
    if (!Number.isFinite(o) || o <= 1) {
      toast.error("Enter the ticket's decimal odds (e.g. 4.50).");
      return;
    }
    try {
      const updated = await api.setBetOdds(bet.id, o);
      onUpdated?.(updated);
      toast.success("Odds set — bet settled.");
    } catch (e) {
      toast.error(e);
    }
  }
  return (
    <div className="card space-y-2">
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="text-sm font-semibold truncate">{t.title || t.type}</div>
          <div className="text-xs text-slate-400">
            {t.type} · {t.legs.length} legs · stake ${bet.stake.toFixed(2)}
            {t.combined_odds != null ? ` @ ${t.combined_odds.toFixed(2)}` : ""}
          </div>
        </div>
        <div className="flex items-center gap-1 shrink-0">
          <span className="badge bg-edge text-slate-300 normal-case" title="Strategy">
            {stratLabel(bet.strategy)}
          </span>
          {bet.grok_used && (
            <span className="badge bg-accent/20 text-accent" title="Grok sentiment used">
              🔍
            </span>
          )}
          {bet.clv != null && (
            <span
              className={`badge ${bet.clv >= 0 ? "bg-accent/20 text-accent" : "bg-bad/20 text-bad"}`}
              title="Closing-line value: your price vs the closing price. Consistently positive = real edge."
            >
              CLV {bet.clv >= 0 ? "+" : ""}{(bet.clv * 100).toFixed(1)}%
            </span>
          )}
          <span className={`badge ${statusBadge(bet.status)}`}>{bet.status}</span>
        </div>
      </div>

      {needsOdds && (
        <div className="flex items-center gap-2 text-xs bg-accent/10 rounded p-2">
          <span className="text-accent">✓ All legs landed — add the ticket odds to settle:</span>
          <input
            className="input w-20 text-xs"
            inputMode="decimal"
            placeholder="4.50"
            value={oddsIn}
            onChange={(e) => setOddsIn(e.target.value)}
          />
          <button className="btn btn-primary text-xs px-2 py-1" onClick={addOdds}>
            Settle
          </button>
        </div>
      )}

      <div className="space-y-1">
        {t.legs.map((l, i) => {
          const r = bet.leg_results[i];
          const won = r?.won;
          const isVoid = r?.void === true;
          const mark = isVoid ? "∅" : won === true ? "✓" : won === false ? "✗" : "•";
          const color = isVoid ? "text-slate-400" : won === true ? "text-accent" : won === false ? "text-bad" : "text-slate-500";
          return (
            <div key={i} className="flex items-center justify-between text-xs">
              <span className="break-words min-w-0">
                <span className={`${color} font-bold mr-1`}>{mark}</span>
                {l.selection} · {l.market}
                {l.line ? ` ${l.line}` : ""}
              </span>
              {r?.detail && (
                <span className="text-slate-500 shrink-0 ml-2">
                  {r.detail}
                  {won === false && r?.margin != null && r.margin > -1 && (
                    <span className="text-warn ml-1" title="Near-miss — lost by less than 1">· off by {Math.abs(r.margin).toFixed(1)}</span>
                  )}
                </span>
              )}
            </div>
          );
        })}
      </div>

      <div className="flex items-center justify-between text-xs pt-1">
        <div>
          {bet.settled ? (
            <span className={pnl != null && pnl >= 0 ? "text-accent" : "text-bad"}>
              {pnl != null ? money(pnl) : ""}
              {bet.status === "won" && bet.returns > 0 ? ` (returns $${bet.returns.toFixed(2)})` : ""}
            </span>
          ) : (
            <span className="text-slate-400">open</span>
          )}
        </div>
        <div className="flex gap-2">
          {!bet.settled && (
            <button className="underline text-slate-400" onClick={onSettle}>
              settle
            </button>
          )}
          <button className="underline text-slate-500" onClick={onDelete}>
            delete
          </button>
        </div>
      </div>
    </div>
  );
}

export default function Tracker({ onClose }: { onClose: () => void }) {
  const [bets, setBets] = useState<PlacedBet[]>([]);
  const [bank, setBank] = useState<BankrollView | null>(null);
  const [calib, setCalib] = useState<CalibrationReport | null>(null);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);

  function refresh() {
    api.listBets().then(setBets).catch((e) => setErr(errMsg(e)));
    api.getBankroll().then(setBank).catch(() => {});
    api.calibration().then(setCalib).catch(() => {});
  }
  // On open, auto-settle ended matches, then load.
  useEffect(() => {
    api
      .settleAll()
      .then((updated) => {
        setBets(updated);
        api.getBankroll().then(setBank).catch(() => {});
        api.calibration().then(setCalib).catch(() => {});
      })
      .catch(() => refresh())
      .finally(() => setLoading(false));
  }, []);

  async function checkWinners() {
    if (busy) return; // guard against request spam
    setBusy(true);
    setErr(null);
    const openBefore = bets.filter((b) => !b.settled).length;
    try {
      const updated = await api.settleAll();
      const openAfter = updated.filter((b) => !b.settled).length;
      const justSettled = Math.max(0, openBefore - openAfter);
      setBets(updated);
      setBank(await api.getBankroll());
      if (justSettled > 0) {
        toast.success(`Settled ${justSettled} bet${justSettled > 1 ? "s" : ""}${openAfter > 0 ? ` · ${openAfter} still awaiting results` : ""}`);
      } else if (openAfter > 0) {
        toast.info(`No results yet — ${openAfter} bet${openAfter > 1 ? "s" : ""} still pending. A match only settles once the data feed marks it finished with final stats, which can lag ~10-30 min after full-time.`);
      } else {
        toast.info("All bets already settled — nothing pending.");
      }
    } catch (e) {
      setErr(errMsg(e));
      toast.error(e);
    } finally {
      setBusy(false);
    }
  }

  async function settleOne(id: number) {
    try {
      await api.settleBet(id);
      refresh();
    } catch (e) {
      setErr(errMsg(e));
    }
  }
  function del(id: number) {
    // Optimistic remove + deferred delete, so it's undoable for 5s.
    setBets((prev) => prev.filter((b) => b.id !== id));
    const timer = setTimeout(() => {
      api.deleteBet(id).then(refresh).catch(toast.error);
    }, 5000);
    toast.undo("Bet removed", () => {
      clearTimeout(timer);
      refresh();
    });
  }

  // Grok attribution: win rate + ROI split by whether Grok was used.
  const split = (arr: PlacedBet[]) => {
    const s = arr.filter((b) => b.settled);
    const wins = s.filter((b) => b.status === "won").length;
    const staked = s.reduce((a, b) => a + b.stake, 0);
    const ret = s.reduce((a, b) => a + b.returns, 0);
    return { n: s.length, wins, roi: staked > 0 ? ((ret - staked) / staked) * 100 : 0 };
  };
  const withGrok = split(bets.filter((b) => b.grok_used));
  const noGrok = split(bets.filter((b) => !b.grok_used));

  // ROI split by the strategy each ticket came from.
  const strategies = ["apex", "value", "favorites", "likely", "oracle", "power", "bankers", "jackpot", "predictor", "scout", "live", "custom", "ladder", "board"].filter((s) =>
    bets.some((b) => b.strategy === s && b.settled)
  );
  // Closing-line value: the fastest-converging proof of edge (needs far fewer
  // bets than win/loss ROI to mean something).
  const clvBets = bets.filter((b) => b.clv != null);
  const avgClv = clvBets.length > 0 ? clvBets.reduce((a, b) => a + (b.clv as number), 0) / clvBets.length : null;

  // Group by day (descending).
  const groups = new Map<string, PlacedBet[]>();
  for (const b of bets) {
    const arr = groups.get(b.day) ?? [];
    arr.push(b);
    groups.set(b.day, arr);
  }
  const days = [...groups.keys()].sort().reverse();

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-bold">Bet tracker</h2>
        <button className="btn btn-ghost text-sm py-2" onClick={onClose}>
          Done
        </button>
      </div>

      {loading && (
        <div className="card text-sm text-slate-400 inline-flex items-center gap-2">
          <Spinner /> Loading bets &amp; settling ended matches…
        </div>
      )}

      {bank && (
        <div className="card space-y-1">
          <div className="flex items-baseline justify-between">
            <div className="text-xs text-slate-400">Balance</div>
            <div className="text-2xl font-bold">{money(bank.current)}</div>
          </div>
          <div className="flex flex-wrap gap-x-4 text-xs text-slate-400">
            <span>
              P&amp;L <b className={bank.pnl >= 0 ? "text-accent" : "text-bad"}>{money(bank.pnl)}</b>
            </span>
            <span>open stake ${bank.staked_open.toFixed(2)}</span>
            <span>{bank.open_count} open · {bank.settled_count} settled</span>
          </div>
          <div className="text-[11px] text-slate-500">Bankroll base set in Settings: ${bank.bankroll.toFixed(2)}</div>
        </div>
      )}

      {calib && calib.n > 0 && (
        <div className="card space-y-1.5">
          <div className="flex items-center justify-between">
            <div className="text-xs font-semibold text-slate-400">
              Model calibration
              <Hint text="λ measures how well our probabilities match reality, learned from your settled bets. λ<1 = overconfident, so new builds shrink edges toward 50/50; λ≈1 = well calibrated. It's weighted by how many legs have settled, so it eases in as evidence grows." />
            </div>
            <span className={`badge ${calib.applied ? "bg-accent/20 text-accent" : "bg-edge text-slate-300"}`}>
              λ {calib.lambda.toFixed(2)}
              {calib.applied ? " · live" : ""}
            </span>
          </div>
          <div className="text-[11px] text-slate-400">{calib.verdict}</div>
          {calib.bins.length > 0 && (
            <table className="w-full text-[11px] mt-1">
              <thead className="text-slate-500">
                <tr>
                  <td>predicted</td>
                  <td className="text-right">actual</td>
                  <td className="text-right">n</td>
                </tr>
              </thead>
              <tbody>
                {calib.bins.map((b, i) => {
                  const off = b.actual_rate - b.predicted_avg;
                  return (
                    <tr key={i}>
                      <td className="text-slate-300">{Math.round(b.predicted_avg * 100)}%</td>
                      <td className={`text-right ${Math.abs(off) > 0.1 ? "text-warn" : "text-slate-300"}`}>
                        {Math.round(b.actual_rate * 100)}%
                      </td>
                      <td className="text-right text-slate-500">{b.n}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </div>
      )}

      {strategies.length > 0 && (
        <div className="card space-y-1">
          <div className="text-xs font-semibold text-slate-400">By strategy (settled bets)</div>
          {strategies.map((s) => {
            const r = split(bets.filter((b) => b.strategy === s));
            return (
              <div key={s} className="flex justify-between text-xs">
                <span className="text-slate-300">{stratLabel(s)}</span>
                <span>
                  {r.wins}/{r.n} won · ROI{" "}
                  <b className={r.roi >= 0 ? "text-accent" : "text-bad"}>{r.roi.toFixed(0)}%</b>
                </span>
              </div>
            );
          })}
        </div>
      )}

      {avgClv != null && (
        <div className="card space-y-1">
          <div className="flex items-center gap-1 text-xs font-semibold text-slate-400">
            Closing-line value
            <Hint text="Your placed price vs the closing price, averaged across settled bets. Beating the close consistently is the professional standard of proof — it converges in dozens of bets where win/loss ROI needs hundreds. Positive = the market moved toward your bets after you placed them." />
          </div>
          <div className="flex justify-between text-xs">
            <span className="text-slate-400">{clvBets.length} bet{clvBets.length === 1 ? "" : "s"} measured</span>
            <b className={avgClv >= 0 ? "text-accent" : "text-bad"}>
              avg CLV {avgClv >= 0 ? "+" : ""}{(avgClv * 100).toFixed(1)}%
            </b>
          </div>
        </div>
      )}

      {(withGrok.n > 0 || noGrok.n > 0) && (
        <div className="card space-y-1">
          <div className="text-xs font-semibold text-slate-400">Does Grok help? (settled bets)</div>
          <div className="flex justify-between text-xs">
            <span className="text-accent">🔍 With Grok</span>
            <span>
              {withGrok.wins}/{withGrok.n} won · ROI{" "}
              <b className={withGrok.roi >= 0 ? "text-accent" : "text-bad"}>{withGrok.roi.toFixed(0)}%</b>
            </span>
          </div>
          <div className="flex justify-between text-xs">
            <span className="text-slate-400">Without</span>
            <span>
              {noGrok.wins}/{noGrok.n} won · ROI{" "}
              <b className={noGrok.roi >= 0 ? "text-accent" : "text-bad"}>{noGrok.roi.toFixed(0)}%</b>
            </span>
          </div>
        </div>
      )}

      <button className="btn btn-primary w-full" onClick={checkWinners} disabled={busy || bets.length === 0}>
        {busy ? (
          <span className="inline-flex items-center gap-2"><Spinner /> Checking results…</span>
        ) : (
          "Check winners"
        )}
      </button>
      {err && <div className="text-xs text-bad">{err}</div>}

      {bets.length === 0 && (
        <div className="card text-sm text-slate-400">
          No placed bets yet. On a results screen, tap “Place” on a ticket you backed.
        </div>
      )}

      {days.map((day) => (
        <div key={day} className="space-y-2">
          <div className="text-xs font-semibold text-slate-400">{day}</div>
          {groups.get(day)!.map((b) => (
            <BetCard
              key={b.id}
              bet={b}
              onSettle={() => settleOne(b.id)}
              onDelete={() => del(b.id)}
              onUpdated={(u) => setBets((prev) => prev.map((x) => (x.id === u.id ? u : x)))}
            />
          ))}
        </div>
      ))}
    </div>
  );
}
