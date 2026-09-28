import { getLocalizedCourse } from "../core/course-language";
import { AppShell, MantineProvider } from "@mantine/core";
import { UiButton } from "../components/mantine";
import {
  hydrateSyllabusLanguages,
  subscribeSyllabusLanguages,
} from "../cache/syllabus-languages";
import i18n, { t } from "../i18n";
import { useTranslation } from "react-i18next";
import { desktopBridge, hideNativeSchoolWeb } from "./native";
import { stamp, type Page, type Section } from "./shared";
import {
  onTimetable,
  setTimetable,
  allTerms,
} from "../components/timetable/workspace";

import { Today } from "../pages/today/TodayPage";
import { Schedule } from "../pages/schedule/SchedulePage";
import { Courses } from "../pages/courses/CoursesPage";
import { Tasks } from "../pages/tasks/TasksPage";
import { CourseDrawer } from "../pages/courses/CourseDrawer";
import { Web } from "../pages/web/WebPage";
import { Settings } from "../pages/settings/SettingsPage";

import { useEffect, useRef, useState } from "react";
import { termForDate } from "../core/academic";
import { workspaceActions, workspaceStore } from "../store/workspace";
import { useWorkspace } from "../store/useWorkspace";
import { detectPlatform, resolveCapabilities } from "../platform/capabilities";
import { signInAtLaunch } from "../platform/school-session";
import { appTheme, cssVariablesResolver } from "./theme";

import { academicTermForDate } from "../core/academic-calendar";

import { performAction } from "../platform/actions";
import { type NativeCapabilitySnapshot } from "../platform/desktop-bridge";

import {
  schedulerStateFromAcknowledgements,
  type SchedulerRuntimeState,
} from "../platform/scheduler";

import { CourseCatalog } from "../pages/courses/course-catalog";
import {
  addSyllabusDraft,
  isSyllabusCourse,
  removeSyllabusDraft,
} from "../core/syllabus";

import {
  CourseEditor,
  EventEditor,
  TermDialog,
  type CourseDraft,
  type EventDraft,
} from "../pages/courses/editors";
import type { Course, PersonalEvent, TermId, Workspace } from "../core/model";

const pages: Page[] = ["today", "schedule", "tasks", "courses", "web"];
const navLabels = {
  en: {
    today: "Today",
    schedule: "Schedule",
    courses: "Courses",
    tasks: "Tasks",
    web: "Web",
    settings: "Settings",
  },
  ja: {
    today: "今日",
    schedule: "予定",
    courses: "授業",
    tasks: "課題",
    web: "Web",
    settings: "設定",
  },
  zh: {
    today: "今天",
    schedule: "课表",
    courses: "课程",
    tasks: "任务",
    web: "网页",
    settings: "设置",
  },
} as const;
const icons: Record<Page, React.ReactElement> = {
  today: (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <rect x="3" y="5" width="18" height="16" rx="3" />
      <path d="M3 10h18M8 3v4M16 3v4" />
    </svg>
  ),
  schedule: (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M4 6h16M4 10h16M8 3v5M16 3v5" />
      <rect x="4" y="4" width="16" height="17" rx="3" />
      <path d="M8 14h2M14 14h2M8 18h2M14 18h2" />
    </svg>
  ),
  courses: (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <rect x="3" y="3" width="7.5" height="7.5" rx="2" />
      <rect x="13.5" y="3" width="7.5" height="7.5" rx="2" />
      <rect x="3" y="13.5" width="7.5" height="7.5" rx="2" />
      <rect x="13.5" y="13.5" width="7.5" height="7.5" rx="2" />
    </svg>
  ),
  tasks: (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="m5 7 2 2 4-4M5 15l2 2 4-4M13 8h6M13 16h6" />
    </svg>
  ),
  web: (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="12" cy="12" r="9" />
      <path d="M3 12h18M12 3a13.5 13.5 0 0 1 0 18M12 3a13.5 13.5 0 0 0 0 18" />
    </svg>
  ),
  settings: (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M4 7h9M17 7h3M4 17h3M11 17h9" />
      <circle cx="15" cy="7" r="2.2" />
      <circle cx="9" cy="17" r="2.2" />
    </svg>
  ),
};

