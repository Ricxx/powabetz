import { useEffect, useState } from "react";
import { errMsg } from "../toast";
import { api } from "../api";
import type { GenReportRow, MarketReportRow, ModelPurposeRow } from "../types";

function purposeLabel(p: string): string {
  const m: Record<string, string> = {
    build: "Build tickets",
    eval: "Quick analysis",
    plausibility: "Plausibility",
    ingest: "Ingest (web pages)",
    tactics: "Tactics/coach",
  };
  return m[p] || p;
}
function modelShort(m: string): string {
  if (m.includes("opus")) return "Opus";
  if (m.includes("sonnet")) return "Sonnet";
  if (m.includes("haiku")) return "Haiku";
  if (m === "gpt-5-nano") return "GPT-5 nano";
  if (m === "gpt-5-mini") return "GPT-5 mini";
  if (m.startsWith("gpt-")) return m;
  return m;
}
function providerOf(m: string): string {
  if (m.startsWith("gpt-")) return "OpenAI";
  if (m.includes("claude")) return "Claude";
  return "Other";
}

function ModelPurposeTable({ rows }: { rows: ModelPurposeRow[] }) {
  if (rows.length === 0) return null;
  const grand = rows.reduce((a, r) => a + r.cost_usd, 0);
  // Group by provider for a presentable breakdown with subtotals.
  const byProvider = new Map<string, ModelPurposeRow[]>();
  for (const r of rows) {
    const p = providerOf(r.model);
    if (!byProvider.has(p)) byProvider.set(p, []);
    byProvider.get(p)!.push(r);
  }
  const providers = [...byProvider.entries()].sort(
    (a, b) => b[1].reduce((s, r) => s + r.cost_usd, 0) - a[1].reduce((s, r) => s + r.cost_usd, 0)
  );
  return (
    <div className="card space-y-2">
      <div className="flex items-baseline justify-between">
        <div className="text-xs font-semibold text-slate-400">AI spend — by provider, model &amp; purpose</div>
        <div className="text-sm font-bold text-accent">${grand.toFixed(4)}</div>
      </div>
      {providers.map(([prov, prs]) => {
        const sub = prs.reduce((a, r) => a + r.cost_usd, 0);
        return (
          <div key={prov}>
            <div className="flex items-baseline justify-between text-[11px] mb-0.5">
              <span className="font-semibold text-slate-300">{prov}</span>
              <span className="text-slate-400">${sub.toFixed(4)}</span>
            </div>
            <table className="w-full text-xs">
              <tbody>
                {prs
                  .slice()
                  .sort((a, b) => b.cost_usd - a.cost_usd)
                  .map((r, i) => (
                    <tr key={i} className="border-t border-edge">
                      <td className="py-1 text-slate-300">
                        {modelShort(r.model)} <span className="text-slate-500">· {purposeLabel(r.purpose)}</span>
                      </td>
                      <td className="text-right text-slate-500 w-20">
                        {((r.input_tokens + r.output_tokens) / 1000).toFixed(0)}k
                      </td>
                      <td className="text-right text-slate-200 w-16">${r.cost_usd.toFixed(4)}</td>
                    </tr>
                  ))}
              </tbody>
            </table>
          </div>
        );
      })}
    </div>
  );
}

function stratLabel(s: string): string {
  if (s === "likely") return "Secret picks";
  if (s === "favorites") return "Form faves";
  if (s === "oracle") return "Oracle ✦";
  if (s === "power") return "Power Stacker ⚡";
  if (s === "bankers") return "Anchors ⚓";
  if (s === "custom") return "Cherry-picked 🍒";
  if (s === "ladder") return "Acca ladder";
  if (s === "board") return "Board";
  return "Value +EV";
}

