import { useEffect, useState } from "react";
import { api } from "../api";
import type { BuildResult, SavedTicket } from "../types";
import Results from "./Results";

export default function History({ onClose }: { onClose: () => void }) {
  const [tickets, setTickets] = useState<SavedTicket[]>([]);
  const [open, setOpen] = useState<SavedTicket | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    api.listTickets().then(setTickets).catch((e) => setErr(String(e)));
  }, []);

  if (open) {
    let parsed: BuildResult | null = null;
    try {
      parsed = JSON.parse(open.result_json);
    } catch {
      parsed = null;
    }
    return (
      <div className="space-y-4">
        <div className="flex items-center justify-between">
          <h2 className="text-lg font-bold">Saved ticket</h2>
          <button className="btn btn-ghost text-sm py-2" onClick={() => setOpen(null)}>
            Back
          </button>
        </div>
        {open.user_notes && <div className="card text-sm text-slate-300">{open.user_notes}</div>}
        {parsed ? (
          <Results result={parsed} />
        ) : (
          <div className="card text-sm text-bad">Could not read this saved ticket.</div>
        )}
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-bold">History</h2>
        <button className="btn btn-ghost text-sm py-2" onClick={onClose}>
          Done
        </button>
      </div>
      {err && <div className="text-xs text-bad">{err}</div>}
      {tickets.length === 0 && <div className="card text-sm text-slate-400">No saved tickets yet.</div>}
      {tickets.map((t) => {
        let n = 0;
        try {
          n = (JSON.parse(t.result_json) as BuildResult).tickets.length;
        } catch {
          n = 0;
        }
        return (
          <button
            key={t.id}
            className="card w-full text-left hover:border-slate-500"
            onClick={() => setOpen(t)}
          >
            <div className="flex items-center justify-between">
              <div className="text-sm font-semibold">{n} legs</div>
              <div className="text-xs text-slate-400">
                {new Date(t.created_at * 1000).toLocaleString()}
              </div>
            </div>
            {t.user_notes && (
              <div className="text-xs text-slate-400 mt-1 truncate">{t.user_notes}</div>
            )}
          </button>
        );
      })}
    </div>
  );
}