function WorkspaceApp() {
  useTranslation();
  const store = useWorkspace((state) => state);
  const sourceData = store.workspace;
  const [, setLanguageRevision] = useState(0);
  const data = sourceData
    ? {
        ...sourceData,
        courses: sourceData.courses.map((course) =>
          getLocalizedCourse(course, sourceData.settings.locale),
        ),
      }
    : undefined;
  useEffect(
    () =>
      subscribeSyllabusLanguages(() =>
        setLanguageRevision((value) => value + 1),
      ),
    [],
  );
  useEffect(() => {
    void i18n.changeLanguage(data?.settings.locale === "ja" ? "ja" : "en");
    document.documentElement.lang =
      data?.settings.locale === "ja" ? "ja" : "en";
  }, [data?.settings.locale]);
  const [page, setPage] = useState<Page>("today");
  const pageRef = useRef<Page>("today");
  const navigateRef = useRef<(next: Page) => Promise<void>>(
    async () => undefined,
  );
  const navigationSequence = useRef(0);
  const [section, setSection] = useState<Section>("accounts");
  const [webContext, setWebContext] = useState<{
    page: "today" | "tasks" | "courses";
    label: string;
    url: string;
  } | null>(null);
  const [drawerId, setDrawerId] = useState<string | null>(null);
  const [drawerTab, setDrawerTab] = useState<
    "overview" | "materials" | "assignments"
  >("overview");
  const [courseEditor, setCourseEditor] = useState<{
    course?: Course;
    term: TermId;
  } | null>(null);
  const [eventEditor, setEventEditor] = useState<{
    event?: PersonalEvent;
    term: TermId;
  } | null>(null);
  const [termDialog, setTermDialog] = useState(false);
  const [courseCatalog, setCourseCatalog] = useState<TermId | null>(null);
  const [toast, setToast] = useState("");
  const [schedulerState, setSchedulerState] =
    useState<SchedulerRuntimeState>("off");
  const [nativeCapabilities, setNativeCapabilities] =
    useState<NativeCapabilitySnapshot>();
  useEffect(() => {
    if (!desktopBridge) return;
    void desktopBridge
      .getRuntimeCapabilities()
      .then(setNativeCapabilities)
      .catch(() => setNativeCapabilities(undefined));
  }, []);
  useEffect(() => {
    if (!sourceData || !desktopBridge || !nativeCapabilities?.syllabusSearch)
      return;
    for (const course of sourceData.courses.filter(
      (course) =>
        !course.deletedAt &&
        course.termId === sourceData.selectedTerm &&
        isSyllabusCourse(course) &&
        course.url,
    ))
      void hydrateSyllabusLanguages(
        course.url!,
        sourceData.settings.locale,
        desktopBridge.syllabusCourse,
      ).catch(() => undefined);
  }, [
    sourceData?.selectedTerm,
    sourceData?.settings.locale,
    sourceData?.courses,
    nativeCapabilities?.syllabusSearch,
  ]);
  useEffect(() => {
    void workspaceStore.initialize();
  }, []);

  // Sign saved school accounts in once at launch. After that, a school page
  // redirected to sign-in is renewed natively, so there is no polling.
  useEffect(() => {
    void signInAtLaunch(desktopBridge);
  }, []);

  useEffect(() => {
    if (!data || !desktopBridge || !nativeCapabilities?.backgroundSchedule)
      return;
    void desktopBridge.updateExportWorkspace(data).catch(() => undefined);
  }, [data, nativeCapabilities]);
  useEffect(() => {
    if (
      !desktopBridge ||
      !nativeCapabilities?.backgroundSchedule ||
      detectPlatform() !== "native-desktop"
    )
      return;
    const school =
      (localStorage.getItem("stws-hook-school") as "launch" | "hourly") ||
      "off";
    const backup =
      (localStorage.getItem("stws-hook-export") as "launch" | "hourly") ||
      "off";
    Promise.all([
      desktopBridge.configureSchoolSchedule(school),
      desktopBridge.configureExportSchedule(backup),
    ])
      .then((results) =>
        setSchedulerState(
          schedulerStateFromAcknowledgements([school, backup], results),
        ),
      )
      .catch(() => setSchedulerState("failed"));
    const changed = (event: Event) =>
      setSchedulerState(
        (event as CustomEvent<"active" | "off" | "failed">).detail,
      );
    addEventListener("stws-scheduler-state", changed);
    return () => removeEventListener("stws-scheduler-state", changed);
  }, [nativeCapabilities]);
  useEffect(() => {
    if (
      !desktopBridge ||
      !nativeCapabilities?.backgroundSchedule ||
      detectPlatform() !== "native-desktop"
    )
      return;
    return desktopBridge.onScheduledSchoolResult((result) =>
      workspaceActions.applySchoolImport(result),
    );
  }, [nativeCapabilities]);
  useEffect(() => {
    if (!data) return;
    document.documentElement.dataset.appearance = data.settings.appearance;
    document.documentElement.dataset.mantineColorScheme =
      data.settings.appearance === "system"
        ? typeof matchMedia === "function" &&
          matchMedia("(prefers-color-scheme: dark)").matches
          ? "dark"
          : "light"
        : data.settings.appearance;
    document.documentElement.dataset.textSize =
      data.settings.textSize ?? "medium";
    document.body.style.backgroundImage = data.settings.customization
      ?.background
      ? `url("${data.settings.customization.background.replaceAll('"', "%22")}")`
      : "";
    let style = document.getElementById(
      "user-app-css",
    ) as HTMLStyleElement | null;
    if (!style) {
      style = document.createElement("style");
      style.id = "user-app-css";
      document.head.append(style);
    }
    style.textContent = data.settings.customization?.appCss ?? "";
  }, [
    data?.settings.appearance,
    data?.settings.textSize,
    data?.settings.customization,
  ]);
  useEffect(() => {
    if (store.error && data) setToast(store.error);
  }, [store.error, data]);
  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(""), 2800);
    return () => clearTimeout(t);
  }, [toast]);
  useEffect(() => {
    void hideNativeSchoolWeb().catch((error) =>
      setToast(
        error instanceof Error
          ? error.message
          : "The school page could not be hidden. Return to Web and retry.",
      ),
    );
  }, []);
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") setDrawerId(null);
      if (
        (e.metaKey || e.ctrlKey) &&
        ["1", "2", "3", "4", "5"].includes(e.key)
      ) {
        e.preventDefault();
        const next = pages[Number(e.key) - 1];
        void navigateRef.current(next);
      }
    };
    addEventListener("keydown", key);
    return () => removeEventListener("keydown", key);
  }, []);
  if (!data)
    return (
      <div className="loading">
        {store.error ?? "Opening your workspace…"}
        {store.error && (
          <UiButton onClick={() => void workspaceStore.initialize()}>
            Retry
          </UiButton>
        )}
      </div>
    );

  const update = workspaceActions.updateSettings;
  const platform = detectPlatform();
  const readOnly = platform === "browser-preview";
  const academicUpdate = (fn: (d: Workspace) => Workspace) => {
    void performAction("edit-academic-data", platform, () =>
      workspaceActions.editAcademic(fn),
    ).catch((error) =>
      setToast(error instanceof Error ? error.message : "Action denied"),
    );
  };
  const advancedCalendarUpdate = (fn: (d: Workspace) => Workspace) => {
    void performAction("edit-calendar-advanced", platform, () =>
      workspaceActions.editCalendar(fn),
    ).catch((error) =>
      setToast(error instanceof Error ? error.message : "Action denied"),
    );
  };
  const caps = resolveCapabilities(
    platform,
    data.syncStatus.state === "idle" && Boolean(data.syncStatus.lastSyncedAt),
    data.settings.notifications.localPermission,
    {
      native: nativeCapabilities,
      school: {
        status: data.importStatus.school,
        lastResult: data.importStatus.lastResult,
      },
      background: { scheduler: schedulerState },
      noBackendPush: true,
    },
  );
  const navigation = navLabels[data.settings.locale];
  const currentTerm =
    academicTermForDate(new Date(), data.academicProfile) ??
    termForDate(new Date());
  const drawerCourse = drawerId
    ? data.courses.find((c) => c.id === drawerId && !c.deletedAt)
    : undefined;

  const saveCourse = (draft: CourseDraft, term: TermId, existing?: Course) =>
    academicUpdate((d) => {
      const now = stamp();
      let course = existing
        ? d.courses.find((c) => c.id === existing.id)
        : undefined;
      if (!course) {
        course = {
          id: crypto.randomUUID(),
          kind: "course",
          rev: 0,
          updatedAt: now,
          updatedBy: d.deviceId,
          termId: term,
          code: "",
          title: "",
          instructor: "",
          credits: 0,
          language: "",
          color: draft.color,
          meetings: [],
          source: { provider: "manual" },
        };
        d.courses.push(course);
      }
      const title =
        existing && draft.title.trim() === existing.title
          ? course.title
          : draft.title.trim();
      const instructor =
        existing && draft.instructor.trim() === existing.instructor
          ? course.instructor
          : draft.instructor.trim();
      const language =
        existing && draft.language === existing.language
          ? course.language
          : draft.language;
      if (existing && draft.title.trim() !== existing.title)
        course.overrides = { ...course.overrides, title };
      if (existing && draft.instructor.trim() !== existing.instructor)
        course.overrides = { ...course.overrides, instructor };
      Object.assign(course, {
        title,
        code: draft.code.trim(),
        instructor,
        credits: draft.credits,
        language,
        color: draft.color,
        url: draft.url.trim() || undefined,
        meetings: draft.meetings,
        rev: course.rev + 1,
        updatedAt: now,
        updatedBy: d.deviceId,
      });
      setTimetable(d, course, draft.onTimetable);
      return d;
    });
  const deleteCourse = (course: Course) =>
    academicUpdate((d) => {
      const now = stamp();
      for (const item of [
        ...d.courses.filter((c) => c.id === course.id),
        ...d.memberships.filter((m) => m.courseId === course.id),
        ...d.drafts.filter((x) => x.courseId === course.id),
      ]) {
        item.deletedAt = now;
        item.rev += 1;
        item.updatedAt = now;
        item.updatedBy = d.deviceId;
      }
      return d;
    });
  const toggleTimetable = (course: Course) =>
    academicUpdate((d) => {
      const target = d.courses.find((c) => c.id === course.id);
      if (target) setTimetable(d, target, !onTimetable(d, target));
      return d;
    });
  const saveEvent = (
    draft: EventDraft,
    term: TermId,
    existing?: PersonalEvent,
  ) =>
    academicUpdate((d) => {
      const now = stamp();
      const target = existing
        ? d.events.find((e) => e.id === existing.id)
        : undefined;
      if (target)
        Object.assign(target, draft, {
          rev: target.rev + 1,
          updatedAt: now,
          updatedBy: d.deviceId,
        });
      else
        d.events.push({
          ...draft,
          id: crypto.randomUUID(),
          kind: "event",
          rev: 1,
          updatedAt: now,
          updatedBy: d.deviceId,
          termId: term,
        });
      return d;
    });
  const deleteEvent = (event: PersonalEvent) =>
    academicUpdate((d) => {
      const target = d.events.find((e) => e.id === event.id);
      if (target) {
        target.deletedAt = stamp();
        target.rev += 1;
      }
      return d;
    });
  const navigateTo = async (next: Page) => {
    if (next === pageRef.current) return;
    const request = ++navigationSequence.current;

    const leavingWeb = pageRef.current === "web" && next !== "web";
    if (leavingWeb) {
      try {
        await hideNativeSchoolWeb();
      } catch (error) {
        if (request === navigationSequence.current)
          setToast(
            error instanceof Error
              ? error.message
              : typeof error === "string"
                ? error
                : "The school page could not be hidden.",
          );
        return;
      }
      if (request !== navigationSequence.current) return;
    }

    if (next !== "web") setDrawerId(null);
    pageRef.current = next;
    setPage(next);
  };
  navigateRef.current = navigateTo;

  const navigatePage = (next: Page) => {
    if (next === "web") setWebContext(null);
    void navigateTo(next);
  };

  return (
    <AppShell
      className="app"
      data-density={data.settings.density}
      navbar={{ width: 72, breakpoint: "48em", collapsed: { mobile: true } }}
      footer={{ height: 62, offset: false }}
      padding={0}
    >
      <AppShell.Navbar className="rail">
        <nav aria-label="Pages">
          {pages.map((p, i) => (
            <UiButton
              className={page === p ? "active" : ""}
              onClick={() => navigatePage(p)}
              key={p}
              title={`${navigation[p]} (⌘${i + 1})`}
            >
              {icons[p]}
              <span>{navigation[p]}</span>
            </UiButton>
          ))}
        </nav>
        <UiButton
          className={`railUtility${page === "settings" ? " active" : ""}`}
          onClick={() => void navigateTo("settings")}
          title={navigation.settings}
        >
          {icons.settings}
          <span>{navigation.settings}</span>
        </UiButton>
      </AppShell.Navbar>
      <AppShell.Footer className="mobileRail" hiddenFrom="sm">
        <nav aria-label="Pages">
          {pages.map((p) => (
            <UiButton
              className={page === p ? "active" : ""}
              onClick={() => navigatePage(p)}
              key={p}
              title={navigation[p]}
            >
              {icons[p]}
              <span>{navigation[p]}</span>
            </UiButton>
          ))}
          <UiButton
            className={page === "settings" ? "active" : ""}
            onClick={() => void navigateTo("settings")}
            title={navigation.settings}
          >
            {icons.settings}
            <span>{navigation.settings}</span>
          </UiButton>
        </nav>
      </AppShell.Footer>
      <AppShell.Main
        component="main"
        className={`workspaceMain${page === "web" ? " webMain" : page === "settings" ? " settingsMain" : ""}`}
      >
        {page === "today" ? (
          <Today
            data={data}
            term={
              data.courses.some((c) => c.termId === currentTerm && !c.deletedAt)
                ? currentTerm
                : data.selectedTerm
            }
            open={(courseId, tab) => {
              setDrawerTab(tab ?? "overview");
              setDrawerId(courseId);
            }}
            openEvent={(event) =>
              !readOnly && setEventEditor({ event, term: event.termId })
            }
            openTask={() => void navigateTo("tasks")}
            openSchedule={() => void navigateTo("schedule")}
            connectAccount={() => {
              setSection("accounts");
              void navigateTo("settings");
            }}
          />
        ) : page === "schedule" ? (
          <Schedule
            data={data}
            term={
              data.courses.some((c) => c.termId === currentTerm && !c.deletedAt)
                ? currentTerm
                : data.selectedTerm
            }
            openCourse={(courseId) => {
              setDrawerTab("overview");
              setDrawerId(courseId);
            }}
            openEvent={(event) =>
              !readOnly && setEventEditor({ event, term: event.termId })
            }
          />
        ) : page === "courses" ? (
          <Courses
            catalog={
              courseCatalog && (
                <CourseCatalog
                  locale={data.settings.locale}
                  year={Number(courseCatalog.slice(0, 4))}
                  term={courseCatalog}
                  available={
                    nativeCapabilities?.syllabusSearch === true &&
                    Boolean(desktopBridge)
                  }
                  structuredAvailable={
                    !desktopBridge ||
                    nativeCapabilities?.syllabusFilters === true
                  }
                  planned={
                    new Set(
                      data.courses
                        .filter(
                          (course) =>
                            isSyllabusCourse(course) &&
                            course.source.externalId &&
                            data.drafts.some(
                              (draft) =>
                                draft.courseId === course.id &&
                                draft.termId === course.termId &&
                                !draft.deletedAt,
                            ),
                        )
                        .map(
                          (course) =>
                            `${course.source.externalId}:${course.termId}`,
                        ),
                    )
                  }
                  search={(query, year, filters) =>
                    desktopBridge!.searchSyllabus(
                      query,
                      year,
                      100,
                      filters,
                      data.settings.locale === "ja" ? "ja" : "en",
                    )
                  }
                  hydrate={(url) =>
                    hydrateSyllabusLanguages(
                      url,
                      data.settings.locale,
                      desktopBridge!.syllabusCourse,
                    )
                  }
                  add={(course) => {
                    academicUpdate((draft) => {
                      addSyllabusDraft(draft, course);
                      draft.selectedTerm = `${course.year}-${course.quarter}`;
                      return draft;
                    });
                    setCourseCatalog(`${course.year}-${course.quarter}`);
                  }}
                  remove={(sourceKey) =>
                    academicUpdate((draft) => {
                      const course = draft.courses.find(
                        (item) =>
                          isSyllabusCourse(item) &&
                          `${item.source.externalId}:${item.termId}` ===
                            sourceKey,
                      );
                      if (course)
                        removeSyllabusDraft(draft, course.id, course.termId);
                      return draft;
                    })
                  }
                  close={() => setCourseCatalog(null)}
                  manual={() => {
                    setCourseCatalog(null);
                    setCourseEditor({ term: courseCatalog });
                  }}
                />
              )
            }
            data={data}
            currentTerm={currentTerm}
            readOnly={readOnly}
            update={academicUpdate}
            open={(courseId) => {
              setDrawerTab("overview");
              setDrawerId(courseId);
            }}
            openEvent={(event, term) =>
              !readOnly && setEventEditor({ event, term })
            }
            openCourse={(course, term) =>
              !readOnly && setCourseEditor({ course, term })
            }
            openCatalog={(term) => !readOnly && setCourseCatalog(term)}
            closeCatalog={() => setCourseCatalog(null)}
            addTerm={() => setTermDialog(true)}
            toggleTimetable={toggleTimetable}
            notify={setToast}
          />
        ) : page === "tasks" ? (
          <Tasks
            data={data}
            openCourse={(courseId) => {
              setDrawerTab("assignments");
              setDrawerId(courseId);
            }}
            openOfficial={(_assignmentId, url) => {
              setWebContext({ page: "tasks", label: "Tasks", url });
              void navigateTo("web");
            }}
          />
        ) : page === "web" ? (
          <Web
            key={webContext?.url ?? "web"}
            platform={platform}
            data={data}
            notify={setToast}
            schoolWebAvailable={nativeCapabilities?.schoolWeb === true}
            requestedUrl={webContext?.url}
            returnLabel={webContext?.label}
            onReturn={
              webContext
                ? () => {
                    const destination = webContext.page;
                    setWebContext(null);
                    void navigateTo(destination);
                  }
                : undefined
            }
          />
        ) : (
          <Settings
            platform={platform}
            data={data}
            update={update}
            advancedCalendarUpdate={advancedCalendarUpdate}
            caps={caps}
            section={section}
            setSection={setSection}
            notify={setToast}
            showLoginPage={(profile) => {
              sessionStorage.setItem(
                "stws-active-school-site",
                profile === "science" ? "science-portal" : "tokyo-portal",
              );
              void navigateTo("web");
            }}
          />
        )}
      </AppShell.Main>
      {drawerCourse && (
        <CourseDrawer
          course={drawerCourse}
          data={data}
          readOnly={readOnly}
          onTimetable={onTimetable(data, drawerCourse)}
          initialTab={drawerTab}
          close={() => setDrawerId(null)}
          edit={() => {
            setDrawerId(null);
            setCourseEditor({
              course: drawerCourse,
              term: drawerCourse.termId,
            });
          }}
          toggle={() => toggleTimetable(drawerCourse)}
        />
      )}
      {courseEditor && (
        <CourseEditor
          key={courseEditor.course?.id ?? "new"}
          course={courseEditor.course}
          fetchSyllabus={
            nativeCapabilities?.syllabusSearch === true && desktopBridge
              ? desktopBridge.syllabusCourse
              : undefined
          }
          term={courseEditor.term}
          onTimetable={
            courseEditor.course ? onTimetable(data, courseEditor.course) : true
          }
          onClose={() => setCourseEditor(null)}
          onSave={(draft) => {
            saveCourse(draft, courseEditor.term, courseEditor.course);
            setCourseEditor(null);
            setToast(courseEditor.course ? "Course saved" : "Course added");
          }}
          onDelete={
            courseEditor.course
              ? () => {
                  deleteCourse(courseEditor.course!);
                  setCourseEditor(null);
                  setToast("Course deleted");
                }
              : undefined
          }
        />
      )}
      {eventEditor && (
        <EventEditor
          key={eventEditor.event?.id ?? "new"}
          event={eventEditor.event}
          term={eventEditor.term}
          onClose={() => setEventEditor(null)}
          onSave={(draft) => {
            saveEvent(draft, eventEditor.term, eventEditor.event);
            setEventEditor(null);
            setToast("Event saved");
          }}
          onDelete={
            eventEditor.event
              ? () => {
                  deleteEvent(eventEditor.event!);
                  setEventEditor(null);
                  setToast("Event deleted");
                }
              : undefined
          }
        />
      )}
      {termDialog && (
        <TermDialog
          existing={allTerms(data, currentTerm)}
          defaultYear={Number(currentTerm.slice(0, 4))}
          onClose={() => setTermDialog(false)}
          onAdd={(term) => {
            academicUpdate((d) => {
              d.terms = [...new Set([...(d.terms ?? []), term])];
              d.selectedTerm = term;
              return d;
            });
            setTermDialog(false);
          }}
        />
      )}
      {(toast || store.error) && (
        <div className="toast" role="status">
          {store.error ?? toast}
          {store.error && (
            <UiButton onClick={() => void workspaceStore.retrySave()}>
              Retry saving
            </UiButton>
          )}
        </div>
      )}
    </AppShell>
  );
}

export default function App() {
  return (
    <MantineProvider
      theme={appTheme}
      cssVariablesResolver={cssVariablesResolver}
      defaultColorScheme="light"
    >
      <WorkspaceApp />
    </MantineProvider>
  );
}
