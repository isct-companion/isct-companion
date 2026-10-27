import React from "react";
import { createRoot } from "react-dom/client";
import App from "./app/App";
import { PageErrorBoundary } from "./app/PageErrorBoundary";
import { installInteractionTrace } from "./app/interaction-trace";
import "./styles.css";
// Opt in with VITE_INTERACTION_TRACE=1 yarn desktop:dev (see app/interaction-trace.ts).
if (import.meta.env.DEV && import.meta.env.VITE_INTERACTION_TRACE)
  installInteractionTrace();
createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <PageErrorBoundary name="app">
      <App />
    </PageErrorBoundary>
  </React.StrictMode>,
);
