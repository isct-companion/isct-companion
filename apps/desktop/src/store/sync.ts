import type { SyncSnapshot } from "../generated/persistence";
import { useSyncExternalStore } from "react";
import { desktopBridge } from "../platform/native";
import { EMPTY_SYNC } from "../platform/sync";

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
