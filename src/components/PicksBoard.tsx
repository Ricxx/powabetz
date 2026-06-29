import { useEffect, useMemo, useState } from "react";
import { errMsg, toast } from "../toast";
import { api } from "../api";
import Hint from "./Hint";
import StakeBumps from "./StakeBumps";
import { kellyStake, shortTeam } from "../types";
import type { Candidate, FixtureInput, Ticket, TicketEval } from "../types";

function pct(p?: number | null): string {
  return p == null ? "—" : `${Math.round(p * 100)}%`;
}
function conf(p: number): string {
  if (p > 0.78) return "Very High";
  if (p > 0.6) return "High";
  if (p > 0.45) return "Medium";
  return "Low";
}
function verdictColor(v: string): string {
  const s = v.toLowerCase();
  if (s === "strong") return "bg-accent text-ink";
  if (s === "fair") return "bg-warn/25 text-warn";
  if (s === "thin") return "bg-bad/30 text-bad";
  return "bg-edge text-slate-300";
}

interface Built {
  ticket: Ticket;
  evalSel: boolean;
  result: TicketEval | null;
}

const EVAL_MODELS = [
  { id: "claude-sonnet-4-6", label: "Sonnet" },
  { id: "claude-haiku-4-5", label: "Haiku" },
  { id: "claude-opus-4-8", label: "Opus" },
];

