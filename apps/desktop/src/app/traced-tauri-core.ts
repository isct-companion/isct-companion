/**
 * `@tauri-apps/api/core` as the app sees it under `vite serve`: the dev server
 * resolves every app import of that module here (scripts/desktop/
 * interaction-trace.mjs), so each invoke lands in the interaction trace. Tauri
 * defines `__TAURI_INTERNALS__.invoke` read-only, so the module is the only
 * place left to observe it. Production builds import the real module.
 */
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { tracedInvoke } from "./interaction-trace";

export * from "@tauri-apps/api/core";
export const invoke: typeof tauriInvoke = (cmd, args, options) =>
  tracedInvoke(cmd, args, () => tauriInvoke(cmd, args, options));
