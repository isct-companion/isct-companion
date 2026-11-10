import { useEffect, useSyncExternalStore } from "react";
import { desktopBridge } from "./native";
import {
  EMPTY_SYNC,
  type SyncHint,
  type SyncScope,
  type SyncSnapshot,
  type SyncView,
} from "../platform/sync";

// One subscription for the whole shell: every surface reads the same
// snapshot, which the bridge seeds with `sync_snapshot` and then replaces on
// each `sync-snapshot` event.
let latest: SyncSnapshot = EMPTY_SYNC;
const listeners = new Set<() => void>();
let dispose: (() => void) | undefined;

function subscribe(listener: () => void) {
  listeners.add(listener);
  dispose ??= desktopBridge?.onSyncSnapshot?.((snapshot) => {
    latest = snapshot;
    for (const notify of listeners) notify();
  });
  return () => {
    listeners.delete(listener);
    if (listeners.size) return;
    dispose?.();
    dispose = undefined;
    latest = EMPTY_SYNC;
  };
}

/** Core's latest sync snapshot. */
export function useSync(): SyncSnapshot {
  return useSyncExternalStore(subscribe, () => latest);
}

/** Forwards a fact or intent; Core decides what follows. */
export function sendHint(hint: SyncHint) {
  void desktopBridge?.syncHint?.(hint).catch(() => undefined);
}

export const syncAvailable = () => Boolean(desktopBridge?.syncHint);

export const refresh = (scope: SyncScope) =>
  sendHint({ kind: "refresh", scope });

/** Tells Core a view became visible. Re-sent when the view changes or is
 * shown again; Core's open threshold keeps repeats cheap. */
export function useOpenHint(view: SyncView | null) {
  const key = view ? JSON.stringify(view) : "";
  useEffect(() => {
    if (key) sendHint({ kind: "open", view: JSON.parse(key) as SyncView });
  }, [key]);
}
