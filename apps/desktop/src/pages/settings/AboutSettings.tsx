import {
  Row,
  SettingsGroup,
  SettingsHeader,
  Toggle,
} from "../../components/forms";
import { UiButton } from "../../components/mantine";
import { desktopBridge } from "../../app/native";
import { useAppUpdate } from "../../app/useDesktopUpdates";
import { workspaceStore } from "../../store/workspace";
import type { Workspace } from "../../lib/model";
import { t } from "../../i18n";

export function AboutSettings({
  data,
  update,
}: {
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
}) {
  const { status, pending, check } = useAppUpdate();
  const labels = {
    idle: "Check for updates",
    checking: "Checking for updates…",
    current: "You're up to date",
    available: "Update available",
    downloading: "Downloading update…",
    ready: "Update ready to install",
    error: "Update check failed",
  };
  return (
    <>
      <SettingsHeader title="About" />
      <SettingsGroup>
        <Row id="version" group title="Version">
          <span className="aboutValue">{__APP_VERSION__}</span>
        </Row>
        {desktopBridge?.checkAppUpdate && (
          <>
            <Row id="updates" group title="App updates">
              <span role="status">
                {t(
                  pending
                    ? "Checking for updates…"
                    : labels[status?.phase ?? "idle"],
                )}
                {status?.version ? ` · ${status.version}` : ""}
              </span>
              <UiButton disabled={pending} onClick={() => void check(true)}>
                {t("Check for updates")}
              </UiButton>
              {status?.phase === "available" && (
                <UiButton
                  onClick={() => {
                    if (status.signedUpdates) void check(true, true);
                    else void desktopBridge?.openExternal(status.releaseUrl);
                  }}
                >
                  {t(
                    status.signedUpdates
                      ? "Download update"
                      : "Download from releases",
                  )}
                </UiButton>
              )}
              {status?.phase === "ready" && (
                <UiButton
                  onClick={async () => {
                    await workspaceStore.flush();
                    await desktopBridge?.installAppUpdate?.();
                  }}
                >
                  {t("Install and quit")}
                </UiButton>
              )}
            </Row>
            <Row
              id="automatic-updates"
              title="Automatic updates"
              hint="Download in the background and install when the app quits."
            >
              <Toggle
                checked={data.settings.automaticAppUpdates ?? false}
                disabled={!status?.signedUpdates}
                onChange={(value) =>
                  update((draft) => {
                    draft.settings.automaticAppUpdates = value;
                    return draft;
                  })
                }
              />
            </Row>
            {status && !status.signedUpdates && (
              <p className="settingsHint">
                {t("Automatic installation requires an update-enabled build.")}
              </p>
            )}
          </>
        )}
      </SettingsGroup>
    </>
  );
}
