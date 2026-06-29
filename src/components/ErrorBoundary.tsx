import { Component, type ReactNode } from "react";

// Without this, ONE component throwing during render blanks the whole app
// (white screen). This catches it, shows the actual error, and lets you recover
// without losing the session.
export default class ErrorBoundary extends Component<
  { children: ReactNode },
  { error: Error | null }
> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error) {
    // Surface it in the dev console too.
    console.error("UI crash:", error);
  }

  render() {
    if (this.state.error) {
      return (
        <div className="p-4">
          <div className="card border-bad/60 space-y-2 max-w-lg mx-auto mt-10">
            <div className="text-bad font-bold">Something broke on this screen</div>
            <div className="text-xs text-slate-400 break-words font-mono">
              {this.state.error.message || String(this.state.error)}
            </div>
            <p className="text-[11px] text-slate-500">
              Your data is safe. Tap below to recover — and please send this message so it can be fixed.
            </p>
            <button
              className="btn btn-primary text-sm"
              onClick={() => {
                this.setState({ error: null });
              }}
            >
              Dismiss &amp; continue
            </button>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}
