import { useState } from "react";
import TicketAnalysis from "./TicketAnalysis";
import { kellyStake, legKey, shortTeam } from "../types";
import type { BuildResult, BuildUsage, Ticket, TicketLeg } from "../types";

function modelLabel(id: string): string {
  if (id.includes("opus")) return "Opus 4.8";
  if (id.includes("sonnet")) return "Sonnet 4.6";
  if (id.includes("haiku")) return "Haiku 4.5";
  return id;
}

function confidenceColor(c: string): string {
  const v = c.toLowerCase();
  if (v.includes("very")) return "bg-accent text-ink";
  if (v === "high") return "bg-accent/30 text-accent";
  if (v === "medium") return "bg-warn/25 text-warn";
  return "bg-edge text-slate-300";
}

function pct(p?: number | null): string {
  return p == null ? "—" : `${Math.round(p * 100)}%`;
}

function GrokBanner({ digest }: { digest?: string | null }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="card bg-ink border-accent/40">
      <button className="w-full flex items-center justify-between" onClick={() => setOpen(!open)}>
        <span className="text-xs font-semibold text-accent">🔍 Grok X/news digest used</span>
        {digest && <span className="text-slate-500 text-xs">{open ? "hide" : "show"}</span>}
      </button>
      {open && digest && (
        <pre className="text-[11px] text-slate-300 whitespace-pre-wrap mt-2 leading-snug font-sans">
          {digest}
        </pre>
      )}
    </div>
  );
}

function typeBadge(t: string): string {
  if (t === "SGP+") return "bg-accent text-ink";
  if (t === "SGP") return "bg-accent/30 text-accent";
  return "bg-edge text-slate-300";
}

