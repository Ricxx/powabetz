import type { MatchForecast } from "../types";

// Match Predictor: a deterministic single-game forecast — each line is a % read
// straight off our engine (no model), grouped into sections. When the match is
// in-play a live "remaining ~X'" section is added on top by the backend.
export default function ForecastPanel({ f, footer }: { f: MatchForecast; footer?: string }) {
  const barColor = (p: number) => (p >= 60 ? "bg-accent" : p >= 40 ? "bg-warn" : "bg-slate-600");
  return (
    <div className="card border-accent/40 space-y-2">
      <div>
        <div className="text-sm font-bold">🔮 {f.home} vs {f.away}</div>
        <div className="text-[11px] text-accent">{f.headline}</div>
      </div>
      {f.sections.map((s, i) => (
        <div key={i} className="space-y-0.5">
          <div className="text-[10px] font-semibold uppercase tracking-wide text-slate-500">{s.title}</div>
          {s.lines.map((l, j) => (
            <div key={j} className="flex items-center gap-2 text-xs">
              <span className="text-slate-300 flex-1 min-w-0 truncate">{l.label}</span>
              <div className="w-20 h-1.5 rounded-full bg-edge overflow-hidden shrink-0">
                <div className={`h-full ${barColor(l.pct)}`} style={{ width: `${Math.min(100, l.pct)}%` }} />
              </div>
              <span className="text-slate-100 font-semibold w-9 text-right shrink-0">{Math.round(l.pct)}%</span>
            </div>
          ))}
        </div>
      ))}
      {footer && <p className="text-[10px] text-slate-500">{footer}</p>}
    </div>
  );
}
