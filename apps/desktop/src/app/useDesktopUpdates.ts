import { useEffect } from "react";
import { create } from "zustand";
import { desktopBridge } from "./native";
import type { AppUpdateStatus } from "../platform/desktop-bridge";

// Transport state only: the native updater owns checks, verified bytes and install.
export const useAppUpdate = create<{
  status?: AppUpdateStatus;
  pending: boolean;
  check: (manual?: boolean, download?: boolean) => Promise<void>;
}>((set, get) => ({
  pending: false,
  check: async (manual = false, download = false) => {
    if (!desktopBridge?.checkAppUpdate || get().pending) return;
    set({ pending: true });
    try {
      set({ status: await desktopBridge.checkAppUpdate(manual, download) });
    } catch (error) {
      const status = await desktopBridge
        .appUpdateState?.()
        .catch(() => undefined);
      if (status) set({ status });
      else
        set({
          status: {
            phase: "error",
            message: String(error),
            signedUpdates: false,
            releaseUrl:
              "https://github.com/isct-companion/isct-companion/releases/latest",
          },
        });
    } finally {
      set({ pending: false });
    }
  },
}));

export function useDesktopUpdates(ready: boolean, automatic = false) {
  useEffect(() => {
    if (!ready || !desktopBridge?.checkAppUpdate) return;
    void useAppUpdate.getState().check();
    // The shell supplies a wakeup; Core decides whether a check is due.
    const timer = window.setInterval(
      () => void useAppUpdate.getState().check(),
      60 * 60 * 1000,
    );
    return () => window.clearInterval(timer);
  }, [ready, automatic]);
}
