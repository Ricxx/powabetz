import { useEffect, useState } from "react";
import { api } from "../api";
import Spinner from "./Spinner";
import type { FixtureInput, LineupView } from "../types";

// 👥 Starting XIs for the selected fixtures — the confirmation step before
// trusting player props. Source per side: API feed (confirmed), YOUR ingested
// page (SofaScore etc.), or an honest "none yet".
export default function Lineups({ fixtures }: { fixtures: FixtureInput[] }) {
  const [rows, setRows] = useState<LineupView[] | null>(null);
  const [busy, setBusy] = useState(false);

  async function load() {
    setBusy(true);
    try {
      setRows(await api.getLineups(fixtures));
    } catch {
      setRows([]);
    } finally {
      setBusy(false);
    }
  }
  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fixtures.map((f) => f.fixture_id).join(",")]);

  const badge = (src: string) =>
    src === "api" ? (
      <span className="badge bg-accent/20 text-accent">confirmed · feed</span>
    ) : src === "ingested" ? (
      <span className="badge bg-warn/20 text-warn">from your ingested page</span>
    ) : (
      <span className="badge bg-edge text-slate-400">no lineup yet</span>
    );

  if (busy && !rows) return <div className="text-xs text-slate-400 flex items-center gap-2"><Spinner /> Loading lineups…</div>;

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <p className="text-xs text-slate-400">
          Starting XIs — confirm who actually plays before trusting player props. Lineups usually
          post ~40–70 min before kickoff; ingest a SofaScore lineup page (🧲) to fill gaps earlier.
        </p>
        <button className="btn btn-ghost text-xs py-1.5 shrink-0 ml-2" onClick={load} disabled={busy}>
          {busy ? "…" : "↻"}
        </button>
      </div>
      {(rows ?? []).map((r) => (
        <div key={r.fixture_id} className="card space-y-2">
          <div className="text-sm font-semibold">{r.label}</div>
          <div className="grid grid-cols-2 gap-3">
            {r.sides.map((s, i) => (
              <div key={i} className="space-y-1 min-w-0">
                <div className="text-xs font-semibold text-slate-300 break-words">{s.team}</div>
                {badge(s.source)}
                {s.players.length === 0 ? (
                  <div className="text-[11px] text-slate-500">
                    Not posted yet — check back near kickoff, or ingest a lineup page.
                  </div>
                ) : (
                  <ol className="text-[11px] text-slate-300 space-y-0.5">
                    {s.players.map((p, j) => (
                      <li key={j} className="truncate">{p}</li>
                    ))}
                  </ol>
                )}
              </div>
            ))}
          </div>
        </div>
      ))}
      {rows && rows.length === 0 && <div className="text-xs text-slate-500">Select matches first.</div>}
    </div>
  );
}
