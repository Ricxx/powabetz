import { useEffect, useRef, useState } from "react";
import { errMsg, toast } from "../toast";
import type { FixtureInput } from "../types";
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

export default function Ingest({ onClose, fixtures = [] }: { onClose: () => void; fixtures?: FixtureInput[] }) {
  const [fixing, setFixing] = useState(false);
  const [info, setInfo] = useState<IngestInfo | null>(null);
  const [items, setItems] = useState<IngestItem[]>([]);
  const [busy, setBusy] = useState<Set<number>>(new Set()); // ids currently processing
  const [bulk, setBulk] = useState<{ done: number; total: number } | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [extPath, setExtPath] = useState<string | null>(null);
  const [showSetup, setShowSetup] = useState(false);
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  const [openGroups, setOpenGroups] = useState<Set<string>>(new Set()); // collapsed by default
  const [showArchive, setShowArchive] = useState(false);
  const [procModel, setProcModel] = useState("deepseek-v4-pro");

  function load() {
    api.ingestInfo().then(setInfo).catch(() => {});
    api
      .listIngested()
      .then((list) => {
        // Auto-clean: drop archived pages whose fixture was 7+ days ago.
        const cutoff = new Date(Date.now() - 7 * 86400_000).toISOString().slice(0, 10);
        const stale = list.filter((it) => it.fixture_date && it.fixture_date < cutoff);
        if (stale.length > 0) {
          Promise.allSettled(stale.map((it) => api.deleteIngested(it.id))).then(() =>
            api.listIngested().then(setItems).catch(() => {})
          );
          setItems(list.filter((it) => !stale.some((s) => s.id === it.id)));
        } else {
          setItems(list);
        }
      })
      .catch((e) => setErr(errMsg(e)));
  }
  useEffect(load, []);
  // Auto-refresh so pages sent from the browser appear without a manual reload.
  // The poll filters out rows with a PENDING deferred delete — it used to
  // resurrect an optimistically-removed item mid-undo-window (flash back, then
  // vanish again when the deferred delete landed).
  const pendingDelete = useRef<Set<number>>(new Set());
  useEffect(() => {
    const t = setInterval(() => {
      api
        .listIngested()
        .then((list) => setItems(list.filter((x) => !pendingDelete.current.has(x.id))))
        .catch(() => {});
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
    pendingDelete.current.add(id);
    const timer = setTimeout(() => {
      api
        .deleteIngested(id)
        .catch(toast.error)
        .finally(() => pendingDelete.current.delete(id));
    }, 5000);
    toast.undo("Page removed", () => {
      clearTimeout(timer);
      pendingDelete.current.delete(id);
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
  async function assignFixture(it: IngestItem, label: string) {
    try {
      await api.assignIngestFixture(it.id, label, it.fixture_date || undefined);
      toast.success(`Assigned to ${label}`);
      load();
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
  const groups = new Map<string, { label: string; date: string; resolved: boolean; items: IngestItem[] }>();
  for (const it of items) {
    const label = it.fixture_label || "Unmatched (process to tag a fixture)";
    const date = it.fixture_date || "";
    const resolved = it.date_source === "fixture";
    const key = it.fixture_label ? fixtureKey(it.fixture_label) : "unmatched";
    if (!groups.has(key)) groups.set(key, { label, date, resolved, items: [] });
    const g = groups.get(key)!;
    g.items.push(it);
    // A fixture-RESOLVED date (real kickoff in YOUR timezone) beats whatever
    // date the page printed (site timezone — often a day ahead for late games).
    if (resolved && !g.resolved) {
      g.date = date;
      g.resolved = true;
    }
  }
  // Active = matches today/upcoming (or undated); Archive = day already passed.
  // Use LOCAL calendar dates (not UTC) so an evening kickoff isn't pushed to
  // "tomorrow" when UTC has already rolled over.
  const ymd = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  const today = ymd(new Date());
  const tomorrow = ymd(new Date(Date.now() + 86400_000));
  const all = [...groups.values()];
  const active = all
    .filter((g) => !g.date || g.date >= today)
    .sort((a, b) => (a.date || "9999").localeCompare(b.date || "9999") || a.label.localeCompare(b.label));
  const archive = all.filter((g) => g.date && g.date < today).sort((a, b) => (b.date || "").localeCompare(a.date || ""));

  const dayLabel = (d: string) => {
    if (!d) return "Undated";
    const dt = new Date(d + "T00:00:00");
    if (d === today) return "Today";
    if (d === tomorrow) return "Tomorrow";
    return dt.toLocaleDateString([], { weekday: "short", month: "short", day: "numeric" });
  };

  // One ingested source card.
  const renderItem = (it: IngestItem) => (
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

      {/* Fixture assignment — correct a mis-tagged page or assign an unmatched one. */}
      <div className="flex items-center gap-1.5 text-[11px]">
        <span className="text-slate-500 shrink-0" title="Which match this page is about — fix it if wrong">📌</span>
        <input
          className={`flex-1 rounded bg-ink border px-1.5 py-1 text-slate-200 ${it.fixture_label ? "border-edge" : "border-warn/60"}`}
          defaultValue={it.fixture_label || ""}
          placeholder="Home vs Away — assign this page's match"
          onBlur={(e) => {
            const v = e.target.value.trim();
            if (v && v !== (it.fixture_label || "")) assignFixture(it, v);
          }}
        />
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
  );

  // One fixture's collapsible group (collapsed by default).
  const renderGroup = (g: { label: string; date: string; items: IngestItem[] }) => {
    const gkey = g.label + g.date;
    const isOpen = openGroups.has(gkey);
    const unprocessed = g.items.filter((x) => x.status !== "processed").length;
    return (
      <div key={gkey} className="space-y-2">
        <div className="flex items-center gap-2 px-1">
          <button
            className="flex items-center gap-2 text-left min-w-0"
            onClick={() => setOpenGroups((p) => { const n = new Set(p); n.has(gkey) ? n.delete(gkey) : n.add(gkey); return n; })}
          >
            <span className="text-slate-500 text-xs">{isOpen ? "▾" : "▸"}</span>
            <span className="text-sm font-semibold text-slate-200 truncate">📌 {g.label}</span>
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
        {isOpen && g.items.map((it) => renderItem(it))}
      </div>
    );
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-bold">Ingested pages</h2>
        <div className="flex items-center gap-2">
          <button
            className="btn btn-ghost text-sm py-2 disabled:opacity-40"
            disabled={fixing || fixtures.length === 0}
            title={fixtures.length === 0 ? "Load a day's matches first (pick a date), then fix names" : "One cached DeepSeek call re-matches badly-extracted team names to the loaded fixtures, so pages group correctly"}
            onClick={async () => {
              setFixing(true);
              try {
                const n = await api.fixIngestNames(fixtures);
                toast.success(n > 0 ? `Fixed ${n} page name${n > 1 ? "s" : ""}.` : "Nothing to fix — all pages already match a fixture.");
                load();
              } catch (e) {
                toast.error(e);
              } finally {
                setFixing(false);
              }
            }}
          >
            {fixing ? "🩹…" : "🩹 Fix names"}
          </button>
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

      {/* ACTIVE — today & upcoming, grouped by day, earliest first. */}
      {active.length > 0 &&
        active.map((g, i) => {
          const prev = active[i - 1];
          const showDay = !prev || (prev.date || "") !== (g.date || "");
          return (
            <div key={g.label + g.date} className="space-y-2">
              {showDay && (
                <div className="text-[11px] font-bold text-accent uppercase tracking-wide pt-1 px-1">
                  📅 {dayLabel(g.date)}
                  {g.date && !g.resolved && (
                    <span className="text-slate-500 normal-case font-normal" title="This is the date as the SOURCE PAGE printed it (its timezone, often a day ahead for late kickoffs). It snaps to the real kickoff in YOUR timezone after the first build that uses this page.">
                      {" "}· site date
                    </span>
                  )}
                </div>
              )}
              {renderGroup(g)}
            </div>
          );
        })}

      {/* ARCHIVE — matches whose day has passed. Not used in future builds; auto-deleted 7 days on. */}
      {archive.length > 0 && (
        <div className="space-y-2 pt-2 border-t border-edge">
          <button
            className="w-full flex items-center justify-between text-left px-1"
            onClick={() => setShowArchive((v) => !v)}
          >
            <span className="text-xs font-semibold text-slate-400">🗄 Archive — {archive.length} past fixture{archive.length === 1 ? "" : "s"}</span>
            <span className="text-[11px] text-slate-500">{showArchive ? "hide ▴" : "show ▾"}</span>
          </button>
          {showArchive && (
            <>
              <p className="text-[10px] text-slate-500 px-1">Past matches — kept for reference, never used in new builds, auto-removed 7 days after kickoff.</p>
              {archive.map((g) => (
                <div key={g.label + g.date} className="space-y-2">
                  <div className="text-[11px] text-slate-600 px-1">📅 {dayLabel(g.date)}</div>
                  {renderGroup(g)}
                </div>
              ))}
            </>
          )}
        </div>
      )}
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
