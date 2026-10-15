import React from "react";
import { createRoot } from "react-dom/client";
import "@mantine/core/styles.css";
import "@mantine/notifications/styles.css";
import App from "./app/App";
import { PageErrorBoundary } from "./app/PageErrorBoundary";
import "./styles.css";
const root = createRoot(document.getElementById("root")!);
if (
  import.meta.env.DEV &&
  new URLSearchParams(location.search).get("preview") === "lms"
) {
  void import("./features/lms/LmsPreview").then(({ LmsPreview }) =>
    root.render(<LmsPreview />),
  );
} else
  root.render(
    <React.StrictMode>
      <PageErrorBoundary name="app">
        <App />
      </PageErrorBoundary>
    </React.StrictMode>,
  );