function TicketCard({
  t,
  onPlace,
  bankroll = 0,
  kellyFraction = 0,
  leagues,
  onVoidSubject,
  cartKeys,
  onToggleCartLeg,
}: {
  t: Ticket;
  onPlace?: (t: Ticket, stake: number, odds: number | null) => Promise<void>;
  bankroll?: number;
  kellyFraction?: number;
  leagues?: Record<number, string>;
  onVoidSubject?: (subject: string, voided: boolean) => void;
  cartKeys?: Set<string>;
  onToggleCartLeg?: (l: TicketLeg) => void;
}) {
  const [open, setOpen] = useState(true);
  const [placing, setPlacing] = useState(false);
  const [placed, setPlaced] = useState(false);
  const [removed, setRemoved] = useState<Set<number>>(new Set());
  const [stake, setStake] = useState("");

  // Active (non-voided) legs and the recomputed combined numbers.
  const active = t.legs.filter((_, i) => !removed.has(i));
  const allEst = active.length > 0 && active.every((l) => l.est_prob != null);
  const combinedProb = allEst ? active.reduce((a, l) => a * (l.est_prob as number), 1) : null;
  const allPriced = active.length > 0 && active.every((l) => l.book_odds != null);
  const combinedOdds = allPriced ? active.reduce((a, l) => a * (l.book_odds as number), 1) : null;
  const kind =
    active.length <= 1 ? "Single" : new Set(active.map((l) => l.match)).size <= 1 ? "SGP" : "SGP+";
  const modified: Ticket = {
    ...t,
    legs: active,
    type: kind,
    combined_prob: combinedProb,
    combined_odds: combinedOdds,
  };
  const recStake = kellyStake(active, combinedOdds, bankroll, kellyFraction);
  const [odds, setOdds] = useState(t.combined_odds != null ? String(t.combined_odds) : "");
  const isLong = active.length > 1 && combinedProb != null && combinedProb >= 0.1 && combinedProb <= 0.2;

  async function confirm() {
    const s = parseFloat(stake);
    if (!Number.isFinite(s) || s <= 0 || active.length === 0) return;
    const o = parseFloat(odds);
    await onPlace?.(modified, s, Number.isFinite(o) && o > 0 ? o : null);
    setPlaced(true);
    setPlacing(false);
  }

  return (
    <div className={`card space-y-2 ${placed ? "border-accent/50" : ""}`}>
      <button
        className="w-full flex items-start justify-between gap-2 text-left"
        onClick={() => setOpen(!open)}
      >
        <div className="min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <span className={`badge ${typeBadge(kind)}`}>{kind}</span>
            <span className="text-xs text-slate-400">
              {active.length} leg{active.length > 1 ? "s" : ""}
              {removed.size > 0 && <span className="text-warn"> · {removed.size} voided</span>}
            </span>
            {isLong && <span className="badge bg-warn/25 text-warn">longshot</span>}
            {placed && <span className="text-accent text-xs">✓ placed</span>}
          </div>
          <div className="font-semibold leading-tight mt-1">{t.title}</div>
          {!open && (
            <div className="text-xs text-slate-400 mt-0.5">
              {combinedOdds != null ? `@${combinedOdds.toFixed(2)} · ` : ""}
              {active.length > 1 ? "hit" : "prob"} {pct(combinedProb)}
            </div>
          )}
        </div>
        <div className="flex items-center gap-2 shrink-0">
          <span className={`badge ${confidenceColor(t.confidence)}`}>{t.confidence}</span>
          <span className="text-slate-500 text-xs">{open ? "▴" : "▾"}</span>
        </div>
      </button>

      {open && (
        <>
      <div className="space-y-1.5">
        {t.legs.map((l, i) => {
          const ev = l.ev != null ? l.ev : null;
          const sharp = l.ev_source === "sharp";
          const isVoid = removed.has(i);
          const toggleVoid = () => {
            const willVoid = !removed.has(i);
            setRemoved((prev) => {
              const n = new Set(prev);
              n.has(i) ? n.delete(i) : n.add(i);
              return n;
            });
            // Report up so a voided player stays out of "add more" ladder tickets.
            onVoidSubject?.(l.selection, willVoid);
          };
          return (
            <div
              key={i}
              className={`rounded-lg bg-ink border border-edge px-2.5 py-1.5 ${isVoid ? "opacity-45" : ""}`}
            >
              <div className="flex items-center justify-between gap-2">
                <div className="min-w-0">
                  <div className={`text-sm font-medium truncate ${isVoid ? "line-through" : ""}`}>
                    {l.selection}
                    {l.team && (
                      <span className="ml-1.5 text-[9px] font-semibold text-slate-400 bg-edge rounded px-1 py-0.5 align-middle">
                        {shortTeam(l.team)}
                      </span>
                    )}
                  </div>
                  <div className="text-[11px] text-slate-400 truncate">
                    {l.market}
                    {l.line ? ` · ${l.line}` : ""} — {l.match}
                  </div>
                </div>
                <div className="text-right shrink-0">
                  {l.book_odds != null ? (
                    <>
                      <div className="text-sm font-mono">{l.book_odds.toFixed(2)}</div>
                      {l.book && <div className="text-[9px] text-slate-500 -mt-0.5">{l.book}</div>}
                    </>
                  ) : (
                    <div className="text-[10px] text-slate-500">unpriced</div>
                  )}
                  {ev != null && (
                    <div
                      className={`text-[11px] font-semibold ${ev > 0 ? "text-accent" : "text-slate-500"}`}
                      title={sharp ? "EV vs Pinnacle true price" : "EV vs our model probability"}
                    >
                      {ev > 0 ? "+" : ""}
                      {(ev * 100).toFixed(1)}% EV{sharp ? "" : "*"}
                    </div>
                  )}
                </div>
              </div>
              <div className="flex items-center justify-between mt-0.5">
                <div className="flex gap-3 text-[10px] text-slate-500">
                  <span>model {pct(l.est_prob)}</span>
                  {l.pinnacle_prob != null && <span>pinnacle {pct(l.pinnacle_prob)}</span>}
                </div>
                <div className="flex items-center gap-3">
                  {onToggleCartLeg && (
                    <button
                      className={`text-[10px] ${cartKeys?.has(legKey(l)) ? "text-accent" : "text-slate-500 hover:text-accent"}`}
                      onClick={() => onToggleCartLeg(l)}
                      title={cartKeys?.has(legKey(l)) ? "Remove from custom slip" : "Add this leg to a custom slip"}
                    >
                      {cartKeys?.has(legKey(l)) ? "🍒 in slip" : "🍒 pick"}
                    </button>
                  )}
                  <button
                    className={`text-[10px] ${isVoid ? "text-accent" : "text-slate-500 hover:text-bad"}`}
                    onClick={toggleVoid}
                    title={isVoid ? "Put this leg back" : "Void this leg (e.g. player not in the lineup)"}
                  >
                    {isVoid ? "↩ restore" : "✕ void"}
                  </button>
                </div>
              </div>
            </div>
          );
        })}
      </div>

      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs pt-0.5">
        {combinedOdds != null && (
          <span className="text-slate-300">
            Odds <b className="text-slate-100">{combinedOdds.toFixed(2)}</b>
          </span>
        )}
        <span className="text-slate-300">
          {active.length > 1 ? "Hit chance" : "Model prob"}{" "}
          <b className="text-slate-100">{pct(combinedProb)}</b>
        </span>
        {removed.size > 0 && <span className="text-[11px] text-warn">recomputed after voiding</span>}
      </div>

      {t.flags?.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {t.flags.map((f, i) => (
            <span key={i} className="badge bg-edge text-slate-300 normal-case">
              {f}
            </span>
          ))}
        </div>
      )}

      {t.why && <p className="text-sm text-slate-300 leading-snug">{t.why}</p>}
        </>
      )}

      <TicketAnalysis ticket={modified} leagues={leagues} />

      {onPlace &&
        (placed ? null : placing ? (
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
                value={odds}
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
              <button
                className="text-[11px] text-accent underline"
                onClick={() => setStake(String(recStake))}
              >
                Kelly suggests ${recStake.toFixed(2)} — use it
              </button>
            )}
          </div>
        ) : active.length === 0 ? (
          <div className="text-xs text-slate-500 text-center">All legs voided.</div>
        ) : (
          <button
            className="btn btn-ghost text-sm w-full"
            onClick={() => {
              if (recStake > 0) setStake(String(recStake));
              if (combinedOdds != null) setOdds(combinedOdds.toFixed(2));
              setPlacing(true);
            }}
          >
            ＋ Place {removed.size > 0 ? "voided " : ""}ticket{recStake > 0 ? ` · Kelly $${recStake.toFixed(2)}` : ""}
          </button>
        ))}
    </div>
  );
}

