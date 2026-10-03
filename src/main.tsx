import React from "react";
import { createRoot } from "react-dom/client";
import "@mantine/core/styles.css";
import "@mantine/notifications/styles.css";
import App from "./app/App";
import { PageErrorBoundary } from "./app/PageErrorBoundary";
import "./styles.css";
createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <PageErrorBoundary name="app">
      <App />
    </PageErrorBoundary>
  </React.StrictMode>,
);
