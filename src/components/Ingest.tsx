import { useEffect, useState } from "react";
import { errMsg, toast } from "../toast";
import { api } from "../api";
import Spinner from "./Spinner";
import { ANALYSIS_MODELS, type IngestInfo, type IngestItem } from "../types";

function modelLabel(m?: string | null): string {
  if (!m) return "";
  return ANALYSIS_MODELS.find((x) => x.id === m)?.label || (m.includes("haiku") ? "Haiku" : m);
}

function fmtDate(ts: number): string {
  try {
    return new Date(ts * 1000).toLocaleDateString(undefined, { month: "short", day: "numeric" });
  } catch {
    return "";
  }
}

export default function Ingest({ onClose }: { onClose: () => void }) {
  const [info, setInfo] = useState<IngestInfo | null>(null);
  const [items, setItems] = useState<IngestItem[]>([]);
  const [busy, setBusy] = useState<Set<number>>(new Set()); // ids currently processing
  const [bulk, setBulk] = useState<{ done: number; total: number } | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [extPath, setExtPath] = useState<string | null>(null);
  const [showSetup, setShowSetup] = useState(false);
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [procModel, setProcModel] = useState("claude-haiku-4-5");

  function load() {
    api.ingestInfo().then(setInfo).catch(() => {});
    api.listIngested().then(setItems).catch((e) => setErr(errMsg(e)));
  }
  useEffect(load, []);
  // Auto-refresh so pages sent from the browser appear without a manual reload.
  useEffect(() => {
    const t = setInterval(() => {
      api.listIngested().then(setItems).catch(() => {});
    }, 5000);
    return () => clearInterval(t);
  }, []);

  async function downloadExt() {
    try {
      setExtPath(await api.exportExtension());
    } catch (e) {
      setErr(errMsg(e));
    }
  }
  async function process(id: number): Promise<boolean> {
    setBusy((prev) => new Set(prev).add(id));
    setErr(null);
    try {
      const updated = await api.processIngested(id, procModel);
      setItems((prev) => prev.map((x) => (x.id === id ? updated : x)));
      setExpanded((prev) => new Set(prev).add(id)); // show the output so it's verifiable
      return true;
    } catch (e) {
      setErr(errMsg(e));
      return false;
    } finally {
      setBusy((prev) => {
        const n = new Set(prev);
        n.delete(id);
        return n;
      });
    }
  }

  // Process a set of un-processed pages, one at a time (bounded cost, sequential).
  async function processIds(idsAll: number[]) {
    const ids = idsAll.filter((id) => items.find((x) => x.id === id)?.status !== "processed");
    if (ids.length === 0) return;
    setBulk({ done: 0, total: ids.length });
    for (let i = 0; i < ids.length; i++) {
      await process(ids[i]);
      setBulk({ done: i + 1, total: ids.length });
    }
    setBulk(null);
  }
  const processAllNew = () => processIds(items.filter((x) => x.status !== "processed").map((x) => x.id));
  function del(id: number) {
    setItems((prev) => prev.filter((x) => x.id !== id)); // optimistic
    const timer = setTimeout(() => {
      api.deleteIngested(id).catch(toast.error);
    }, 5000);
    toast.undo("Page removed", () => {
      clearTimeout(timer);
      load(); // still on the server — restore the list
    });
  }
  async function saveNote(id: number, note: string) {
    try {
      await api.updateIngestNote(id, note);
    } catch (e) {
      toast.error(e);
    }
  }
  function toggle(id: number) {
    setExpanded((prev) => {
      const n = new Set(prev);
      n.has(id) ? n.delete(id) : n.add(id);
      return n;
    });
  }

  const endpoint = info ? `http://127.0.0.1:${info.port}/ingest` : "";

  // Group by fixture, merging slight label/date variations ("A vs B" == "B v A")
  // so the same match never shows under two headings.
  const fixtureKey = (label: string) =>
    label
      .toLowerCase()
      .replace(/\bv(s|ersus)?\b/g, "|")
      .replace(/[^a-z0-9|]/g, "")
      .split("|")
      .map((s) => s.trim())
      .filter(Boolean)
      .sort()
      .join("|");
  const groups = new Map<string, { label: string; date: string; items: IngestItem[] }>();
  for (const it of items) {
    const label = it.fixture_label || "Unmatched (process to tag a fixture)";
    const date = it.fixture_date || "";
    const key = it.fixture_label ? fixtureKey(it.fixture_label) : "unmatched";
    if (!groups.has(key)) groups.set(key, { label, date, items: [] });
    groups.get(key)!.items.push(it);
  }
  const grouped = [...groups.values()].sort((a, b) => (b.date || "").localeCompare(a.date || ""));

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-bold">Ingested pages</h2>
        <div className="flex items-center gap-2">
          <button className="btn btn-ghost text-sm py-2" onClick={load} title="Refresh">
            ↻
          </button>
          <button className="btn btn-ghost text-sm py-2" onClick={onClose}>
            Done
          </button>
        </div>
      </div>

      {items.length > 0 && (
        <div className="flex items-center gap-1.5 flex-wrap text-xs">
          <span className="text-[10px] text-slate-500">Process with:</span>
          {ANALYSIS_MODELS.map((m) => (
            <button
              key={m.id}
              className={`chip text-[10px] py-0.5 ${procModel === m.id ? "chip-on" : ""}`}
              onClick={() => setProcModel(m.id)}
              title={m.provider === "openai" ? "needs an OpenAI key" : ""}
            >
              {m.label}
            </button>
          ))}
        </div>
      )}

      {items.some((x) => x.status !== "processed") && (
        <button
          className="btn btn-primary w-full text-sm"
          disabled={bulk !== null}
          onClick={processAllNew}
        >
          {bulk ? (
            <span className="inline-flex items-center gap-2">
              <Spinner /> Processing {bulk.done}/{bulk.total}…
            </span>
          ) : (
            `Process all new (${items.filter((x) => x.status !== "processed").length}) with ${modelLabel(procModel)}`
          )}
        </button>
      )}

      {/* Collapsible setup */}
      <div className="card">
        <button className="w-full flex items-center justify-between text-left" onClick={() => setShowSetup((v) => !v)}>
          <span className="text-xs font-semibold text-slate-300">🧩 Browser extension — install &amp; connect</span>
          <span className="text-xs text-slate-500">{showSetup ? "hide ▴" : "show ▾"}</span>
        </button>
        {showSetup && (
          <div className="mt-2 space-y-2">
            <button className="btn btn-primary w-full text-sm" onClick={downloadExt}>
              ⬇ Save extension folder &amp; open it
            </button>
            <p className="text-[10px] text-slate-500">
              Chrome extensions aren't a single installer file — the <b>folder is the extension</b>.
              Chrome loads it directly via “Load unpacked”. Nothing to compile.
            </p>
            {extPath && <div className="text-[11px] text-accent break-all">Saved to: {extPath}</div>}
            <ol className="text-[11px] text-slate-400 list-decimal pl-4 space-y-0.5">
              <li>Click the button above — it saves the <code>powabetz-extension</code> folder and opens it.</li>
              <li>Open <code>chrome://extensions</code> → toggle on <b>Developer mode</b> (top-right).</li>
              <li>Click <b>Load unpacked</b> → select that <code>powabetz-extension</code> folder.</li>
              <li>Click the new extension's icon → paste the <b>Endpoint + Token</b> below → <b>Save</b>.</li>
              <li>On any fixture page: <b>right-click → Add to Powabetz</b> (or “Ingest with note…”).</li>
            </ol>
            {info && (
              <div className="space-y-1 text-xs pt-1 border-t border-edge">
                <Row label="Endpoint" value={endpoint} />
                <Row label="Token" value={info.token} mono />
                <div className="text-[11px] text-slate-500">
                  Server: {info.enabled ? <span className="text-accent">on</span> : <span className="text-bad">off (enable in Settings)</span>}
                </div>
                <button
                  className="btn btn-ghost text-xs"
                  onClick={() => {
                    navigator.clipboard.writeText(`${endpoint}\n${info.token}`).then(() => {
                      setCopied(true);
                      setTimeout(() => setCopied(false), 1500);
                    });
                  }}
                >
                  {copied ? "Copied!" : "Copy endpoint + token"}
                </button>
              </div>
            )}
          </div>
        )}
      </div>

      {err && <div className="text-xs text-bad">{err}</div>}

      {items.length === 0 && (
        <div className="card text-sm text-slate-400">
          Nothing ingested yet. On a fixture page in your browser, right-click → Add to Powabetz.
        </div>
      )}

      {grouped.map((g) => {
        const gkey = g.label + g.date;
        const isCollapsed = collapsed.has(gkey);
        const unprocessed = g.items.filter((x) => x.status !== "processed").length;
        return (
        <div key={gkey} className="space-y-2">
          <div className="flex items-center gap-2 px-1">
            <button
              className="flex items-center gap-2 text-left min-w-0"
              onClick={() => setCollapsed((p) => { const n = new Set(p); n.has(gkey) ? n.delete(gkey) : n.add(gkey); return n; })}
            >
              <span className="text-slate-500 text-xs">{isCollapsed ? "▸" : "▾"}</span>
              <span className="text-sm font-semibold text-slate-200 truncate">📌 {g.label}</span>
              {g.date && <span className="text-[11px] text-slate-500 shrink-0">{g.date}</span>}
              <span className="text-[11px] text-slate-500 shrink-0">· {g.items.length} src{unprocessed > 0 ? `, ${unprocessed} new` : ""}</span>
            </button>
            {unprocessed > 0 && (
              <button
                className="ml-auto chip text-[10px] py-0.5 shrink-0"
                disabled={!!bulk}
                onClick={() => processIds(g.items.map((x) => x.id))}
                title="Process every new source for this fixture"
              >
                Process {unprocessed}
              </button>
            )}
          </div>

          {!isCollapsed && g.items.map((it) => (
            <div key={it.id} className="card space-y-1.5">
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <div className="text-sm font-medium truncate">{it.title || it.url}</div>
                  <div className="text-[11px] text-slate-500 flex items-center gap-2 flex-wrap">
                    <a className="truncate hover:text-slate-300" href={it.url} target="_blank" rel="noreferrer">
                      {hostOf(it.url)}
                    </a>
                    <span>· ingested {fmtDate(it.created_at)}</span>
                    {it.status === "processed" && it.model && <span>· by {modelLabel(it.model)}</span>}
                    {it.used && <span className="text-accent">· used in a build</span>}
                  </div>
                </div>
                <span className={`badge shrink-0 ${it.status === "processed" ? "bg-accent/20 text-accent" : "bg-edge text-slate-300"}`}>
                  {it.status === "processed" ? "processed" : "new"}
                </span>
              </div>

              {it.summary && <p className="text-xs text-slate-400">{it.summary}</p>}

              {/* Extraction output — viewable so you can check it isn't garbage */}
              {it.status === "processed" && it.data.length > 0 && (
                <div>
                  <button className="text-[11px] text-slate-400 hover:text-slate-100" onClick={() => toggle(it.id)}>
                    {expanded.has(it.id) ? "▴ hide extracted data" : `▾ view extracted data (${it.data.length})`}
                  </button>
                  {expanded.has(it.id) && (
                    <table className="w-full text-[11px] mt-1">
                      <tbody>
                        {it.data.map((kv, i) => (
                          <tr key={i} className="border-t border-edge">
                            <td className="py-0.5 pr-2 text-slate-500 align-top">{kv.label}</td>
                            <td className="py-0.5 text-slate-200">{kv.value}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  )}
                </div>
              )}

              <textarea
                className="w-full rounded-lg bg-ink border border-edge px-2 py-1.5 text-xs"
                rows={2}
                placeholder="Note for Haiku (e.g. 'extract only the predicted scoreline + corners')"
                defaultValue={it.note}
                onBlur={(e) => saveNote(it.id, e.target.value)}
              />

              <div className="flex items-center gap-3 text-xs">
                <button
                  className="btn btn-primary text-xs px-3 py-1.5"
                  disabled={busy.has(it.id) || bulk !== null}
                  onClick={() => process(it.id)}
                >
                  {busy.has(it.id) ? (
                    <span className="inline-flex items-center gap-2"><Spinner /> Processing…</span>
                  ) : it.status === "processed" ? (
                    `Re-process with ${modelLabel(procModel)}`
                  ) : (
                    `Process with ${modelLabel(procModel)}`
                  )}
                </button>
                <button className="underline text-slate-500" onClick={() => del(it.id)}>
                  delete
                </button>
              </div>
            </div>
          ))}
        </div>
        );
      })}
    </div>
  );
}

function hostOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return url;
  }
}

function Row({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex items-baseline gap-2">
      <span className="text-slate-500 w-16 shrink-0">{label}</span>
      <span className={`text-slate-200 truncate ${mono ? "font-mono" : ""}`}>{value}</span>
    </div>
  );
}
