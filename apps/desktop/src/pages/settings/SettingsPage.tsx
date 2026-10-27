import { notify as appNotify } from "../../app/notify";
import { settingsSection } from "../../app/routes";
import { Fragment, useEffect, useMemo, useState } from "react";
import { t } from "../../i18n";
import type {
  ScheduledTask,
  StudyEnrollment,
  Workspace,
} from "../../lib/model";
import type { WorkspaceIntent } from "../../lib/workspace-intents";
import type { Platform, Section } from "../../app/shared";
import type { SchoolAuthProfile } from "../../platform/desktop-bridge";
import { performAction } from "../../platform/actions";
import { resolveCapabilities } from "../../platform/capabilities";
import { workspaceActions } from "../../store/workspace";
import { UiButton, UiInput } from "../../components/ui";
import { SettingsHeader } from "../../components/forms";
import { AcademicCalendarSettings } from "./AcademicCalendarSettings";
import { AboutSettings } from "./AboutSettings";
import { AgentSettings } from "./AgentSettings";
import { AppearanceSettings } from "./AppearanceSettings";
import { AutomationSettings } from "./AutomationSettings";
import { BackupSettings } from "./BackupSettings";
import { CampusWifiSettings } from "./CampusWifiSettings";
import { DataUpdatesSettings } from "./DataUpdatesSettings";
import { DeveloperSettings, useDeveloperMode } from "./DeveloperSettings";
import { DiagnosticsSettings } from "./DiagnosticsSettings";
import { ExtensionsSettings } from "./ExtensionsSettings";
import { FilesSettings } from "./FilesSettings";
import { NotificationSettings } from "./NotificationSettings";
import { SchoolAccounts } from "./SchoolAccounts";
import { StudyProfileSettings } from "./StudyProfileSettings";
import {
  SETTING_ENTRIES,
  sectionLabel,
  sectionParent,
  sectionSupported,
  sectionTier,
  visibleSections,
  type SectionContext,
} from "./registry";
import "./settings.css";
import { withRecords } from "../../platform/records";

type Update = (f: (d: Workspace) => Workspace) => void;

type SearchResult = {
  section: Section;
  id?: string;
  title: string;
};

function searchSettings(
  query: string,
  context: SectionContext,
): SearchResult[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return [];
  const matches = (text: string) => {
    const haystack = text.toLowerCase();
    return words.every((word) => haystack.includes(word));
  };
  const supported = (section: Section) => sectionSupported(section, context);
  const pages: SearchResult[] = visibleSections({ ...context, developer: true })
    .filter((section) => matches(`${section.label} ${t(section.label)}`))
    .map((section) => ({ section: section.id, title: section.label }));
  const settings: SearchResult[] = SETTING_ENTRIES.filter(
    (entry) =>
      supported(entry.section) &&
      (entry.available?.() ?? true) &&
      matches(
        `${entry.title} ${t(entry.title)} ${sectionLabel(entry.section)} ${t(sectionLabel(entry.section))} ${entry.terms ?? ""}`,
      ),
  ).map((entry) => ({
    section: entry.section,
    id: entry.id,
    title: entry.title,
  }));
  return [...pages, ...settings].slice(0, 12);
}

