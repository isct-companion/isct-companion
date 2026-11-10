import { GitBranch, Globe, Heart, Scale, type LucideIcon } from "lucide-react";
import { Switch, UiButton } from "../../components/ui";
import { useResetGuide } from "../../components/guide";
import { desktopBridge } from "../../app/native";
import { lastInstallText, useAppUpdate } from "../../app/useDesktopUpdates";
import { workspaceStore } from "../../store/workspace";
import type { Workspace } from "../../lib/model";
import { t, type MessageKey } from "../../i18n";
import { IconToy } from "./IconToy";
import "./about.css";

const WEBSITE = "https://isctcomp.app/";
const REPOSITORY = "https://github.com/isct-companion/isct-companion";
const LICENSE = `${REPOSITORY}/blob/main/LICENSE`;
const SPONSOR = "https://isctcomp.app/sponsor/";

/** A chip that opens `url` in the system browser. */
function LinkChip({
  id,
  label,
  title,
  icon: Icon,
  url,
}: {
  id: string;
  label: string;
  title?: string;
  icon: LucideIcon;
  url: string;
}) {
  return (
    <UiButton
      className="aboutLink"
      data-setting={id}
      title={title}
      onClick={() => void desktopBridge?.openExternal(url)}
    >
      <Icon aria-hidden strokeWidth={1.9} />
      <span>{label}</span>
    </UiButton>
  );
}

/** The release part of the version, and what a development build adds to it
 * (`-dev.<time>+<commit>`), which reads as a footnote. */
function splitVersion(version: string) {
  const cut = version.search(/[-+]/);
  return cut < 0 ? [version, ""] : [version.slice(0, cut), version.slice(cut)];
}

/** The app itself: its name and version, updates, where it lives, its license
 * and how to support it. A page of its own, under Settings in the rail. */
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
  const phase = status?.phase ?? (pending ? "checking" : "idle");
  const megabytes = (bytes: number) => (bytes / 1024 / 1024).toFixed(1);
  const progress =
    status?.phase === "downloading" && status.progress
      ? ` · ${megabytes(status.progress.downloaded)}${
          status.progress.total ? ` / ${megabytes(status.progress.total)}` : ""
        } MB`
      : "";
  const [release, build] = splitVersion(__APP_VERSION__);
  return (
    <div className="page">
      <div className="aboutCanvas">
        <div className="aboutLayout">
          <section className="aboutIntro">
            <p className="aboutEyebrow">{t("settings.main.about")}</p>
            <h1 className="aboutName">
              Isct<span className="aboutMark">Companion</span>
            </h1>
            <p className="aboutTagline">{t("about.tagline")}</p>

            {desktopBridge?.checkAppUpdate && (
              <div
                className="aboutUpdate"
                data-setting="updates"
                role="group"
                aria-label={t("settings.about.appUpdates")}
              >
                <div className="aboutUpdateLine">
                  {status?.phase === "available" ? (
                    <UiButton
                      className="button primary"
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
                  ) : status?.phase === "ready" ? (
                    <UiButton
                      className="button primary"
                      onClick={async () => {
                        await workspaceStore.flush();
                        await desktopBridge?.installAppUpdate?.();
                      }}
                    >
                      {t("settings.about.installAndQuit")}
                    </UiButton>
                  ) : (
                    <UiButton
                      className="button"
                      disabled={pending}
                      onClick={() => void check(true)}
                    >
                      {t("settings.about.checkForUpdates")}
                    </UiButton>
                  )}
                  <span
                    className="aboutStatus"
                    role="status"
                    data-phase={phase}
                  >
                    {t(labels[phase])}
                    {status?.version ? ` · ${status.version}` : ""}
                    {progress}
                    {status?.phase === "error" && status.message
                      ? ` · ${status.message}`
                      : ""}
                    {status?.lastInstall && (
                      <small>{lastInstallText(status.lastInstall)}</small>
                    )}
                  </span>
                </div>
              </div>
            )}
            <div className="aboutPrefs">
              {desktopBridge?.checkAppUpdate && (
                <span
                  data-setting="automatic-updates"
                  title={t("settings.about.downloadInTheBackgroundAnd")}
                >
                  <Switch
                    className="settingSwitch"
                    label={t("settings.about.automaticUpdates")}
                    checked={data.settings.automaticAppUpdates ?? false}
                    disabled={!status?.signedUpdates}
                    onCheckedChange={(value) =>
                      update((draft) => {
                        draft.settings.automaticAppUpdates = value;
                        return draft;
                      })
                    }
                  />
                </span>
              )}
              <UiButton
                className="textButton"
                data-setting="guide"
                onClick={resetGuide}
              >
                {t("settings.main.showWelcomeAndTipsAgain")}
              </UiButton>
            </div>
            {status && !status.signedUpdates && (
              <p className="aboutNote">
                {t("settings.about.automaticInstallationRequiresAnUpdate")}
              </p>
            )}

            <p className="aboutMeta">
              <span>
                {t("settings.about.version")} <b>{release}</b>
                {build && <small>{build}</small>}
              </span>
              <span>{t("about.meta")}</span>
            </p>

            <nav className="aboutLinks">
              <LinkChip
                id="website"
                label={t("about.website")}
                icon={Globe}
                url={WEBSITE}
              />
              <LinkChip
                id="github"
                label="GitHub"
                icon={GitBranch}
                url={REPOSITORY}
              />
              <LinkChip
                id="license"
                label="GPL-3.0"
                title={t("about.license")}
                icon={Scale}
                url={LICENSE}
              />
              <LinkChip
                id="sponsor"
                label={t("about.sponsor")}
                icon={Heart}
                url={SPONSOR}
              />
            </nav>
          </section>
          <IconToy className="aboutToy" />
        </div>
      </div>
    </div>
  );
}
