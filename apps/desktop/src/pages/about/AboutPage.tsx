import {
  ArrowUpRight,
  GitBranch,
  Globe,
  Heart,
  Scale,
  type LucideIcon,
} from "lucide-react";
import { Row, Toggle } from "../../components/forms";
import { UiButton } from "../../components/ui";
import { useResetGuide } from "../../components/guide";
import { desktopBridge } from "../../app/native";
import { lastInstallText, useAppUpdate } from "../../app/useDesktopUpdates";
import { workspaceStore } from "../../store/workspace";
import type { Workspace } from "../../lib/model";
import { t, type MessageKey } from "../../i18n";
import "./about.css";

const WEBSITE = "https://isctcomp.app/";
const REPOSITORY = "https://github.com/isct-companion/isct-companion";
const LICENSE = `${REPOSITORY}/blob/main/LICENSE`;
const SPONSOR = "https://isctcomp.app/sponsor/";

/** A tile that opens `url` in the system browser. */
function LinkTile({
  id,
  title,
  hint,
  icon: Icon,
  action = "about.open",
  url,
}: {
  id: string;
  title: string;
  hint: string;
  icon: LucideIcon;
  action?: MessageKey;
  url: string;
}) {
  return (
    <UiButton
      className="aboutTile"
      data-setting={id}
      onClick={() => void desktopBridge?.openExternal(url)}
    >
      <Icon className="aboutTileIcon" aria-hidden strokeWidth={1.75} />
      <span className="aboutTileAction">
        <span>{t(action)}</span>
        <ArrowUpRight aria-hidden strokeWidth={2} />
      </span>
      <span className="aboutTileText">
        <b>{title}</b>
        <small>{hint}</small>
      </span>
    </UiButton>
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
    <div className="page">
      <div className="aboutCanvas">
        <div className="aboutBlob" aria-hidden />
        <div className="aboutLayout">
          <header className="aboutMark">
            <h1>{t("settings.main.about")}</h1>
            <div className="aboutWordmark">
              <span className="aboutWordmarkFirst">
                Isct
                <img className="aboutIcon" src="./icon.png" alt="" />
              </span>
              <span className="aboutWordmarkSecond">Companion</span>
            </div>
            <p className="aboutVersion">
              {t("settings.about.version")}
              <b>{__APP_VERSION__}</b>
            </p>
          </header>
          <section className="aboutPanel">
            {desktopBridge?.checkAppUpdate && (
              <>
                <Row
                  id="updates"
                  group
                  title="settings.about.appUpdates"
                  hint={
                    <span role="status">
                      {t(
                        labels[
                          status?.phase ?? (pending ? "checking" : "idle")
                        ],
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
                  <p className="aboutNote">
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
          </section>
          <nav className="aboutLinks">
            <LinkTile
              id="website"
              title={t("about.website")}
              hint="isctcomp.app"
              icon={Globe}
              url={WEBSITE}
            />
            <LinkTile
              id="github"
              title="GitHub"
              hint="isct-companion/isct-companion"
              icon={GitBranch}
              url={REPOSITORY}
            />
            <LinkTile
              id="license"
              title={t("about.license")}
              hint="GNU GPL v3.0"
              icon={Scale}
              url={LICENSE}
            />
            <LinkTile
              id="sponsor"
              title={t("about.sponsor")}
              hint={t("about.sponsorHint")}
              icon={Heart}
              action="about.sponsorOpen"
              url={SPONSOR}
            />
          </nav>
        </div>
      </div>
    </div>
  );
}
