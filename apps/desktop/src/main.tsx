import React from "react";
import { createRoot } from "react-dom/client";
import { PageErrorBoundary } from "./app/PageErrorBoundary";
import { installInteractionTrace } from "./app/interaction-trace";
import "./styles.css";
// Opt in with VITE_INTERACTION_TRACE=1 yarn desktop:dev (see app/interaction-trace.ts).
if (import.meta.env.DEV && import.meta.env.VITE_INTERACTION_TRACE)
  installInteractionTrace();
// The webview's own menu offers browser actions (Back, Reload, Save as,
// Print) that mean nothing in the app. Keep it where it edits or copies
// text, and in dev builds for Inspect Element. Our own menus (Radix
// ContextMenu) handle the event first, at the React root.
if (!import.meta.env.DEV)
  document.addEventListener("contextmenu", (event) => {
    const target = event.target as HTMLElement;
    const editable = target.closest("input, textarea, [contenteditable]");
    if (!editable && window.getSelection()?.isCollapsed !== false)
      event.preventDefault();
  });
const root = createRoot(document.getElementById("root")!);
void import("./app/App").then(({ default: App }) =>
  root.render(
    <React.StrictMode>
      <PageErrorBoundary name="app">
        <App />
      </PageErrorBoundary>
    </React.StrictMode>,
  ),
);
