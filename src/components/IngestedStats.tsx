import { useEffect, useState } from "react";
import { errMsg } from "../toast";
import { api } from "../api";
import type { FixtureInput, IngestItem } from "../types";

const fold = (s: string) => s.toLowerCase().replace(/[^a-z0-9]/g, "");
// Mirror the backend's token-aware team matcher (odds::team_match) so a page
// that matches in a Scout build also shows here: full-name containment, then
// distinctive tokens (≥4 chars, not generic suffixes), then reverse-prefix
// ("barca" → "barcelona").
const STOP = new Set(["club", "city", "united", "town", "county", "real", "sporting", "athletic", "atletico", "deportivo"]);
const teamMatch = (labelRaw: string, team: string): boolean => {
  const hay = fold(labelRaw);
  const t = fold(team);
  if (!hay || !t) return false;
  if (hay.includes(t)) return true;
  const hayWords = labelRaw.toLowerCase().split(/[^a-z0-9]+/).filter(Boolean);
  return team
    .toLowerCase()
    .split(/[^a-z0-9]+/)
    .some(
      (tok) =>
        tok.length >= 4 &&
        !STOP.has(tok) &&
        (hay.includes(tok) || hayWords.some((w) => w.length >= 4 && tok.startsWith(w)))
    );
};

// The scraped stat rows from your ingested pages, matched to the selected fixtures.
export default function IngestedStats({ fixtures }: { fixtures: FixtureInput[] }) {
  const [items, setItems] = useState<IngestItem[]>([]);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    api.listIngested().then(setItems).catch((e) => setErr(errMsg(e)));
  }, []);

  // A page matches a fixture only when BOTH of that fixture's teams appear in
  // its label (token-aware, mirroring the backend) — one-team matching showed
  // "Arsenal vs Chelsea" pages under a "Chelsea vs Liverpool" slate.
  const matched = items.filter((it) => {
    if (it.status !== "processed" || !it.fixture_label) return false;
    return fixtures.some((f) => teamMatch(it.fixture_label!, f.home_team) && teamMatch(it.fixture_label!, f.away_team));
  });

  return (
    <div className="space-y-3">
      <p className="text-[11px] text-slate-500">
        The stats scraped from the pages you ingested, matched to your selected fixtures. Feeds Scout and the model as
        context. Fix a mis-tagged page in the 🧲 Ingest screen.
      </p>
      {err && <div className="text-xs text-bad">{err}</div>}
      {matched.length === 0 && (
        <div className="card text-sm text-slate-400">
          No processed ingested pages match your selected fixtures yet. Ingest a stats/preview page and process it.
        </div>
      )}
      {matched.map((it) => (
        <div key={it.id} className="card space-y-1.5">
          <div className="text-sm font-semibold">📌 {it.fixture_label}</div>
          {it.summary && <p className="text-xs text-slate-400">{it.summary}</p>}
          {it.data.length > 0 ? (
            <table className="w-full text-[11px]">
              <tbody>
                {it.data.map((kv, i) => (
                  <tr key={i} className="border-t border-edge">
                    <td className="py-0.5 pr-2 text-slate-500 align-top">{kv.label}</td>
                    <td className="py-0.5 text-slate-200">{kv.value}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            <div className="text-[11px] text-slate-500">No extracted stat rows on this page.</div>
          )}
          <div className="text-[10px] text-slate-500 truncate">from {it.url}</div>
        </div>
      ))}
    </div>
  );
}
