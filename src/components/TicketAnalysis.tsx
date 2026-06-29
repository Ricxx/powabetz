import { useState } from "react";
import { api } from "../api";
import Spinner from "./Spinner";
import Hint from "./Hint";
import { ANALYSIS_MODELS, type Ticket, type TicketEval } from "../types";

function verdictColor(v: string): string {
  if (v === "Strong") return "bg-accent text-ink";
  if (v === "Thin") return "bg-bad/30 text-bad";
  return "bg-warn/25 text-warn"; // Fair
}

function ratingStyle(r: string): { mark: string; color: string } {
  switch (r) {
    case "solid":
      return { mark: "✓", color: "text-accent" };
    case "risky":
      return { mark: "⚠", color: "text-warn" };
    case "trap":
      return { mark: "✗", color: "text-bad" };
    default:
      return { mark: "•", color: "text-slate-400" }; // ok / unknown
  }
}

// Shared Haiku quick-analysis: an expandable button that scores the given ticket
// and renders the result as clear structured data (verdict, per-leg ratings,
// risks, recommended changes). Used by generated tickets AND the custom slip.
export default function TicketAnalysis({
  ticket,
  leagues,
}: {
  ticket: Ticket;
  leagues?: Record<number, string>;
}) {
  const [res, setRes] = useState<TicketEval | null>(null);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [model, setModel] = useState("claude-haiku-4-5");
  const [usedModel, setUsedModel] = useState<string | null>(null);

  async function runWith(m: string) {
    setModel(m);
    setBusy(true);
    setRes(null);
    try {
      const r = await api.evaluateTickets([ticket], m, leagues);
      setRes(r[0] ?? { analysis: "(no analysis)", leg_notes: [], risks: [], recommendations: [], verdict: "" });
      setUsedModel(m);
    } catch (e) {
      setRes({ analysis: `Analysis failed: ${e}`, leg_notes: [], risks: [], recommendations: [], verdict: "" });
    } finally {
      setBusy(false);
    }
  }

  function analyze() {
    setOpen((v) => !v);
    if (!res && !busy) runWith(model);
  }

  return (
    <div className="pt-1">
      <button
        className="text-[11px] text-slate-400 hover:text-slate-100 flex items-center gap-1"
        onClick={analyze}
      >
        🤖 Quick analysis {open ? "▴" : "▾"}
      </button>
      {open && (
        <div className="mt-1.5 rounded-lg bg-ink border border-edge px-2.5 py-2 text-xs space-y-2">
          <div className="flex items-center gap-1.5 flex-wrap">
            <span className="text-[10px] text-slate-500">model:</span>
            <Hint text="Spends one cached model call (free to re-open at the same ticket). Cost order: GPT-5 nano cheapest, then Haiku, then GPT-5 mini (~15× nano). GPT models need an OpenAI key." />
            {ANALYSIS_MODELS.map((m) => (
              <button
                key={m.id}
                className={`chip text-[10px] py-0.5 ${model === m.id ? "chip-on" : ""}`}
                disabled={busy}
                onClick={() => runWith(m.id)}
                title={m.provider === "openai" ? "needs an OpenAI key" : ""}
              >
                {m.label}
              </button>
            ))}
            {usedModel && (
              <span className="text-[10px] text-slate-500">
                · read by {ANALYSIS_MODELS.find((x) => x.id === usedModel)?.label || usedModel}
              </span>
            )}
          </div>
          {busy && (
            <div className="text-slate-400 inline-flex items-center gap-2">
              <Spinner /> Analysing…
            </div>
          )}
          {res && !busy && (
            <>
              <div className="flex items-center gap-2">
                {res.verdict && <span className={`badge ${verdictColor(res.verdict)}`}>{res.verdict}</span>}
                {res.analysis && <p className="text-slate-300 flex-1">{res.analysis}</p>}
              </div>

              {res.leg_notes?.length > 0 && (
                <div className="space-y-0.5">
                  <div className="text-[10px] font-semibold uppercase tracking-wide text-slate-500">Per-leg read</div>
                  {res.leg_notes.map((ln, i) => {
                    const s = ratingStyle(ln.rating);
                    return (
                      <div key={i} className="flex items-start gap-1.5">
                        <span className={`${s.color} font-bold`}>{s.mark}</span>
                        <span className="text-slate-300 min-w-0">
                          <span className="font-medium">{ln.leg}</span>
                          {ln.rating ? <span className={`ml-1 ${s.color}`}>· {ln.rating}</span> : null}
                          {ln.note ? <span className="text-slate-400"> — {ln.note}</span> : null}
                        </span>
                      </div>
                    );
                  })}
                </div>
              )}

              {res.risks?.length > 0 && (
                <div>
                  <div className="text-[10px] font-semibold uppercase tracking-wide text-slate-500">Risks</div>
                  <ul className="list-disc pl-4 text-slate-400">
                    {res.risks.map((r, i) => (
                      <li key={i}>{r}</li>
                    ))}
                  </ul>
                </div>
              )}

              {res.recommendations?.length > 0 && (
                <div>
                  <div className="text-[10px] font-semibold uppercase tracking-wide text-accent">Recommended changes</div>
                  <ul className="list-disc pl-4 text-slate-300">
                    {res.recommendations.map((r, i) => (
                      <li key={i}>{r}</li>
                    ))}
                  </ul>
                </div>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}
