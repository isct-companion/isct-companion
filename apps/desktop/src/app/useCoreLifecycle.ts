import { useEffect } from "react";
import { desktopBridge } from "./native";
import { subscriptionNetwork } from "../platform/subscription-polling";
import { detectPlatform } from "../platform/capabilities";

export function useCoreLifecycle() {
  useEffect(() => {
    if (!desktopBridge?.schoolRefreshEvent) return;
    const report = (kind: "foreground" | "background") => {
      void desktopBridge
        ?.schoolRefreshEvent?.({
          kind,
          network: subscriptionNetwork(detectPlatform()),
        })
        .catch(() => undefined);
      void desktopBridge?.cloudLifecycle?.(kind).catch(() => undefined);
    };
    const focus = () => {
      if (document.visibilityState !== "hidden") report("foreground");
    };
    const visibility = () =>
      report(
        document.visibilityState === "hidden" ? "background" : "foreground",
      );
    focus();
    addEventListener("focus", focus);
    addEventListener("online", focus);
    addEventListener("offline", visibility);
    document.addEventListener("visibilitychange", visibility);
    return () => {
      removeEventListener("focus", focus);
      removeEventListener("online", focus);
      removeEventListener("offline", visibility);
      document.removeEventListener("visibilitychange", visibility);
    };
  }, []);
}
