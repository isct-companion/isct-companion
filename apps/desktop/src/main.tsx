import React from "react";
import { createRoot } from "react-dom/client";
import { PageErrorBoundary } from "./app/PageErrorBoundary";
import { installInteractionTrace } from "./app/interaction-trace";
import "./styles.css";
// Opt in with VITE_INTERACTION_TRACE=1 yarn desktop:dev (see app/interaction-trace.ts).
if (import.meta.env.DEV && import.meta.env.VITE_INTERACTION_TRACE)
  installInteractionTrace();
const root = createRoot(document.getElementById("root")!);
// A window that shows one view loads only that view, never the app and
// the work its modules start (`__ISCT_COMPANION_VIEW__`, set natively).
if (window.__ISCT_COMPANION_VIEW__ === "campus-print")
  void import("./features/campus-print/CampusPrintWindow").then(
    ({ CampusPrintWindow }) =>
      root.render(
        <React.StrictMode>
          <PageErrorBoundary name="campus-print">
            <CampusPrintWindow />
          </PageErrorBoundary>
        </React.StrictMode>,
      ),
  );
else
  void import("./app/App").then(({ default: App }) =>
    root.render(
      <React.StrictMode>
        <PageErrorBoundary name="app">
          <App />
        </PageErrorBoundary>
      </React.StrictMode>,
    ),
  );
