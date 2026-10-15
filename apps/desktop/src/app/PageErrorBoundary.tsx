import { Component, type ReactNode } from "react";

/**
 * One page failing to render shows its error here instead of blanking the
 * whole window; the other pages keep working.
 */
export class PageErrorBoundary extends Component<
  { name: string; children: ReactNode },
  { error?: Error }
> {
  state: { error?: Error } = {};
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  componentDidCatch(error: Error, info: { componentStack?: string | null }) {
    console.error(`[${this.props.name}] page crashed`, error, info.componentStack);
  }
  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div className="pageError" role="alert">
        <b>This page hit an error.</b>
        <code>{error.message}</code>
        <button type="button" className="button" onClick={() => this.setState({ error: undefined })}>
          Try again
        </button>
      </div>
    );
  }
}