function ReportTable({
  rows,
  label,
  labelFn = stratLabel,
}: {
  rows: GenReportRow[];
  label: string;
  labelFn?: (s: string) => string;
}) {
  const sorted = rows.slice().sort((a, b) => (b.roi ?? -99) - (a.roi ?? -99) || b.hit_rate - a.hit_rate);
  if (sorted.length === 0) return null;
  return (
    <div className="card">
      <div className="text-xs font-semibold text-slate-400 mb-1">{label}</div>
      <table className="w-full text-xs">
        <thead className="text-slate-500">
          <tr>
            <td className="pb-1">{label.includes("type") ? "Type" : "Strategy"}</td>
            <td className="text-right pb-1">tickets</td>
            <td className="text-right pb-1">hit</td>
            <td className="text-right pb-1">ROI</td>
          </tr>
        </thead>
        <tbody>
          {sorted.map((r, i) => (
            <tr key={i} className="border-t border-edge">
              <td className="py-1">
                {labelFn(r.strategy)}
                {r.grok_used && <span className="text-accent ml-1">🔍</span>}
              </td>
              <td className="text-right text-slate-400">
                {r.settled}/{r.total}
              </td>
              <td className="text-right">
                {r.settled > 0 ? (
                  <b className={r.hit_rate >= 0.5 ? "text-accent" : "text-slate-200"}>
                    {Math.round(r.hit_rate * 100)}%
                  </b>
                ) : (
                  <span className="text-slate-500">—</span>
                )}
              </td>
              <td className="text-right">
                {r.roi == null ? (
                  <span className="text-slate-500">—</span>
                ) : (
                  <b className={r.roi >= 0 ? "text-accent" : "text-bad"}>
                    {r.roi >= 0 ? "+" : ""}
                    {Math.round(r.roi * 100)}%
                  </b>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

// Per-pick (market) calibration: actual hit-rate vs the model's prediction. The
// gap is the bias — and it's exactly what auto-feeds the model's calibration.
function MarketTable({ rows }: { rows: MarketReportRow[] }) {
  const sorted = rows.slice().filter((r) => r.settled > 0).sort((a, b) => b.settled - a.settled);
  if (sorted.length === 0) return null;
  return (
    <div className="card">
      <div className="text-xs font-semibold text-slate-400 mb-1">By pick (market) — predicted vs actual</div>
      <table className="w-full text-xs">
        <thead className="text-slate-500">
          <tr>
            <td className="pb-1">Market</td>
            <td className="text-right pb-1">legs</td>
            <td className="text-right pb-1">pred</td>
            <td className="text-right pb-1">actual</td>
            <td className="text-right pb-1">bias</td>
          </tr>
        </thead>
        <tbody>
          {sorted.map((r, i) => {
            const bias = r.hit_rate - r.predicted; // +ve = model under-rates this market
            const strong = Math.abs(bias) >= 0.08 && r.settled >= 10;
            return (
              <tr key={i} className="border-t border-edge">
                <td className="py-1">{r.market}</td>
                <td className="text-right text-slate-400">
                  {r.won}/{r.settled}
                </td>
                <td className="text-right text-slate-400">{Math.round(r.predicted * 100)}%</td>
                <td className="text-right">
                  <b className={r.hit_rate >= r.predicted ? "text-accent" : "text-slate-200"}>
                    {Math.round(r.hit_rate * 100)}%
                  </b>
                </td>
                <td className="text-right">
                  <span className={strong ? (bias >= 0 ? "text-accent" : "text-bad") : "text-slate-500"}>
                    {bias >= 0 ? "+" : ""}
                    {Math.round(bias * 100)}%
                  </span>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <p className="text-[10px] text-slate-500 mt-1">
        bias = actual − predicted. <span className="text-accent">+</span> = model under-rates that
        market (your picks land more than it expects); <span className="text-bad">−</span> = over-rates
        (a trap-prone market). Needs ~10+ settled legs to trust.
      </p>
    </div>
  );
}

export default function Ledger({ onClose }: { onClose: () => void }) {
  const [rows, setRows] = useState<GenReportRow[] | null>(null);
  const [byKind, setByKind] = useState<GenReportRow[]>([]);
  const [byMarket, setByMarket] = useState<MarketReportRow[]>([]);
  const [byPurpose, setByPurpose] = useState<ModelPurposeRow[]>([]);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  function loadAgg() {
    api.generatedReportByKind().then(setByKind).catch(() => {});
    api.generatedReportByMarket().then(setByMarket).catch(() => {});
    api.usageByPurpose().then(setByPurpose).catch(() => {});
  }
  function load() {
    api.generatedReport().then(setRows).catch((e) => setErr(errMsg(e)));
    loadAgg();
  }
  useEffect(load, []);

  async function settleAll() {
    setBusy(true);
    setErr(null);
    try {
      setRows(await api.settleGenerated());
      loadAgg();
    } catch (e) {
      setErr(errMsg(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-bold">Strategy ledger</h2>
        <button className="btn btn-ghost text-sm py-2" onClick={onClose}>
          Done
        </button>
      </div>
      <p className="text-[11px] text-slate-500">
        Every unique ticket the tool generates is logged by strategy. Settle them all against
        results to see — across far more tickets than you'd ever place — which approach actually
        hits, and where the value really is.
      </p>

      <button
        className="btn btn-primary w-full text-sm flex items-center justify-center gap-2"
        onClick={settleAll}
        disabled={busy}
      >
        {busy && <span className="inline-block w-4 h-4 border-2 border-ink/40 border-t-ink rounded-full animate-spin" />}
        {busy ? "Settling…" : "Settle all generated tickets"}
      </button>

      {err && <div className="text-xs text-bad">{err}</div>}

      <ModelPurposeTable rows={byPurpose} />

      {!rows && !err && <div className="text-sm text-slate-400">Loading…</div>}
      {rows && rows.length === 0 && (
        <div className="card text-sm text-slate-400">
          No generated tickets logged yet — build a slate first, then come back.
        </div>
      )}

      {rows && rows.length > 0 && (
        <>
          <ReportTable rows={rows} label="By strategy" />
          <ReportTable rows={byKind} label="By ticket type" labelFn={(s) => s} />
          <MarketTable rows={byMarket} />
          <p className="text-[10px] text-slate-500">
            Hit = all legs landed. ROI = notional return at 1 unit/ticket on priced tickets only
            (unpriced player-prop accas show hit-rate but no ROI).
          </p>
          <div className="card bg-accent/5 border-accent/30">
            <div className="text-xs font-semibold text-accent mb-1">↩ How this feeds the tool</div>
            <p className="text-[11px] text-slate-400">
              Every settled leg here (not just placed bets) now trains the model's{" "}
              <b className="text-slate-200">calibration</b>: if your picks consistently beat or miss
              their predicted rate, future builds shrink/stretch probabilities to match reality (see
              the λ in the Tracker). The <b className="text-slate-200">By pick</b> table above shows
              exactly which markets are biased, so you can lean into the ones that over-deliver and
              fade the trap-prone ones.
            </p>
          </div>
        </>
      )}
    </div>
  );
}
