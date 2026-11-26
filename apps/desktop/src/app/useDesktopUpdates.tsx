import { useEffect } from "react";
import { notify } from "../components/notifications/notify";
import { desktopBridge } from "../platform/native";
import { MessageDialog, type AutomaticDialog } from "../components/confirm";
import { workspaceActions, workspaceStore } from "../store/workspace";
import { t } from "../i18n";
import type { AppUpdateStatus } from "../platform/desktop-bridge";
import { useAppUpdate } from "../store/app-update";
import { lastInstallText } from "../lib/app-update-text";

let installReported = false;

async function install(status: AppUpdateStatus, version: string) {
  if (!status.signedUpdates) {
    await desktopBridge?.openExternal(status.releaseUrl);
    return;
  }
  if (status.phase !== "ready")
    await useAppUpdate.getState().check(true, true, version);
  const current = useAppUpdate.getState().status;
  if (current?.version !== version || current.phase !== "ready") return;
  await workspaceStore.flush();
  await desktopBridge?.installAppUpdate?.(version);
}

export function useDesktopUpdates(
  ready: boolean,
  automatic = false,
): AutomaticDialog | undefined {
  const status = useAppUpdate((state) => state.status);
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

  useEffect(() => {
    if (status?.lastInstall?.outcome !== "installed" || installReported) return;
    installReported = true;
    notify.info(lastInstallText(status.lastInstall));
  }, [status?.lastInstall]);

  const version = status?.version;
  if (!ready || !status?.prompt || !version) return;
  const downloaded = status.phase === "ready";
  if (!downloaded && (status.phase !== "available" || automatic)) return;
  const request = {
    title: t("appUpdate.prompt.title"),
    message: t(
      downloaded ? "appUpdate.prompt.ready" : "appUpdate.prompt.available",
      { version },
    ),
    choices: [
      { id: "off", label: t("appUpdate.prompt.turnOff") },
      { id: "skip", label: t("appUpdate.prompt.skip") },
      {
        id: "update",
        label: t(
          downloaded
            ? "settings.about.installAndQuit"
            : "appUpdate.prompt.update",
        ),
      },
    ],
  };
  return {
    id: `update:${version}`,
    render: (dismiss) => (
      <MessageDialog
        request={request}
        finish={(choice) => {
          dismiss();
          if (choice === "update")
            void install(status, version).catch(notify.error);
          if (choice === "skip" || choice === "off")
            void workspaceActions
              .updateSettings((draft) => {
                if (choice === "skip")
                  draft.settings.skippedAppVersion = version;
                else draft.settings.appUpdateChecks = false;
                return draft;
              })
              .catch(notify.error);
        }}
      />
    ),
  };
}
