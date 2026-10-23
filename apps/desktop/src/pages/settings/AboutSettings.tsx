import {
  Row,
  SettingsGroup,
  SettingsHeader,
  Toggle,
} from "../../components/forms";
import { UiButton } from "../../components/mantine";
import { useResetGuide } from "../../components/guide";
import { desktopBridge } from "../../app/native";
import { lastInstallText, useAppUpdate } from "../../app/useDesktopUpdates";
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
  const resetGuide = useResetGuide();
  const labels = {
    idle: "Check for updates",
    unpublished: "No stable release has been published yet",
    checking: "Checking for updates…",
    current: "You're up to date",
    available: "Update available",
    downloading: "Downloading update…",
    ready: "Update ready to install",
    error: "Update check failed",
  };
  const megabytes = (bytes: number) => (bytes / 1024 / 1024).toFixed(1);
  const progress =
    status?.phase === "downloading" && status.progress
      ? ` · ${megabytes(status.progress.downloaded)}${
          status.progress.total ? ` / ${megabytes(status.progress.total)}` : ""
        } MB`
      : "";
  return (
    <>
      <SettingsHeader title="About" />
      <SettingsGroup>
        <Row id="version" group title="Version">
          <span className="aboutValue">{__APP_VERSION__}</span>
        </Row>
        {desktopBridge?.checkAppUpdate && (
          <>
            <Row
              id="updates"
              group
              title="App updates"
              hint={
                <span role="status">
                  {t(labels[status?.phase ?? (pending ? "checking" : "idle")])}
                  {status?.version ? ` · ${status.version}` : ""}
                  {progress}
                  {status?.phase === "error" && status.message
                    ? ` · ${status.message}`
                    : ""}
                  {status?.lastInstall && (
                    <>
                      <br />
                      {lastInstallText(status.lastInstall)}
                    </>
                  )}
                </span>
              }
            >
              <div className="actions">
                <UiButton
                  className="button small"
                  disabled={pending}
                  onClick={() => void check(true)}
                >
                  {t("Check for updates")}
                </UiButton>
                {status?.phase === "available" && (
                  <UiButton
                    className="button small primary"
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
                    className="button small primary"
                    onClick={async () => {
                      await workspaceStore.flush();
                      await desktopBridge?.installAppUpdate?.();
                    }}
                  >
                    {t("Install and quit")}
                  </UiButton>
                )}
              </div>
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
        <Row id="guide" title="Show welcome and tips again">
          <UiButton className="button small" onClick={resetGuide}>
            {t("Show again")}
          </UiButton>
        </Row>
      </SettingsGroup>
    </>
  );
}
