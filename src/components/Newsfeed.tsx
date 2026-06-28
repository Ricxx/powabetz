import { useEffect, useState } from "react";
import { api } from "../api";
import type { GrokLogEntry } from "../types";

export default function Newsfeed({ onClose }: { onClose: () => void }) {
  const [log, setLog] = useState<GrokLogEntry[]>([]);
  const [open, setOpen] = useState<number | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    api.listGrokLog().then(setLog).catch((e) => setErr(String(e)));
  }, []);

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-bold">Grok newsfeed</h2>
        <button className="btn btn-ghost text-sm py-2" onClick={onClose}>
          Done
        </button>
      </div>
      <p className="text-[11px] text-slate-500">
        Every Grok run's raw output — injuries, team news, sentiment. Use it to audit what fed the
        picks (and whether out players were flagged).
      </p>
      {err && <div className="text-xs text-bad">{err}</div>}
      {log.length === 0 && (
        <div className="card text-sm text-slate-400">
          No Grok runs yet. Enable “Grok X/news sentiment” on a build.
        </div>
      )}
      {log.map((e) => (
        <button
          key={e.id}
          className="card w-full text-left hover:border-slate-500"
          onClick={() => setOpen(open === e.id ? null : e.id)}
        >
          <div className="flex items-center justify-between">
            <div className="text-sm font-semibold truncate">{e.matches}</div>
            <div className="text-xs text-slate-400 shrink-0 ml-2">
              {new Date(e.created_at * 1000).toLocaleString()}
            </div>
          </div>
          {open === e.id && (
            <pre className="text-[11px] text-slate-300 whitespace-pre-wrap mt-2 leading-snug font-sans">
              {e.digest}
            </pre>
          )}
        </button>
      ))}
    </div>
  );
}
