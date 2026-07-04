import { useMemo, useState } from "react";
import { api } from "../api";
import Spinner from "./Spinner";
import { toast } from "../toast";
import { legKey, type Ticket, type TicketLeg } from "../types";

// 📌 My Picks — the user's personal shortlist, filled from cherry-picking.
// From here: deterministic N-fold combinations (pure math, 0 tokens), or one
// cheap cached AI pass that assembles the picks into a usable set of tickets.
export default function MyPicks({
  picks,
  selectedFixtureIds = [],
  onRemove,
  onClear,
  onPlace,
}: {
  picks: TicketLeg[];
  selectedFixtureIds?: number[];
  onRemove: (key: string) => void;
  onClear: () => void;
  onPlace: (t: Ticket, stake: number, odds: number | null) => Promise<void>;
}) {
  const [fold, setFold] = useState(4);
  const [combos, setCombos] = useState<Ticket[] | null>(null);
  const [aiTickets, setAiTickets] = useState<Ticket[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);

  // Scope the board to the CURRENT fixture selection: picks from other days /
  // deselected matches are hidden (kept, never deleted) so a big board stays
  // calm. No selection = show everything.
  const sel = useMemo(() => new Set(selectedFixtureIds), [selectedFixtureIds]);
  const visible = useMemo(
    () => (sel.size === 0 ? picks : picks.filter((l) => !l.fixture_id || sel.has(l.fixture_id))),
    [picks, sel]
  );
  const hidden = picks.length - visible.length;

  const byMatch = useMemo(() => {
    const m = new Map<string, TicketLeg[]>();
    for (const l of visible) {
      m.set(l.match, [...(m.get(l.match) ?? []), l]);
    }
    return m;
  }, [visible]);

  // ⧉ Portable export — paste into any external model (Sonnet etc.) or notes.
  function copyBoard() {
    const lines: string[] = ["MY PICKS (football betting shortlist)", ""];
    for (const [match, ls] of byMatch.entries()) {
      lines.push(`## ${match}`);
      for (const l of ls) {
        lines.push(
          `- ${l.selection} · ${l.market}${l.line ? ` ${l.line}` : ""}` +
            `${l.est_prob != null ? ` · model ${Math.round(l.est_prob * 100)}%` : ""}` +
            `${l.pinnacle_prob != null ? ` · sharp ${Math.round((l.pinnacle_prob as number) * 100)}%` : ""}` +
            `${l.book_odds != null ? ` · odds ${l.book_odds.toFixed(2)}` : " · unpriced"}`
        );
      }
      lines.push("");
    }
    const shownT = aiTickets ?? combos;
    if (shownT && shownT.length > 0) {
      lines.push("## GENERATED TICKETS");
      for (const t of shownT) {
        lines.push(
          `- ${t.title}${t.combined_prob != null ? ` (~${Math.round(t.combined_prob * 100)}%` : "("}${t.combined_odds != null ? ` @${t.combined_odds.toFixed(2)})` : ")"}: ` +
            t.legs.map((l) => `${l.selection} ${l.market}${l.line ? ` ${l.line}` : ""} [${l.match}]`).join(" + ")
        );
      }
    }
    navigator.clipboard.writeText(lines.join("\n")).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    });
  }

  // Deterministic N-folds: every combination of `fold` picks with no repeated
  // subject and no two picks of the same market from one match; ranked by
  // combined est_prob; top 12 shown. Pure frontend math — zero tokens.
  function generate() {
    const legs = visible.filter((l) => l.est_prob != null);
    if (legs.length < fold) {
      toast.error(`Need at least ${fold} picks with probabilities (have ${legs.length}).`);
      return;
    }
    const results: { legs: TicketLeg[]; p: number }[] = [];
    let explored = 0;
    const pick = (start: number, chosen: TicketLeg[], p: number) => {
      if (explored > 30000) return; // safety cap on huge boards
      if (chosen.length === fold) {
        results.push({ legs: [...chosen], p });
        return;
      }
      for (let i = start; i < legs.length; i++) {
        const c = legs[i];
        explored++;
        if (chosen.some((x) => x.selection === c.selection)) continue; // nested/same subject
        if (chosen.some((x) => x.match === c.match && x.market === c.market)) continue; // same market same game
        pick(i + 1, [...chosen, c], p * (c.est_prob as number));
      }
    };
    pick(0, [], 1);
    results.sort((a, b) => b.p - a.p);
    setAiTickets(null);
    setCombos(
      results.slice(0, 12).map(({ legs: ls, p }) => {
        const books = ls.map((l) => l.book_odds).filter((o): o is number => o != null);
        const priced = books.length === ls.length && books.length > 0;
        const fixtures = new Set(ls.map((l) => l.match));
        return {
          type: fixtures.size <= 1 ? "SGP" : "Acca",
          title: `${fold}-fold · ~${Math.round(p * 100)}%`,
          confidence: "",
          legs: ls,
          combined_prob: p,
          combined_odds: priced ? books.reduce((a, b) => a * b, 1) : null,
          combined_ev: null,
          flags: ["from my picks"],
          why: null,
        } as Ticket;
      })
    );
    if (results.length === 0) toast.error("No valid combinations — picks conflict (same subject/market).");
  }

  async function askAi() {
    setBusy(true);
    try {
      const t = await api.picksAiBuild(visible);
      setCombos(null);
      setAiTickets(t);
    } catch (e) {
      toast.error(e);
    } finally {
      setBusy(false);
    }
  }

  const shown = aiTickets ?? combos;

  return (
    <div className="space-y-3">
      <div className="flex items-start justify-between gap-2">
        <p className="text-xs text-slate-400">
          Your hand-picked shortlist ({visible.length}{hidden > 0 ? ` shown · ${hidden} from other matches hidden` : ""}).
          Add picks via the 🍒 cherry-pick cart → “📌 save to My Picks”.
        </p>
        {visible.length > 0 && (
          <button className="btn btn-ghost text-xs py-1.5 shrink-0" onClick={copyBoard} title="Copy the board (and any generated tickets) as text — paste into Sonnet or anywhere for a second opinion">
            {copied ? "✓ copied" : "⧉ Copy"}
          </button>
        )}
      </div>

      {visible.length === 0 && (
        <div className="text-xs text-slate-500">
          {picks.length > 0 ? `No picks for the selected matches (${picks.length} on the board from other fixtures).` : "Board is empty."}
        </div>
      )}

      {[...byMatch.entries()].map(([match, ls]) => (
        <div key={match} className="space-y-1">
          <div className="text-[10px] font-semibold text-slate-400 uppercase tracking-wide">{match}</div>
          {ls.map((l) => (
            <div key={legKey(l)} className="flex items-center justify-between gap-2 text-xs rounded-lg bg-ink border border-edge px-2.5 py-1.5">
              <div className="min-w-0">
                <span className="font-medium">{l.selection}</span>
                <span className="text-slate-500"> · {l.market}{l.line ? ` ${l.line}` : ""}
                  {l.est_prob != null ? ` · ${Math.round(l.est_prob * 100)}%` : ""}
                  {l.book_odds != null ? ` · @${l.book_odds.toFixed(2)}` : ""}
                </span>
              </div>
              <button className="text-slate-500 hover:text-bad shrink-0" onClick={() => onRemove(legKey(l))}>✕</button>
            </div>
          ))}
        </div>
      ))}

      {visible.length >= 2 && (
        <div className="card space-y-2">
          <div className="flex items-center gap-1.5">
            <span className="text-[10px] text-slate-500 shrink-0">fold</span>
            {[2, 3, 4, 5, 6].map((n) => (
              <button key={n} className={`chip flex-1 text-center ${fold === n ? "chip-on" : ""}`} onClick={() => setFold(n)}>
                {n}
              </button>
            ))}
          </div>
          <div className="flex gap-2">
            <button className="btn btn-ghost flex-1 text-xs" onClick={generate}>
              ⚙ Top {fold}-folds (0 tokens)
            </button>
            <button className="btn btn-primary flex-1 text-xs" disabled={busy} onClick={askAi}>
              {busy ? <Spinner /> : "🤖 AI: build a usable set"}
            </button>
          </div>
          <button className="text-[10px] text-slate-500 underline" onClick={onClear}>clear board</button>
        </div>
      )}

      {shown && shown.map((t, i) => <PickTicket key={i} t={t} onPlace={onPlace} />)}
    </div>
  );
}