function SettingsBody({
  otpSetupRevision = 0,
  platform,
  data,
  mutate,
  update,
  advancedCalendarUpdate,
  caps,
  section,
  setSection,
  notify,
  showLoginPage,
  runScheduledTask,
  refreshReferences,
  matrixOcrAvailable = false,
  capabilitiesReady = true,
}: {
  platform: Platform;
  data: Workspace;
  mutate: (intent: WorkspaceIntent) => Promise<unknown>;
  /** Preferences and device state. */
  update: Update;
  /** Calendar overrides, academic profile and subscription rows. */
  advancedCalendarUpdate: (f: (d: Workspace) => Workspace) => Promise<unknown>;
  caps: ReturnType<typeof resolveCapabilities>;
  section: Section;
  setSection: (s: Section) => void;
  notify: (s: string) => void;
  otpSetupRevision?: number;
  showLoginPage: (
    profile: SchoolAuthProfile,
    targetUrl?: string,
    setupOtp?: boolean,
  ) => void;
  runScheduledTask?: (task: ScheduledTask) => Promise<string>;
  /** Whether reference data can be checked now (the desktop app). */
  refreshReferences?: boolean;
  matrixOcrAvailable?: boolean;
  capabilitiesReady?: boolean;
}) {
  const [developer, setDeveloper] = useDeveloperMode();
  const [query, setQuery] = useState("");
  const [focus, setFocus] = useState<{ id: string; at: number }>();
  const desktop = platform === "native-desktop";
  const context: SectionContext = {
    platform,
    developer,
    extensionsAvailable:
      desktop &&
      (!capabilitiesReady ||
        caps["site-customization"].support === "supported"),
  };
  const sections = visibleSections(context);
  const results = useMemo(
    () => searchSettings(query, context),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [query, platform, context.extensionsAvailable, data.settings.locale],
  );
  const current = settingsSection(section, context);
  useEffect(() => {
    if (section === "extensions" && !capabilitiesReady) return;
    if (current !== section) setSection(current);
  }, [current, section, setSection, capabilitiesReady]);

  const open = (target: Section, id?: string) => {
    const locked =
      !developer &&
      (sectionTier(target) === "developer" ||
        (target === "developer" &&
          id !== undefined &&
          id !== "developer-mode"));
    if (locked) {
      notify(t("Turn on Developer options to open this page."));
      setSection("developer");
      setFocus({ id: "developer-mode", at: Date.now() });
    } else {
      setSection(target);
      if (id) setFocus({ id, at: Date.now() });
    }
    setQuery("");
  };

  // Search lands on the exact row: unfold it, scroll to it, flash it.
  useEffect(() => {
    if (!focus) return;
    const frame = requestAnimationFrame(() => {
      const target = document.querySelector<HTMLElement>(
        `.settingsPane [data-setting="${focus.id}"]`,
      );
      if (!target) return;
      const folded = target.closest("details");
      if (folded) folded.open = true;
      target.scrollIntoView({ block: "center", behavior: "smooth" });
      target.classList.add("settingFlash");
      window.setTimeout(() => target.classList.remove("settingFlash"), 1600);
    });
    return () => cancelAnimationFrame(frame);
  }, [focus, current]);

  const setStudyProfile = (
    admissionYear: number | undefined,
    studyEnrollments: StudyEnrollment[],
  ) => {
    try {
      return workspaceActions
        .setStudyProfile(admissionYear, studyEnrollments)
        .then(() => undefined);
    } catch (error) {
      return Promise.reject(error);
    }
  };
  const customizeSite = (fn: (d: Workspace) => Workspace) => {
    void performAction("customize-school-site", platform, () =>
      update(fn),
    ).catch((error) => appNotify.error(error));
  };

  const page =
    current === "accounts" ? (
      <>
        <SettingsHeader title="School accounts" />
        <SchoolAccounts
          otpSetupRevision={otpSetupRevision}
          notify={notify}
          showLoginPage={showLoginPage}
          matrixOcrAvailable={matrixOcrAvailable}
        />
      </>
    ) : current === "study" ? (
      <StudyProfileSettings
        data={data}
        update={update}
        setStudyProfile={setStudyProfile}
        openSection={sectionSupported("calendar", context) ? open : undefined}
      />
    ) : current === "notifications" ? (
      <NotificationSettings
        data={data}
        update={update}
        caps={caps}
        notify={notify}
      />
    ) : current === "calendar" ? (
      <AcademicCalendarSettings
        data={data}
        mutate={mutate}
        notify={notify}
        readOnly={!desktop}
        refreshReferences={refreshReferences}
        back={() => open("study")}
      />
    ) : current === "appearance" ? (
      <AppearanceSettings data={data} update={update} />
    ) : current === "files" ? (
      <FilesSettings data={data} update={update} />
    ) : current === "backup" ? (
      <BackupSettings data={data} update={update} caps={caps} notify={notify} />
    ) : current === "sources" || current === "automation" ? (
      <DataUpdatesSettings
        data={data}
        update={update}
        platform={platform}
        refreshReferences={refreshReferences}
        tasks={
          <AutomationSettings
            data={data}
            mutate={mutate}
            runScheduledTask={runScheduledTask}
          />
        }
      />
    ) : current === "developer" ? (
      <DeveloperSettings
        data={data}
        update={update}
        calendarUpdate={advancedCalendarUpdate}
        developer={developer}
        setDeveloper={setDeveloper}
        notify={notify}
        desktop={desktop}
      />
    ) : current === "wifi" ? (
      <CampusWifiSettings notify={notify} />
    ) : current === "agent" ? (
      <AgentSettings data={data} update={update} notify={notify} />
    ) : current === "extensions" ? (
      <ExtensionsSettings data={data} update={customizeSite} />
    ) : current === "diagnostics" ? (
      <DiagnosticsSettings data={data} notify={notify} />
    ) : current === "about" ? (
      <AboutSettings data={data} update={update} />
    ) : null;

  return (
    <div className="page settings">
      <nav className="settingsNav" aria-label={t("Settings sections")}>
        <UiInput
          className="settingsSearch"
          type="search"
          value={query}
          placeholder={t("Search settings")}
          aria-label={t("Search settings")}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && results[0])
              open(results[0].section, results[0].id);
            if (event.key === "Escape") setQuery("");
          }}
        />
        {query.trim() ? (
          <div className="settingsResults" role="list">
            {results.length === 0 && (
              <span className="settingsNoResults">
                {t("No matching settings")}
              </span>
            )}
            {results.map((result) => (
              <UiButton
                key={`${result.section}:${result.id ?? ""}`}
                role="listitem"
                onClick={() => open(result.section, result.id)}
              >
                <b>{t(result.title)}</b>
                {result.id && <small>{t(sectionLabel(result.section))}</small>}
              </UiButton>
            ))}
          </div>
        ) : (
          <div className="settingsNavList">
            {sections.map((item, index) => (
              <Fragment key={item.id}>
                {item.tier !== sections[index - 1]?.tier &&
                  (item.tier === "about" ? (
                    <hr className="settingsNavDivider" />
                  ) : (
                    item.tier !== "basic" && (
                      <span className="settingsNavGroup">
                        {t(item.tier === "advanced" ? "Advanced" : "Developer")}
                      </span>
                    )
                  ))}
                <UiButton
                  className={sectionParent(current) === item.id ? "active" : ""}
                  aria-current={current === item.id ? "page" : undefined}
                  onClick={() => open(item.id)}
                >
                  {t(item.label)}
                </UiButton>
              </Fragment>
            ))}
          </div>
        )}
      </nav>
      <section className="settingsPane" key={current}>
        {page}
      </section>
    </div>
  );
}

/** Settings with the records it shows (`platform/records`). */
export const Settings = withRecords(
  [
    "academicProfile",
    "exceptions",
    "scheduledTasks",
    "subscriptions",
    "intensiveSchedules",
    "importStatus",
    "syncStatus",
    "logs",
    "classGroups",
    "courses",
    "plans",
    "events",
    "academicCalendar",
  ],
  SettingsBody,
);
