import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./styles/base.css";
import { App } from "./App";
import { CrashBoundary } from "./components/shell/CrashBoundary";
import { runSelftestIfRequested } from "./selftest";
import { captureUncaughtErrors } from "./state/crashReports";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root element");

// Errors nothing else handled become crash reports (ADR 0061).
captureUncaughtErrors();

createRoot(root).render(
  <StrictMode>
    <CrashBoundary>
      <App />
    </CrashBoundary>
  </StrictMode>,
);

void runSelftestIfRequested();
