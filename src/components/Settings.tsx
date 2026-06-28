import { useEffect, useState } from "react";
import { api } from "../api";
import { COMMON_BOOKS, MODEL_OPTIONS, TIMEZONES, type BankrollView, type SettingsView } from "../types";

export default function Settings({
  settings,
  onSaved,
  onClose,
}: {
  settings: SettingsView;
  onSaved: (s: SettingsView) => void;
  onClose: () => void;
}) {
  const [af, setAf] = useState("");
  const [an, setAn] = useState("");
  const [grok, setGrok] = useState("");
  const [parlay, setParlay] = useState("");
  const [model, setModel] = useState(settings.model);
  const [limit, setLimit] = useState(String(settings.meter.limit));
  const [selBooks, setSelBooks] = useState<Set<string>>(new Set(settings.books));
  const [kelly, setKelly] = useState(settings.kelly_fraction);
  const [tz, setTz] = useState(settings.timezone);
  const [proxyUrl, setProxyUrl] = useState(settings.proxy_url || "");
  const [proxyToken, setProxyToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const [bank, setBank] = useState<BankrollView | null>(null);
  const [bankInput, setBankInput] = useState("");

  useEffect(() => {
    api.getBankroll().then((b) => {
      setBank(b);
      setBankInput(String(b.bankroll));
    }).catch(() => {});
  }, []);

  const meter = settings.meter;
  const pct = Math.min(100, Math.round((meter.count / meter.limit) * 100));

  async function save() {
    setBusy(true);
    setErr(null);
    try {
      const lim = parseInt(limit, 10);
      const next = await api.saveSettings(
        af || null,
        an || null,
        grok || null,
        parlay || null,
        model,
        Number.isFinite(lim) && lim > 0 ? lim : null,
        [...selBooks],
        kelly,
        tz,
        proxyUrl.trim(),
        proxyToken || null
      );
      onSaved(next);
      setAf("");
      setAn("");
      setGrok("");
      setParlay("");
      setProxyToken("");
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  const [dataMsg, setDataMsg] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);

  async function doExport() {
    try {
      const json = await api.exportData();
      const blob = new Blob([json], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `powabet-backup-${new Date().toISOString().slice(0, 10)}.json`;
      document.body.appendChild(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(url);
      setDataMsg("Exported — saved to your Downloads.");
    } catch (e) {
      setErr(String(e));
    }
  }

  async function doImport(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    e.target.value = ""; // allow re-importing the same file
    if (!file) return;
    if (!confirm(`Import "${file.name}"? This REPLACES your current bets, picks, ledger and stats.`)) return;
    setImporting(true);
    setErr(null);
    try {
      const text = await file.text();
      const n = await api.importData(text);
      setDataMsg(`Imported ${n} rows. Reloading…`);
      setTimeout(() => window.location.reload(), 700);
    } catch (e) {
      setErr(String(e));
    } finally {
      setImporting(false);
    }
  }

  async function doReset() {
    if (!confirm("Reset everything? This permanently clears ALL bets, generated tickets, saved picks, stats, calibration learning and caches. Your API keys and settings are kept. This cannot be undone.")) return;
    if (!confirm("Are you sure? There's no undo.")) return;
    try {
      await api.resetData();
      setDataMsg("Reset complete. Reloading…");
      setTimeout(() => window.location.reload(), 700);
    } catch (e) {
      setErr(String(e));
    }
  }

  async function saveBankroll() {
    const v = parseFloat(bankInput);
    if (!Number.isFinite(v) || v < 0) return;
    try {
      setBank(await api.setBankroll(v));
    } catch (e) {
      setErr(String(e));
    }
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-bold">Settings</h2>
        <button className="btn btn-ghost text-sm py-2" onClick={onClose}>
          Done
        </button>
      </div>

      <div className="card space-y-1">
        <div className="text-xs text-slate-400">Today's request budget</div>
        <div className="text-2xl font-bold">
          {meter.count} <span className="text-base text-slate-400">/ {meter.limit}</span>
        </div>
        <div className="h-2 rounded-full bg-edge overflow-hidden">
          <div
            className={`h-full ${pct >= 100 ? "bg-bad" : pct >= 80 ? "bg-warn" : "bg-accent"}`}
            style={{ width: `${pct}%` }}
          />
        </div>
      </div>

      <div className="card space-y-1">
        <div className="text-xs text-slate-400">Claude usage (lifetime)</div>
        <div className="text-2xl font-bold">${settings.usage.cost_usd.toFixed(2)}</div>
        <div className="text-xs text-slate-400">
          {settings.usage.input_tokens.toLocaleString()} in ·{" "}
          {settings.usage.output_tokens.toLocaleString()} out tokens
        </div>
      </div>

      <div className="card space-y-2">
        <div className="text-xs font-semibold text-slate-400">Bankroll</div>
        {bank && (
          <div className="text-xs text-slate-400">
            Balance <b className="text-slate-100">${bank.current.toFixed(2)}</b> · P&amp;L{" "}
            <b className={bank.pnl >= 0 ? "text-accent" : "text-bad"}>${bank.pnl.toFixed(2)}</b>
          </div>
        )}
        <div className="flex gap-2">
          <input
            type="number"
            inputMode="decimal"
            className="flex-1 rounded-lg bg-ink border border-edge px-3 py-2 text-sm"
            placeholder="starting bankroll $"
            value={bankInput}
            onChange={(e) => setBankInput(e.target.value)}
          />
          <button className="btn btn-ghost text-sm px-3" onClick={saveBankroll}>
            Set
          </button>
        </div>

        <div className="pt-1">
          <div className="text-xs font-semibold text-slate-400 mb-1">Stake sizing (Kelly)</div>
          <p className="text-[11px] text-slate-500 mb-2">
            Suggests a stake per ticket from edge + bankroll. Fractional Kelly trades a little
            growth for far less variance — ¼ is the sane default.
          </p>
          <div className="flex gap-1.5">
            {[
              ["Off", 0],
              ["¼ Kelly", 0.25],
              ["½ Kelly", 0.5],
              ["Full", 1],
            ].map(([label, val]) => (
              <button
                key={label as string}
                className={`chip flex-1 text-center ${kelly === val ? "chip-on" : ""}`}
                onClick={() => setKelly(val as number)}
              >
                {label}
              </button>
            ))}
          </div>
        </div>
      </div>

      <div className="card space-y-3">
        <div>
          <label className="text-xs text-slate-400">Model</label>
          <div className="flex flex-col gap-2 mt-1">
            {MODEL_OPTIONS.map((m) => (
              <button
                key={m.id}
                className={`text-left rounded-lg border px-3 py-2 transition ${
                  model === m.id ? "border-accent bg-accent/10" : "border-edge bg-ink"
                }`}
                onClick={() => setModel(m.id)}
              >
                <div className="text-sm font-semibold">{m.label}</div>
                <div className="text-[11px] text-slate-400">{m.note}</div>
              </button>
            ))}
          </div>
        </div>

        <div>
          <label className="text-xs text-slate-400">Timezone (fixture times &amp; date boundaries)</label>
          <select
            className="w-full mt-1 rounded-lg bg-ink border border-edge px-3 py-2 text-sm"
            value={tz}
            onChange={(e) => setTz(e.target.value)}
          >
            {TIMEZONES.map((z) => (
              <option key={z.id} value={z.id}>
                {z.label}
              </option>
            ))}
          </select>
        </div>

        <div>
          <label className="text-xs text-slate-400">Daily request limit (your API-Football plan)</label>
          <input
            type="number"
            inputMode="numeric"
            className="w-full mt-1 rounded-lg bg-ink border border-edge px-3 py-2 text-sm"
            value={limit}
            onChange={(e) => setLimit(e.target.value)}
          />
        </div>

        <div>
          <label className="text-xs text-slate-400">
            Books to line-shop {selBooks.size === 0 ? "(all)" : `(${selBooks.size})`}
          </label>
          <div className="flex flex-wrap gap-2 mt-1">
            {COMMON_BOOKS.map((b) => {
              const on = selBooks.has(b);
              return (
                <button
                  key={b}
                  className={`chip ${on ? "chip-on" : ""}`}
                  onClick={() =>
                    setSelBooks((prev) => {
                      const next = new Set(prev);
                      next.has(b) ? next.delete(b) : next.add(b);
                      return next;
                    })
                  }
                >
                  {b}
                </button>
              );
            })}
          </div>
          <p className="text-[10px] text-slate-500 mt-1">
            Pinnacle is always used for the sharp true price. Leave empty to compare all books in
            the feed. Selected books set the "price to take".
          </p>
        </div>
      </div>

      <div className="card space-y-3">
        <KeyInput
          label="API-Football key"
          set={settings.has_api_football_key}
          value={af}
          onChange={setAf}
        />
        <KeyInput
          label="Anthropic key"
          set={settings.has_anthropic_key}
          value={an}
          onChange={setAn}
        />
        <KeyInput
          label="Grok (x.ai) key — X sentiment, optional"
          set={settings.has_grok_key}
          value={grok}
          onChange={setGrok}
        />
        <KeyInput
          label="Parlay API key — sharp odds / de-vig / +EV, optional"
          set={settings.has_parlay_key}
          value={parlay}
          onChange={setParlay}
        />
        <div className="pt-1 border-t border-edge">
          <div className="text-xs font-semibold text-slate-300 mt-2">🌐 Server mode (shared access)</div>
          <p className="text-[11px] text-slate-500 mt-0.5 mb-2">
            Point the app at a proxy that holds the keys, so this install needs none of its own.
            Fill these to use someone else's keys via their proxy; leave blank to use your own keys
            above. {settings.has_proxy_token && <span className="text-accent">(token set)</span>}
          </p>
          <label className="text-xs text-slate-400">Proxy URL</label>
          <input
            className="w-full mt-1 mb-2 rounded-lg bg-ink border border-edge px-3 py-2 text-sm"
            placeholder="https://your-proxy.workers.dev"
            value={proxyUrl}
            onChange={(e) => setProxyUrl(e.target.value)}
          />
          <KeyInput
            label="Access token (NOT a provider key)"
            set={settings.has_proxy_token}
            value={proxyToken}
            onChange={setProxyToken}
          />
        </div>
        {err && <div className="text-xs text-bad">{err}</div>}
        <button className="btn btn-primary w-full" onClick={save} disabled={busy}>
          {busy ? "Saving…" : "Save settings"}
        </button>
        <p className="text-[10px] text-slate-500">
          Keys are stored locally in the app data folder, never in the binary. To share the app
          without giving out keys, run the proxy in <code>proxy/</code> and hand users a token.
        </p>
      </div>

      <div className="card space-y-2">
        <div className="text-xs font-semibold text-slate-400">Data — backup &amp; reset</div>
        <p className="text-[11px] text-slate-500">
          Your bets, generated ledger, saved picks, stats and calibration learning live in a local
          database. Back them up, move them between machines, or wipe everything to start fresh.
        </p>
        <div className="flex gap-2">
          <button className="btn btn-ghost flex-1 text-sm" onClick={doExport}>
            ⬇ Export
          </button>
          <label className="btn btn-ghost flex-1 text-sm text-center cursor-pointer">
            {importing ? "Importing…" : "⬆ Import"}
            <input type="file" accept="application/json,.json" className="hidden" onChange={doImport} />
          </label>
        </div>
        <button className="btn btn-ghost w-full text-sm text-bad border-bad/40" onClick={doReset}>
          ⚠ Reset all data (clears bets, picks, stats &amp; learning)
        </button>
        {dataMsg && <div className="text-xs text-accent">{dataMsg}</div>}
        <p className="text-[10px] text-slate-500">
          Reset keeps your API keys and settings. Import replaces current data. Export saves a JSON
          backup to Downloads.
        </p>
      </div>
    </div>
  );
}

function KeyInput({
  label,
  set,
  value,
  onChange,
}: {
  label: string;
  set: boolean;
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <div>
      <label className="text-xs text-slate-400">
        {label} {set && <span className="text-accent">(set)</span>}
      </label>
      <input
        type="password"
        className="w-full mt-1 rounded-lg bg-ink border border-edge px-3 py-2 text-sm"
        placeholder={set ? "•••••• (leave blank to keep)" : "paste key"}
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
    </div>
  );
}
