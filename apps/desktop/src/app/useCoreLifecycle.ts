import { useEffect } from "react";
import { desktopBridge } from "./native";
import { sendHint } from "./useSync";
import { networkHint } from "../platform/sync";

/** Shell facts for Core: visibility and the network link. The native side
 * reports window focus and OS resume itself. */
export function useCoreLifecycle() {
  useEffect(() => {
    if (!desktopBridge?.syncHint) return;
    const network = () => sendHint({ kind: "network", network: networkHint() });
    const report = (kind: "foreground" | "background") => {
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
    network();
    focus();
    const connection = (
      navigator as Navigator & {
        connection?: {
          addEventListener?: (type: "change", listener: () => void) => void;
          removeEventListener?: (type: "change", listener: () => void) => void;
        };
      }
    ).connection;
    addEventListener("focus", focus);
    addEventListener("online", network);
    addEventListener("offline", network);
    connection?.addEventListener?.("change", network);
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
