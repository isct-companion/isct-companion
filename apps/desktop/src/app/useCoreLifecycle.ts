import { useEffect } from "react";
import { desktopBridge } from "../platform/native";
import { sendHint } from "../platform/sync-intents";
import { networkHint } from "../platform/sync";
import { everythingChanged } from "../platform/query-client";
import { resyncClock } from "../platform/use-clock";

/** Shell facts for Core: visibility and the network link. The native side
 * reports window focus and OS resume itself. Coming back to the foreground
 * also resyncs the shell clock, whose timer may not have fired while hidden. */
export function useCoreLifecycle() {
  useEffect(() => {
    const bridged = Boolean(desktopBridge?.syncHint);
    const network = () => sendHint({ kind: "network", network: networkHint() });
    const report = (kind: "foreground" | "background") => {
      if (kind === "foreground") resyncClock();
      if (!bridged) return;
      sendHint({ kind });
      void desktopBridge?.cloudLifecycle?.(kind).catch(() => undefined);
    };
    let away = document.visibilityState === "hidden" || !document.hasFocus();
    const leave = () => {
      away = true;
    };
    const restore = () => {
      if (away) everythingChanged();
      away = false;
      report("foreground");
    };
    const focus = () => {
      if (document.visibilityState !== "hidden") restore();
    };
    const visibility = () => {
      if (document.visibilityState === "visible") restore();
      else {
        leave();
        report("background");
      }
    };
    const connection = (
      navigator as Navigator & {
        connection?: {
          addEventListener?: (type: "change", listener: () => void) => void;
          removeEventListener?: (type: "change", listener: () => void) => void;
        };
      }
    ).connection;
    if (bridged) {
      network();
      focus();
      addEventListener("online", network);
      addEventListener("offline", network);
      connection?.addEventListener?.("change", network);
    }
    addEventListener("blur", leave);
    addEventListener("focus", focus);
    document.addEventListener("visibilitychange", visibility);
    return () => {
      removeEventListener("blur", leave);
      removeEventListener("focus", focus);
      removeEventListener("online", network);
      removeEventListener("offline", network);
      connection?.removeEventListener?.("change", network);
      document.removeEventListener("visibilitychange", visibility);
    };
  }, []);
}
