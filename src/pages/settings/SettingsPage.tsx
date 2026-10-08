import { Fragment, useEffect, useMemo, useState } from "react";
import { t } from "../../i18n";
import type { ScheduledTask, Workspace } from "../../core/model";
import { desktopBridge } from "../../app/native";
import type { Platform, Section } from "../../app/shared";
import type { SchoolAuthProfile } from "../../platform/desktop-bridge";
import { performAction } from "../../platform/actions";
import { resolveCapabilities } from "../../platform/capabilities";
import { workspaceActions } from "../../store/workspace";
import { UiButton, UiInput } from "../../components/mantine";
import {
  Row,
  SettingsGroup,
  SettingsHeader,
  Toggle,
} from "../../components/forms";
import { AcademicCalendarSettings } from "./AcademicCalendarSettings";
import { AgentSettings } from "./AgentSettings";
import { AppearanceSettings } from "./AppearanceSettings";
import { AutomationSettings } from "./AutomationSettings";
import { BackupSettings } from "./BackupSettings";
import { DataUpdatesSettings } from "./DataUpdatesSettings";
import { DeveloperSettings, useDeveloperMode } from "./DeveloperSettings";
import { DiagnosticsSettings } from "./DiagnosticsSettings";
import { ExtensionsSettings } from "./ExtensionsSettings";
import { FilesSettings } from "./FilesSettings";
import { HiddenItems } from "./HiddenItems";
import { NotificationSettings } from "./NotificationSettings";
import { SchoolAccounts } from "./SchoolAccounts";
import { StudyProfileSettings } from "./StudyProfileSettings";
import {
  SETTING_ENTRIES,
  sectionLabel,
  sectionSupported,
  sectionTier,
  visibleSections,
  type SectionContext,
} from "./registry";
import "./settings.css";

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

