// Small inline loading spinner. Use inside buttons/labels: <Spinner /> Building…
export default function Spinner({ className = "" }: { className?: string }) {
  return (
    <svg
      className={`inline-block animate-spin ${className}`}
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="3" opacity="0.25" />
      <path d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
    </svg>
  );
}
