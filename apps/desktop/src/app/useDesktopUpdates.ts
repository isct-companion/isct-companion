import { useEffect } from "react";
import { create } from "zustand";
import { notifications } from "./notifications";
import { desktopBridge } from "./native";
import { chooseAction } from "../components/confirm";
import { workspaceActions, workspaceStore } from "../store/workspace";
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
      return t("app.updatedToVersion", { version: last.version });
    case "incomplete":
      return t("app.theUpdateToVersionDid", { version: last.version });
    case "failed":
      return t("app.installingVersionFailedMessage", {
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

/** Downloads the version on offer if needed, then installs it and quits. */
async function installNow(status: AppUpdateStatus) {
  if (status.phase !== "ready") {
    if (!status.signedUpdates) {
      void desktopBridge?.openExternal(status.releaseUrl);
      return;
    }
    await useAppUpdate.getState().check(true, true);
    if (useAppUpdate.getState().status?.phase !== "ready") return;
  }
  await workspaceStore.flush();
  await desktopBridge?.installAppUpdate?.();
}

// Each version is offered once per launch; skipping it lasts (Core).
const offered = new Set<string>();
/** The update prompt, when Core offers the version a check found: before the
 * download, or once it is ready when automatic updates fetched it. */
async function offerUpdate(
  status: AppUpdateStatus | undefined,
  automatic: boolean,
) {
  const version = status?.version;
  if (!status?.prompt || !version || offered.has(version)) return;
  const ready = status.phase === "ready";
  if (!ready && (status.phase !== "available" || automatic)) return;
  offered.add(version);
  const choice = await chooseAction(
    t(ready ? "appUpdate.prompt.ready" : "appUpdate.prompt.available", {
      version,
    }),
    [
      { id: "off", label: t("appUpdate.prompt.turnOff") },
      { id: "skip", label: t("appUpdate.prompt.skip") },
      {
        id: "update",
        label: t(
          ready ? "settings.about.installAndQuit" : "appUpdate.prompt.update",
        ),
      },
    ],
    { title: t("appUpdate.prompt.title") },
  );
  if (choice === "update") await installNow(status);
  if (choice === "skip" || choice === "off")
    await workspaceActions.updateSettings((draft) => {
      if (choice === "skip") draft.settings.skippedAppVersion = version;
      else draft.settings.appUpdateChecks = false;
      return draft;
    });
}

export function useDesktopUpdates(ready: boolean, automatic = false) {
  const status = useAppUpdate((state) => state.status);
  useEffect(() => {
    if (ready) void offerUpdate(status, automatic).catch(console.error);
  }, [ready, status, automatic]);
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