export default function Results({
  result,
  usage,
  onSave,
  onCopy,
  onNewSet,
  onExport,
  onPlace,
  saved,
  busy,
  bankroll = 0,
  kellyFraction = 0,
  leagues,
  onVoidSubject,
  cartKeys,
  onToggleCartLeg,
}: {
  result: BuildResult;
  usage?: BuildUsage | null;
  onSave?: () => void;
  onCopy?: () => void;
  onNewSet?: () => void;
  onExport?: () => void;
  onPlace?: (t: Ticket, stake: number, odds: number | null) => Promise<void>;
  saved?: boolean;
  busy?: boolean;
  bankroll?: number;
  kellyFraction?: number;
  leagues?: Record<number, string>;
  onVoidSubject?: (subject: string, voided: boolean) => void;
  cartKeys?: Set<string>;
  onToggleCartLeg?: (l: import("../types").TicketLeg) => void;
}) {
  return (
    <div className="space-y-3">
      {result.from_cache && (
        <div className="text-xs text-accent">Re-used a cached result — 0 tokens, 0 requests.</div>
      )}

      {usage && !usage.from_cache && (
        <div className="text-xs text-slate-400">
          {modelLabel(usage.model)} · {usage.input_tokens.toLocaleString()} in /{" "}
          {usage.output_tokens.toLocaleString()} out tokens ·{" "}
          <span className="text-slate-200">~${usage.cost_usd.toFixed(4)}</span>
        </div>
      )}

      {result.grok_used && <GrokBanner digest={result.grok_digest} />}

      {result.tickets.length === 0 && (
        <div className="card text-sm text-slate-300">
          The model returned no picks for this selection.
        </div>
      )}

      {result.tickets.map((t, i) => (
        <TicketCard
          key={i}
          t={t}
          onPlace={onPlace}
          bankroll={bankroll}
          kellyFraction={kellyFraction}
          leagues={leagues}
          onVoidSubject={onVoidSubject}
          cartKeys={cartKeys}
          onToggleCartLeg={onToggleCartLeg}
        />
      ))}

      {result.context_notes && result.context_notes.length > 0 && (
        <details className="card bg-ink">
          <summary className="text-xs font-semibold text-slate-400 cursor-pointer">
            Match context (weather, standings, H2H, referee)
          </summary>
          <ul className="text-xs text-slate-400 list-disc pl-4 space-y-1 mt-2">
            {result.context_notes.map((n, i) => (
              <li key={i}>{n}</li>
            ))}
          </ul>
        </details>
      )}

      {result.data_quality_notes.length > 0 && (
        <div className="card bg-ink">
          <div className="text-xs font-semibold text-slate-400 mb-1">Data quality</div>
          <ul className="text-xs text-slate-400 list-disc pl-4 space-y-1">
            {result.data_quality_notes.map((n, i) => (
              <li key={i}>{n}</li>
            ))}
          </ul>
        </div>
      )}

      {(onSave || onCopy || onNewSet || onExport) && (
        <div className="space-y-2">
          {onNewSet && (
            <button
              className="btn btn-primary w-full text-sm flex items-center justify-center gap-2"
              onClick={onNewSet}
              disabled={busy}
            >
              {busy && (
                <span className="inline-block w-4 h-4 border-2 border-ink/40 border-t-ink rounded-full animate-spin" />
              )}
              {busy ? "Generating…" : "♻ Generate a new set"}
            </button>
          )}
          <div className="grid grid-cols-3 gap-2">
            {onSave && (
              <button className="btn btn-ghost text-sm" onClick={onSave} disabled={saved}>
                {saved ? "Saved" : "Save"}
              </button>
            )}
            {onCopy && (
              <button className="btn btn-ghost text-sm" onClick={onCopy}>
                Copy
              </button>
            )}
            {onExport && (
              <button className="btn btn-ghost text-sm" onClick={onExport}>
                CSV
              </button>
            )}
          </div>
        </div>
      )}

      <p className="text-[10px] text-slate-500 leading-snug pt-1">
        EV* = vs our model probability (no sharp Pinnacle line for that leg); plain EV = vs
        Pinnacle. "unpriced" legs aren't in the odds feed (most player props) — model likelihood
        only.
      </p>
      <p className="text-[10px] text-slate-500 leading-snug">
        Research and price context only — not financial advice, no claimed market edge. Stacking
        short-priced legs compounds the bookmaker margin across the ticket regardless of pick
        quality.
      </p>
    </div>
  );
}
