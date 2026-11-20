import { useEffect } from "react";
import { notifications } from "../components/notifications/notifications";
import { desktopBridge } from "../platform/native";
import { chooseAction } from "../components/confirm";
import { workspaceActions, workspaceStore } from "../store/workspace";
import { t } from "../i18n";
import type { AppUpdateStatus } from "../platform/desktop-bridge";
import { useAppUpdate } from "../store/app-update";
import { lastInstallText } from "../lib/app-update-text";

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
