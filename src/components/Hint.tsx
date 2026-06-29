import { useState } from "react";

// A small "?" that reveals a one-line explanation — for the jargon the app uses
// (λ, EV, Kelly, proxy xG, Dixon-Coles…) without cluttering the layout. Click to
// toggle (touch-friendly); `title` also gives a hover tooltip on desktop.
export default function Hint({ text, className = "" }: { text: string; className?: string }) {
  const [open, setOpen] = useState(false);
  return (
    <span className={`relative inline-block ${className}`}>
      <button
        type="button"
        title={text}
        aria-label="help"
        onClick={(e) => {
          e.stopPropagation();
          e.preventDefault();
          setOpen((v) => !v);
        }}
        className="ml-1 inline-flex h-3.5 w-3.5 items-center justify-center rounded-full border border-slate-600 text-[8px] font-bold text-slate-400 align-middle hover:text-slate-100 hover:border-slate-400"
      >
        ?
      </button>
      {open && (
        <>
          <span className="fixed inset-0 z-40" onClick={(e) => { e.stopPropagation(); setOpen(false); }} />
          <span className="absolute z-50 left-0 top-5 w-60 rounded-lg border border-edge bg-ink p-2 text-[11px] font-normal normal-case tracking-normal leading-snug text-slate-300 shadow-lg">
            {text}
          </span>
        </>
      )}
    </span>
  );
}