function PickTicket({ t, onPlace }: { t: Ticket; onPlace: (t: Ticket, stake: number, odds: number | null) => Promise<void> }) {
  const [stake, setStake] = useState("");
  const [placed, setPlaced] = useState(false);
  return (
    <div className="card space-y-1.5">
      <div className="flex items-center justify-between gap-2">
        <div className="text-xs font-semibold truncate">{t.title}</div>
        <div className="text-[11px] text-slate-400 shrink-0">
          {t.combined_prob != null ? `${Math.round(t.combined_prob * 100)}%` : ""}
          {t.combined_odds != null ? ` · @${t.combined_odds.toFixed(2)}` : " · partly unpriced"}
        </div>
      </div>
      {t.why && <p className="text-[11px] text-slate-400">{t.why}</p>}
      <div className="space-y-0.5">
        {t.legs.map((l, j) => (
          <div key={j} className="text-[11px] text-slate-300">
            {l.selection} <span className="text-slate-500">· {l.market}{l.line ? ` ${l.line}` : ""} · {l.match}</span>
          </div>
        ))}
      </div>
      <div className="flex items-center gap-1.5">
        <span className="text-sm text-slate-500">$</span>
        <input className="w-16 rounded-lg bg-ink border border-edge px-2 py-1.5 text-sm" placeholder="stake" inputMode="decimal" value={stake} onChange={(e) => setStake(e.target.value)} />
        <button
          className="btn btn-primary text-xs px-3 py-1.5 flex-1"
          disabled={placed}
          onClick={async () => {
            const s = parseFloat(stake);
            if (!Number.isFinite(s) || s <= 0) return;
            await onPlace(t, s, t.combined_odds ?? null);
            setPlaced(true);
          }}
        >
          {placed ? "Placed ✓" : "Place"}
        </button>
      </div>
    </div>
  );
}
