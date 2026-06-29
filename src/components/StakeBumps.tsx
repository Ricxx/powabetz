// Quick-bump buttons for the place box, so you never type a stake. Matches a
// flat-staking workflow (e.g. start at $0.50, nudge +25c / +50c / +$1 per ticket).
export default function StakeBumps({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const cur = parseFloat(value) || 0;
  const bump = (d: number) => onChange(Math.max(0, cur + d).toFixed(2));
  return (
    <div className="flex gap-1">
      {([0.25, 0.5, 1] as const).map((d) => (
        <button
          key={d}
          type="button"
          className="chip text-[10px] px-1.5 py-0.5"
          onClick={() => bump(d)}
          title={`Add $${d.toFixed(2)}`}
        >
          +{d === 1 ? "$1" : `${d * 100}¢`}
        </button>
      ))}
    </div>
  );
}
