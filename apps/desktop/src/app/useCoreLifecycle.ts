import { useEffect } from "react";
import { desktopBridge } from "./native";
import { sendHint } from "./useSync";
import { networkHint } from "../platform/sync";
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
    const focus = () => {
      if (document.visibilityState !== "hidden") report("foreground");
    };
    const visibility = () =>
      report(
        document.visibilityState === "hidden" ? "background" : "foreground",
      );
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
    addEventListener("focus", focus);
    document.addEventListener("visibilitychange", visibility);
    return () => {
      removeEventListener("focus", focus);
      removeEventListener("online", network);
      removeEventListener("offline", network);
      connection?.removeEventListener?.("change", network);
      document.removeEventListener("visibilitychange", visibility);
    };
  }, []);
}
