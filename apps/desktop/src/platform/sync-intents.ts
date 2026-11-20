import { desktopBridge } from "./native";
import type { SyncHint, SyncScope } from "./sync";

/** Forwards a fact or intent; Core decides what follows. */
export function sendHint(hint: SyncHint) {
  void desktopBridge?.syncHint?.(hint).catch(() => undefined);
}

export const syncAvailable = () => Boolean(desktopBridge?.syncHint);

export const refresh = (scope: SyncScope) =>
  sendHint({ kind: "refresh", scope });
