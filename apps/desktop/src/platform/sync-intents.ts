import type { RetryPlan, SyncHint, SyncScope } from "../generated/persistence";
import { desktopBridge } from "./native";

/** Forwards a fact or intent; Core decides what follows. */
export function sendHint(hint: SyncHint) {
  void desktopBridge?.syncHint?.(hint).catch(() => undefined);
}

export const syncAvailable = () => Boolean(desktopBridge?.syncHint);

export const refresh = (scope: SyncScope) =>
  sendHint({ kind: "refresh", scope });

export function retry(plan: RetryPlan) {
  plan.scopes.forEach(refresh);
}