export default function PicksBoard({
  fixtures,
  markets,
  bankroll = 0,
  kellyFraction = 0,
  defaultStake = 0,
  mode = "all",
  onClose,
  onPlaced,
}: {
  fixtures: FixtureInput[];
  markets: string[];
  bankroll?: number;
  kellyFraction?: number;
  defaultStake?: number;
  mode?: "all" | "bankers";
  onClose: () => void;
  onPlaced: () => void;
}) {
  const bankers = mode === "bankers";
  const [cands, setCands] = useState<Candidate[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [sel, setSel] = useState<Set<number>>(new Set());
  const [search, setSearch] = useState("");
  const [sortBy, setSortBy] = useState<"ev" | "prob">(bankers ? "prob" : "ev");
  const [valueOnly, setValueOnly] = useState(false);

  const [built, setBuilt] = useState<Built[]>([]);
  const [evalModel, setEvalModel] = useState("claude-sonnet-4-6");
  const [evaluating, setEvaluating] = useState(false);

  useEffect(() => {
    const load = bankers ? api.getBankers(fixtures, markets) : api.getPicks(fixtures, markets);
    load.then(setCands).catch((e) => setErr(errMsg(e)));
  }, []);

  const shown = useMemo(() => {
    if (!cands) return [];
    const q = search.trim().toLowerCase();
    const rows = cands
      .map((c, i) => ({ c, i }))
      .filter(({ c }) => !valueOnly || (c.ev != null && c.ev > 0))
      .filter(
        ({ c }) =>
          q === "" ||
          c.subject.toLowerCase().includes(q) ||
          c.market.toLowerCase().includes(q) ||
          c.fixture.toLowerCase().includes(q)
      );
    // Bankers arrive pre-ranked by the banker score — keep that order.
    if (!bankers) {
      rows.sort((a, b) =>
        sortBy === "ev" ? (b.c.ev ?? -9) - (a.c.ev ?? -9) : b.c.est_prob - a.c.est_prob
      );
    }
    return rows.slice(0, 120);
  }, [cands, search, sortBy, valueOnly, bankers]);

  const slip = useMemo(() => (cands ? [...sel].map((i) => cands[i]).filter(Boolean) : []), [cands, sel]);
  const slipProb = slip.reduce((a, c) => a * c.est_prob, 1);
  const slipPriced = slip.length > 0 && slip.every((c) => c.book_odds != null);
  const slipOdds = slipPriced ? slip.reduce((a, c) => a * (c.book_odds as number), 1) : null;

  function toggle(i: number) {
    setSel((p) => {
      const n = new Set(p);
      n.has(i) ? n.delete(i) : n.add(i);
      return n;
    });
  }

  function addTicket() {
    if (slip.length === 0) return;
    const fixturesSet = new Set(slip.map((c) => c.fixture));
    const type = slip.length <= 1 ? "Single" : fixturesSet.size <= 1 ? "SGP" : "SGP+";
    const ticket: Ticket = {
      type,
      title: slip.map((c) => c.subject).slice(0, 3).join(" + ") + (slip.length > 3 ? "…" : ""),
      confidence: conf(slipProb),
      legs: slip.map((c) => ({
        match: c.fixture,
        fixture_id: c.fixture_id,
        market: c.market,
        selection: c.subject,
        line: c.line,
        est_prob: c.est_prob,
        pinnacle_prob: c.pinnacle_prob,
        book_odds: c.book_odds,
        book: c.book,
        ev: c.ev,
        ev_source: c.ev_source,
      })),
      combined_prob: slipProb,
      combined_odds: slipOdds,
      combined_ev: null,
      flags: ["custom"],
      why: null,
    };
    setBuilt((b) => [...b, { ticket, evalSel: true, result: null }]);
    setSel(new Set());
  }

  async function evaluateSelected() {
    const chosen = built.filter((b) => b.evalSel);
    if (chosen.length === 0) return;
    setEvaluating(true);
    setErr(null);
    try {
      const evals = await api.evaluateTickets(chosen.map((b) => b.ticket), evalModel);
      let k = 0;
      setBuilt((prev) =>
        prev.map((b) => (b.evalSel ? { ...b, result: evals[k++] ?? null } : b))
      );
    } catch (e) {
      setErr(errMsg(e));
    } finally {
      setEvaluating(false);
    }
  }

  return (
    <div className="space-y-3 pb-40">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-bold">{bankers ? "🏦 Bankers board" : "Picks board"}</h2>
        <button className="btn btn-ghost text-sm py-2" onClick={onClose}>
          Done
        </button>
      </div>
      <p className="text-[11px] text-slate-500">
        {bankers
          ? "The safest, most repeatable legs across your slate — ranked by likelihood, recurring events and recent form, must-play only. Anchor an accumulator with these, then evaluate."
          : 'Tap picks → "Add as ticket" to build several. Select tickets and Evaluate to get model analysis + risks. +EV uses best book price vs Pinnacle.'}
      </p>

      {/* built tickets */}
      {built.length > 0 && (
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            <span className="text-xs font-semibold text-slate-400">Your tickets ({built.length})</span>
            <div className="flex gap-1 ml-auto">
              {EVAL_MODELS.map((m) => (
                <button
                  key={m.id}
                  className={`chip text-xs ${evalModel === m.id ? "chip-on" : ""}`}
                  onClick={() => setEvalModel(m.id)}
                >
                  {m.label}
                </button>
              ))}
            </div>
          </div>
          {built.map((b, bi) => (
            <div key={bi} className="card space-y-1.5">
              <div className="flex items-start justify-between gap-2">
                <label className="flex items-start gap-2 min-w-0">
                  <input
                    type="checkbox"
                    checked={b.evalSel}
                    onChange={() =>
                      setBuilt((prev) => prev.map((x, j) => (j === bi ? { ...x, evalSel: !x.evalSel } : x)))
                    }
                    className="mt-1"
                  />
                  <span className="min-w-0">
                    <span className="text-sm font-semibold">{b.ticket.title}</span>
                    <span className="block text-[11px] text-slate-400">
                      {b.ticket.type} · {b.ticket.legs.length} legs ·{" "}
                      {b.ticket.combined_odds != null ? `@${b.ticket.combined_odds.toFixed(2)} · ` : ""}
                      hit {pct(b.ticket.combined_prob)}
                    </span>
                  </span>
                </label>
                {b.result?.verdict && (
                  <span className={`badge ${verdictColor(b.result.verdict)}`}>{b.result.verdict}</span>
                )}
              </div>
              {b.result && (
                <div className="text-xs text-slate-300 space-y-1 border-t border-edge pt-1.5">
                  {b.result.analysis && <p>{b.result.analysis}</p>}
                  {b.result.risks?.length > 0 && (
                    <ul className="list-disc pl-4 text-slate-400">
                      {b.result.risks.map((r, ri) => (
                        <li key={ri}>{r}</li>
                      ))}
                    </ul>
                  )}
                </div>
              )}
              <PlaceRow
                ticket={b.ticket}
                recStake={kellyStake(b.ticket.legs, b.ticket.combined_odds, bankroll, kellyFraction)}
                defaultStake={defaultStake}
                onPlaced={onPlaced}
                onRemove={() => setBuilt((p) => p.filter((_, j) => j !== bi))}
              />
            </div>
          ))}
          <button
            className="btn btn-primary w-full text-sm flex items-center justify-center gap-2"
            onClick={evaluateSelected}
            disabled={evaluating || built.every((b) => !b.evalSel)}
          >
            {evaluating && <span className="inline-block w-4 h-4 border-2 border-ink/40 border-t-ink rounded-full animate-spin" />}
            {evaluating ? "Evaluating…" : `Evaluate selected (${EVAL_MODELS.find((m) => m.id === evalModel)?.label})`}
          </button>
        </div>
      )}

      {/* board */}
      <input
        className="w-full rounded-lg bg-ink border border-edge px-3 py-2 text-sm"
        placeholder="Search player / market / match…"
        value={search}
        onChange={(e) => setSearch(e.target.value)}
      />
      <div className="flex gap-2 text-xs">
        <button className={`chip ${sortBy === "ev" ? "chip-on" : ""}`} onClick={() => setSortBy("ev")}>
          Sort: EV
        </button>
        <button className={`chip ${sortBy === "prob" ? "chip-on" : ""}`} onClick={() => setSortBy("prob")}>
          Sort: likelihood
        </button>
        <button className={`chip ${valueOnly ? "chip-on" : ""}`} onClick={() => setValueOnly((v) => !v)}>
          +EV only
        </button>
        <Hint text="EV (expected value) is your edge per $1 staked: book odds × true probability − 1. Positive means the price beats the fair chance. We judge it against the sharp Pinnacle de-vigged probability where available, else our model." />
      </div>

      {err && <div className="text-xs text-bad">{err}</div>}
      {!cands && !err && <div className="text-sm text-slate-400">Loading picks…</div>}

      <div className="space-y-1.5">
        {shown.map(({ c, i }) => {
          const on = sel.has(i);
          const ev = c.ev;
          return (
            <button
              key={i}
              className={`w-full text-left rounded-lg border px-2.5 py-1.5 ${on ? "border-accent bg-accent/10" : "border-edge bg-ink"}`}
              onClick={() => toggle(i)}
            >
              <div className="flex items-center justify-between gap-2">
                <div className="min-w-0">
                  <div className="text-sm font-medium truncate">
                    {c.subject}
                    {c.team && c.team !== "Match" && c.team !== "Both Teams" && (
                      <span className="ml-1.5 text-[9px] font-semibold text-slate-400 bg-edge rounded px-1 py-0.5 align-middle">
                        {shortTeam(c.team)}
                      </span>
                    )}{" "}
                    <span className="text-slate-400 font-normal">{c.line}</span>
                  </div>
                  <div className="text-[11px] text-slate-400 truncate">
                    {c.market} — {c.fixture}
                    {(() => {
                      const s = c.flags?.find((f) => f.startsWith("style:"));
                      return s ? (
                        <span className="ml-1.5 text-[9px] text-warn bg-warn/15 rounded px-1 py-0.5">
                          {s.replace("style:", "").trim()}
                        </span>
                      ) : null;
                    })()}
                  </div>
                </div>
                <div className="text-right shrink-0">
                  {c.book_odds != null ? (
                    <div className="text-sm font-mono">{c.book_odds.toFixed(2)}</div>
                  ) : (
                    <div className="text-[10px] text-slate-500">unpriced</div>
                  )}
                  <div className="text-[10px] text-slate-500">
                    {pct(c.est_prob)}
                    {ev != null && (
                      <span className={ev > 0 ? "text-accent ml-1" : "text-slate-500 ml-1"}>
                        {ev > 0 ? "+" : ""}
                        {(ev * 100).toFixed(0)}%
                      </span>
                    )}
                  </div>
                </div>
              </div>
            </button>
          );
        })}
      </div>

      {/* slip */}
      {slip.length > 0 && (
        <div className="fixed bottom-0 left-0 right-0 max-w-md mx-auto p-3 bg-panel border-t border-edge space-y-2">
          <div className="flex items-center justify-between text-xs">
            <span className="font-semibold">Slip · {slip.length} leg{slip.length > 1 ? "s" : ""}</span>
            <span className="text-slate-300">
              {slipOdds != null && <>@{slipOdds.toFixed(2)} · </>}
              hit {pct(slipProb)}
            </span>
          </div>
          <div className="flex gap-2">
            <button className="btn btn-primary text-sm flex-1" onClick={addTicket}>
              ＋ Add as ticket
            </button>
            <button className="btn btn-ghost text-sm px-3" onClick={() => setSel(new Set())}>
              clear
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

function PlaceRow({
  ticket,
  recStake = 0,
  defaultStake = 0,
  onPlaced,
  onRemove,
}: {
  ticket: Ticket;
  recStake?: number;
  defaultStake?: number;
  onPlaced: () => void;
  onRemove: () => void;
}) {
  const [stake, setStake] = useState((recStake > 0 ? recStake : defaultStake) > 0 ? (recStake > 0 ? recStake : defaultStake).toFixed(2) : "");
  const [odds, setOdds] = useState(ticket.combined_odds != null ? String(ticket.combined_odds) : "");
  const [placed, setPlaced] = useState(false);

  async function place() {
    const s = parseFloat(stake);
    if (!Number.isFinite(s) || s <= 0) {
      toast.error("Enter a stake greater than 0.");
      return;
    }
    const o = parseFloat(odds);
    try {
      await api.placeBet(ticket, s, Number.isFinite(o) && o > 0 ? o : null, false, "board");
    } catch (e) {
      toast.error(e);
      return;
    }
    toast.success(`Bet placed — $${s.toFixed(2)}`);
    setPlaced(true);
    onPlaced();
  }

  return (
    <div className="space-y-1 pt-1">
      <div className="flex items-center gap-2">
        <input
          className="w-16 rounded-lg bg-ink border border-edge px-2 py-1.5 text-xs"
          placeholder="stake"
          inputMode="decimal"
          value={stake}
          onChange={(e) => setStake(e.target.value)}
        />
        <input
          className="w-16 rounded-lg bg-ink border border-edge px-2 py-1.5 text-xs"
          placeholder="odds"
          inputMode="decimal"
          value={odds}
          onChange={(e) => setOdds(e.target.value)}
        />
        <button className="btn btn-primary text-xs px-3 py-1.5" onClick={place} disabled={placed}>
          {placed ? "Placed ✓" : `Place${stake ? ` $${stake}` : ""}`}
        </button>
        <button className="text-xs text-slate-500 underline ml-auto" onClick={onRemove}>
          remove
        </button>
      </div>
      <div className="flex items-center gap-2">
        <StakeBumps value={stake} onChange={setStake} />
        {recStake > 0 && (
          <button className="text-[10px] text-accent underline" onClick={() => setStake(recStake.toFixed(2))}>
            Kelly ${recStake.toFixed(2)}
          </button>
        )}
      </div>
    </div>
  );
}
