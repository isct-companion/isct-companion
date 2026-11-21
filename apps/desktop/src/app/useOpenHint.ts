import { useEffect } from "react";
import type { SyncView } from "../generated/persistence";
import { sendHint } from "../platform/sync-intents";

/** Tells Core a view became visible. Re-sent when the view changes or is
 * shown again; Core's open threshold keeps repeats cheap. */
export function useOpenHint(view: SyncView | null) {
  const key = view ? JSON.stringify(view) : "";
  useEffect(() => {
    if (key) sendHint({ kind: "open", view: JSON.parse(key) as SyncView });
  }, [key]);
}
