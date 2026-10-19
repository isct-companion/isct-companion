import { useEffect } from "react";
import { create } from "zustand";
import { notifications } from "@mantine/notifications";
import { desktopBridge } from "./native";
import { t } from "../i18n";
import type {
  AppLastInstall,
  AppUpdateStatus,
} from "../platform/desktop-bridge";

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

export function lastInstallText(last: AppLastInstall) {
  switch (last.outcome) {
    case "installed":
      return t("Updated to {{version}}.", { version: last.version });
    case "incomplete":
      return t(
        "The update to {{version}} did not install. Download it from the releases page.",
        { version: last.version },
      );
    case "failed":
      return t("Installing {{version}} failed: {{message}}", {
        version: last.version,
        message: last.message,
      });
  }
}

let lastInstallShown = false;
function announceLastInstall(status?: AppUpdateStatus) {
  const last = status?.lastInstall;
  if (!last || lastInstallShown) return;
  lastInstallShown = true;
  notifications.show({
    id: "app-update-result",
    message: lastInstallText(last),
    color: last.outcome === "installed" ? undefined : "red",
    autoClose: last.outcome === "installed" ? 4000 : false,
    withCloseButton: true,
  });
}

export function useDesktopUpdates(ready: boolean, automatic = false) {
  useEffect(() => {
    if (!ready || !desktopBridge?.checkAppUpdate) return;
    void desktopBridge
      .appUpdateState?.()
      .then(announceLastInstall)
      .catch(() => undefined);
    void useAppUpdate.getState().check();
    // The shell supplies a wakeup; Core decides whether a check is due.
    const timer = window.setInterval(
      () => void useAppUpdate.getState().check(),
      60 * 60 * 1000,
    );
    return () => window.clearInterval(timer);
  }, [ready, automatic]);
}
