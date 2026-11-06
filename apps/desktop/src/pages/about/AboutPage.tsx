import {
  Row,
  SettingsGroup,
  SettingsHeader,
  Toggle,
} from "../../components/forms";
import { UiButton } from "../../components/ui";
import { useResetGuide } from "../../components/guide";
import { desktopBridge } from "../../app/native";
import { lastInstallText, useAppUpdate } from "../../app/useDesktopUpdates";
import { workspaceStore } from "../../store/workspace";
import type { Workspace } from "../../lib/model";
import { t, type MessageKey } from "../../i18n";
import "../settings/settings.css";

const WEBSITE = "https://isctcomp.app/";
const REPOSITORY = "https://github.com/isct-companion/isct-companion";
const LICENSE = `${REPOSITORY}/blob/main/LICENSE`;
const SPONSOR = "https://isctcomp.app/sponsor/";

/** A row that opens `url` in the system browser. */
function LinkRow({
  id,
  title,
  titleText,
  hint,
  action = "about.open",
  url,
}: {
  id: string;
  title?: MessageKey;
  titleText?: string;
  hint: string;
  action?: MessageKey;
  url: string;
}) {
  return (
    <Row id={id} group title={title} titleText={titleText} hint={hint}>
      <UiButton
        className="button small"
        onClick={() => void desktopBridge?.openExternal(url)}
      >
        {t(action)}
      </UiButton>
    </Row>
  );
}

/** The app itself: its version and updates, where it lives, its license and
 * how to support it. A page of its own, under Settings in the rail. */
export function About({
  data,
  update,
}: {
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
}) {
  const { status, pending, check } = useAppUpdate();
  const resetGuide = useResetGuide();
  const labels = {
    idle: "settings.about.checkForUpdates",
    unpublished: "settings.about.noStableReleaseHasBeen",
    checking: "settings.about.checkingForUpdates",
    current: "settings.about.youreUpToDate",
    available: "settings.about.updateAvailable",
    downloading: "settings.about.downloadingUpdate",
    ready: "settings.about.updateReadyToInstall",
    error: "settings.about.updateCheckFailed",
  } satisfies Record<string, MessageKey>;
  const megabytes = (bytes: number) => (bytes / 1024 / 1024).toFixed(1);
  const progress =
    status?.phase === "downloading" && status.progress
      ? ` · ${megabytes(status.progress.downloaded)}${
          status.progress.total ? ` / ${megabytes(status.progress.total)}` : ""
        } MB`
      : "";
  return (
    <div className="page settings">
      <section className="settingsPane">
        <SettingsHeader title="settings.main.about" />
        <SettingsGroup>
          <Row id="version" group title="settings.about.version">
            <span className="aboutValue">{__APP_VERSION__}</span>
          </Row>
          {desktopBridge?.checkAppUpdate && (
            <>
              <Row
                id="updates"
                group
                title="settings.about.appUpdates"
                hint={
                  <span role="status">
                    {t(
                      labels[status?.phase ?? (pending ? "checking" : "idle")],
                    )}
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
                    {t("settings.about.checkForUpdates")}
                  </UiButton>
                  {status?.phase === "available" && (
                    <UiButton
                      className="button small primary"
                      onClick={() => {
                        if (status.signedUpdates) void check(true, true);
                        else
                          void desktopBridge?.openExternal(status.releaseUrl);
                      }}
                    >
                      {t(
                        status.signedUpdates
                          ? "settings.about.downloadUpdate"
                          : "settings.about.downloadFromReleases",
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
                      {t("settings.about.installAndQuit")}
                    </UiButton>
                  )}
                </div>
              </Row>
              <Row
                id="automatic-updates"
                title="settings.about.automaticUpdates"
                hint={t("settings.about.downloadInTheBackgroundAnd")}
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
                  {t("settings.about.automaticInstallationRequiresAnUpdate")}
                </p>
              )}
            </>
          )}
          <Row id="guide" title="settings.main.showWelcomeAndTipsAgain">
            <UiButton className="button small" onClick={resetGuide}>
              {t("settings.about.showAgain")}
            </UiButton>
          </Row>
        </SettingsGroup>
        <SettingsGroup>
          <LinkRow
            id="website"
            title="about.website"
            hint="isctcomp.app"
            url={WEBSITE}
          />
          <LinkRow
            id="github"
            titleText="GitHub"
            hint="isct-companion/isct-companion"
            url={REPOSITORY}
          />
        </SettingsGroup>
        <SettingsGroup>
          <LinkRow
            id="license"
            title="about.license"
            hint="GNU GPL v3.0"
            url={LICENSE}
          />
          <LinkRow
            id="sponsor"
            title="about.sponsor"
            hint={t("about.sponsorHint")}
            action="about.sponsorOpen"
            url={SPONSOR}
          />
        </SettingsGroup>
      </section>
    </div>
  );
}
