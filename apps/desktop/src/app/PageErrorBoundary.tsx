import { Component, type ReactNode } from "react";
import { traceEvent } from "./interaction-trace";
import { t } from "../i18n";
import { ErrorBanner } from "../components/ui";

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
    console.error(
      `[${this.props.name}] page crashed`,
      error,
      info.componentStack,
    );
    traceEvent("error", {
      source: `page ${this.props.name}`,
      message: error.message,
      stack: error.stack?.split("\n").slice(0, 12).join("\n"),
      componentStack: info.componentStack?.split("\n").slice(0, 25).join("\n"),
    });
  }
  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <ErrorBanner
        className="pageError"
        title={error.message}
        onRetry={() => this.setState({ error: undefined })}
      >
        {t("This page hit an error.")}
      </ErrorBanner>
    );
  }
}
