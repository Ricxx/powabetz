// Global feedback layer: one toast store the whole app shares, so every async
// action can report success/failure consistently instead of swallowing errors
// or dumping raw exceptions. Module-level (no context threading) — any file can
// `import { toast }` and call it; <Toaster/> is mounted once in App.

import { useEffect, useState } from "react";

export type ToastKind = "success" | "error" | "info";
export interface Toast {
  id: number;
  kind: ToastKind;
  msg: string;
  action?: { label: string; onClick: () => void };
  duration: number;
}

let nextId = 1;
let toasts: Toast[] = [];
const listeners = new Set<(t: Toast[]) => void>();

function emit() {
  for (const l of listeners) l([...toasts]);
}

function remove(id: number) {
  toasts = toasts.filter((t) => t.id !== id);
  emit();
}

function push(kind: ToastKind, msg: string, opts?: { action?: Toast["action"]; duration?: number }): number {
  const id = nextId++;
  const duration = opts?.duration ?? (kind === "error" ? 6000 : 3500);
  toasts = [...toasts, { id, kind, msg, action: opts?.action, duration }];
  emit();
  return id;
}

/** Turn a thrown value / backend string into a short, human message. */
export function errMsg(e: unknown): string {
  let s = e instanceof Error ? e.message : String(e ?? "");
  s = s.replace(/^Error:\s*/i, "").trim();
  if (!s) return "Something went wrong.";
  const map: [RegExp, string][] = [
    [/daily request (budget|limit|meter)|meter.*(100|reached|block)/i, "Daily request budget reached — try again tomorrow or raise the limit in Settings."],
    [/\b(401|403|invalid|unauthor|forbidden).*(key|auth)|missing.*key|no api key/i, "API key missing or rejected — check it in Settings."],
    [/\b429\b|rate limit/i, "Rate-limited by the provider — wait a moment and retry."],
    [/timed? ?out|timeout/i, "The request timed out — check your connection and retry."],
    [/network|connection|dns|unreachable|failed to fetch/i, "Network problem — couldn't reach the service."],
    [/not allowed for analysis/i, "That model can't be used here — pick another in the model selector."],
    [/no (live )?markets|no usable legs/i, "Nothing to build from yet — try refreshing."],
  ];
  for (const [re, friendly] of map) if (re.test(s)) return friendly;
  // Cap very long raw strings so the UI never shows a stack trace.
  return s.length > 160 ? s.slice(0, 157) + "…" : s;
}

export const toast = {
  success: (msg: string) => push("success", msg),
  error: (e: unknown) => push("error", typeof e === "string" ? e : errMsg(e)),
  info: (msg: string) => push("info", msg),
  /** Show a message with a 5s "Undo" — runs `onUndo` if tapped. */
  undo: (msg: string, onUndo: () => void) => {
    let id = 0;
    id = push("info", msg, {
      duration: 5000,
      action: { label: "Undo", onClick: () => { onUndo(); remove(id); } },
    });
    return id;
  },
  dismiss: remove,
};

function useToasts(): Toast[] {
  const [list, setList] = useState<Toast[]>(toasts);
  useEffect(() => {
    listeners.add(setList);
    return () => { listeners.delete(setList); };
  }, []);
  return list;
}

const STYLE: Record<ToastKind, string> = {
  success: "border-accent/60 text-accent",
  error: "border-bad/60 text-bad",
  info: "border-edge text-slate-200",
};
const ICON: Record<ToastKind, string> = { success: "✓", error: "✕", info: "•" };

export function Toaster() {
  const list = useToasts();
  return (
    <div className="fixed inset-x-0 bottom-4 z-[60] flex flex-col items-center gap-2 px-4 pointer-events-none">
      {list.map((t) => (
        <ToastRow key={t.id} t={t} />
      ))}
    </div>
  );
}

function ToastRow({ t }: { t: Toast }) {
  useEffect(() => {
    const h = setTimeout(() => remove(t.id), t.duration);
    return () => clearTimeout(h);
  }, [t.id, t.duration]);
  return (
    <div
      className={`pointer-events-auto w-full max-w-md card border ${STYLE[t.kind]} bg-ink/95 backdrop-blur shadow-lg flex items-center gap-3 text-sm animate-[fadeIn_120ms_ease-out]`}
    >
      <span className="shrink-0 font-bold">{ICON[t.kind]}</span>
      <span className="flex-1 text-slate-100">{t.msg}</span>
      {t.action && (
        <button
          className="shrink-0 underline font-semibold text-accent"
          onClick={() => t.action!.onClick()}
        >
          {t.action.label}
        </button>
      )}
      <button className="shrink-0 text-slate-500 hover:text-slate-200" onClick={() => remove(t.id)} aria-label="dismiss">
        ✕
      </button>
    </div>
  );
}
