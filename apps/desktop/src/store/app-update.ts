import { create } from "zustand";
import { desktopBridge } from "../platform/native";
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
    // The command resolves only after the download; read the native phase and
    // byte count meanwhile so the panel does not sit on "Checking…".
    const poll = window.setInterval(() => {
      void desktopBridge
        ?.appUpdateState?.()
        .then((status) => {
          if (get().pending) set({ status });
        })
        .catch(() => undefined);
    }, 500);
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
      window.clearInterval(poll);
      set({ pending: false });
    }
  },
}));