export function Settings({
  platform,
  data,
  update,
  academicUpdate,
  advancedCalendarUpdate,
  caps,
  section,
  setSection,
  notify,
  showLoginPage,
  runScheduledTask,
  refreshReferences,
  refreshSchool,
  matrixOcrAvailable = false,
}: {
  platform: Platform;
  data: Workspace;
  /** Preferences and device state. */
  update: Update;
  /** Personal academic records (hidden assignments). */
  academicUpdate?: Update;
  /** Calendar overrides, academic profile and subscription rows. */
  advancedCalendarUpdate: Update;
  caps: ReturnType<typeof resolveCapabilities>;
  section: Section;
  setSection: (s: Section) => void;
  notify: (s: string) => void;
  showLoginPage: (profile: SchoolAuthProfile) => void;
  runScheduledTask?: (task: ScheduledTask) => Promise<string>;
  refreshReferences?: () => Promise<string>;
  refreshSchool?: () => Promise<void>;
  matrixOcrAvailable?: boolean;
}) {
  const [developer, setDeveloper] = useDeveloperMode();
  const [query, setQuery] = useState("");
  const [focus, setFocus] = useState<{ id: string; at: number }>();
  const desktop = platform === "native-desktop";
  const context: SectionContext = {
    platform,
    developer,
    extensionsAvailable:
      desktop && caps["site-customization"].support === "supported",
  };
  const sections = visibleSections(context);
  const results = useMemo(
    () => searchSettings(query, context),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [query, platform, context.extensionsAvailable, data.settings.locale],
  );
  const current = sections.some((item) => item.id === section)
    ? section
    : sections[0].id;

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

  const hiddenCount = data.assignments.filter(
    (row) => row.hidden && !row.deletedAt,
  ).length;
  const setAdmissionYear = (year: number | undefined) => {
    try {
      workspaceActions.setAdmissionYear(year);
    } catch (error) {
      notify(error instanceof Error ? error.message : String(error));
    }
  };
  const customizeSite = (fn: (d: Workspace) => Workspace) => {
    void performAction("customize-school-site", platform, () =>
      update(fn),
    ).catch((error) =>
      notify(error instanceof Error ? error.message : "Action denied"),
    );
  };
  const [refreshingSchool, setRefreshingSchool] = useState(false);

  const page =
    current === "accounts" ? (
      <>
        <SettingsHeader title="School accounts" />
        <SchoolAccounts
          notify={notify}
          showLoginPage={showLoginPage}
          matrixOcrAvailable={matrixOcrAvailable}
        />
        {desktopBridge && (
          <SettingsGroup title="School records">
            {refreshSchool && (
              <Row
                id="refresh"
                group
                title="Refresh school records"
                hint="Reads your courses, assignments and grades again now"
              >
                <div className="actions">
                  <UiButton
                    className="button small"
                    disabled={refreshingSchool}
                    onClick={async () => {
                      setRefreshingSchool(true);
                      try {
                        await refreshSchool();
                      } finally {
                        setRefreshingSchool(false);
                      }
                    }}
                  >
                    {refreshingSchool ? t("Refreshing…") : t("Refresh now")}
                  </UiButton>
                </div>
              </Row>
            )}
            {desktop && (
              <Row
                id="restore-page"
                title="Reopen the last school page"
                hint="Web opens where you left off, even after a restart"
              >
                <Toggle
                  checked={data.settings.restoreSchoolPage ?? false}
                  onChange={(enabled) => {
                    update((draft) => {
                      draft.settings.restoreSchoolPage = enabled;
                      if (!enabled) draft.settings.activeSchoolSite = undefined;
                      return draft;
                    });
                    void desktopBridge?.configureSchoolWebPolicy({
                      restoreAcrossRestarts: enabled,
                      idleMinutes: 10,
                    });
                  }}
                />
              </Row>
            )}
          </SettingsGroup>
        )}
      </>
    ) : current === "study" ? (
      <StudyProfileSettings
        data={data}
        update={update}
        setAdmissionYear={setAdmissionYear}
        refreshReferences={refreshReferences}
        openSection={open}
      />
    ) : current === "notifications" ? (
      <NotificationSettings
        data={data}
        update={update}
        platform={platform}
        caps={caps}
        notify={notify}
      />
    ) : current === "calendar" ? (
      <AcademicCalendarSettings
        data={data}
        update={advancedCalendarUpdate}
        notify={notify}
        readOnly={!desktop}
        refreshReferences={refreshReferences}
        openSection={open}
      />
    ) : current === "appearance" ? (
      <AppearanceSettings data={data} update={update} />
    ) : current === "files" ? (
      <FilesSettings data={data} update={update} />
    ) : current === "backup" ? (
      <BackupSettings
        data={data}
        update={update}
        platform={platform}
        caps={caps}
        notify={notify}
      />
    ) : current === "hidden" ? (
      <HiddenItems data={data} update={academicUpdate ?? update} />
    ) : current === "sources" ? (
      <DataUpdatesSettings
        data={data}
        update={update}
        updateSubscriptions={advancedCalendarUpdate}
        platform={platform}
        refreshReferences={refreshReferences}
        notify={notify}
      />
    ) : current === "automation" ? (
      <AutomationSettings
        data={data}
        update={update}
        runScheduledTask={runScheduledTask}
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
    ) : current === "agent" ? (
      <AgentSettings data={data} update={update} notify={notify} />
    ) : current === "extensions" ? (
      <ExtensionsSettings data={data} update={customizeSite} />
    ) : current === "diagnostics" ? (
      <DiagnosticsSettings data={data} caps={caps} />
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
                  item.tier !== "basic" && (
                    <span className="settingsNavGroup">
                      {t(item.tier === "advanced" ? "Advanced" : "Developer")}
                    </span>
                  )}
                <UiButton
                  className={current === item.id ? "active" : ""}
                  aria-current={current === item.id ? "page" : undefined}
                  onClick={() => open(item.id)}
                >
                  {t(item.label)}
                  {item.id === "hidden" && hiddenCount > 0 && (
                    <span className="navCount">{hiddenCount}</span>
                  )}
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
