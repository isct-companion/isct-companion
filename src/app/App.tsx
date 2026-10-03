import { pickLocalized } from "../core/model";
import { displayColor } from "../core/course-colors";
import { decodeEntities } from "../core/text";
import { AppShell, MantineProvider } from "@mantine/core";
import { invoke } from "@tauri-apps/api/core";
import { UiButton } from "../components/mantine";
import i18n, { t } from "../i18n";
import { useTranslation } from "react-i18next";
import { desktopBridge, hideNativeSchoolWeb } from "./native";
import { stamp, type Page, type Section } from "./shared";
import {
  onTimetable,
  setTimetable,
  allTerms,
  timetableCourses,
} from "../components/timetable/workspace";

import { Today } from "../pages/today/TodayPage";
import { Schedule } from "../pages/schedule/SchedulePage";
import { Courses } from "../pages/courses/CoursesPage";
import { CourseDrawer } from "../pages/courses/CourseDrawer";
import { Web } from "../pages/web/WebPage";
import { Settings } from "../pages/settings/SettingsPage";
import { PlanEditor, type PlanDraft } from "../pages/schedule/PlanEditor";
import { IntensiveEditor } from "../pages/courses/IntensiveEditor";

import { useEffect, useRef, useState } from "react";
import { termForDate } from "../core/academic";
import { workspaceActions, workspaceStore } from "../store/workspace";
import { useWorkspace } from "../store/useWorkspace";
import { detectPlatform, resolveCapabilities } from "../platform/capabilities";
import {
  checkSessionsOnFocus,
  signInAtLaunch,
} from "../platform/school-session";
import { appTheme, cssVariablesResolver } from "./theme";

import { academicTermForDate } from "../core/academic-calendar";

import { performAction } from "../platform/actions";
import {
  catalogCourse,
  type NativeCapabilitySnapshot,
} from "../platform/desktop-bridge";

import {
  schedulerStateFromAcknowledgements,
  type SchedulerRuntimeState,
} from "../platform/scheduler";
import { createScheduledTaskRuntime } from "../platform/scheduled-tasks";

import { CourseFinder } from "../pages/courses/CourseFinder";
import { LmsMergeDialog } from "../pages/courses/LmsMergeDialog";
import { AcademicSummary } from "../pages/courses/AcademicSummary";
import {
  applyLmsMerge,
  diffLms,
  hasDifferences,
  lmsTerms,
  type MergeDiff,
} from "../core/lms-merge";
import type { SchoolRecords } from "../platform/desktop-bridge";
import { staleSources } from "../platform/school-freshness";
import { CoursePanel, OfferingPanel } from "../pages/courses/CoursePanel";
import { PageErrorBoundary } from "./PageErrorBoundary";
import { coursesTerm } from "../pages/courses/CoursesPage";
import { SYLLABUS_DEPARTMENTS, departmentById } from "../core/departments";
import {
  DAYS as FINDER_DAYS,
  factorsOf,
  normalizedKeys,
  type FinderContext,
  type FinderCourse,
  clashingCourses,
} from "../core/course-query";
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
import type {
  Course,
  PersonalEvent,
  Plan,
  Quarter,
  ScheduledTask,
  TermId,
  Workspace,
} from "../core/model";

const APP_LAUNCHED_AT = new Date();
const KYOMU_REGISTRATION =
  "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0.aspx";
const KYOMU_GPA =
  "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_7.aspx";
const WEEKDAYS = ["月", "火", "水", "木", "金", "土", "日"];

/** A pull result as one short line; undefined when everything succeeded. */
function pullProblem(outcomes: { state: string; message: string }[]) {
  const failed = outcomes.find((item) => item.state !== "ok");
  if (!failed) return undefined;
  return failed.state === "reconnect-required"
    ? "Sign-in required. Check the account in Settings → Accounts."
    : failed.message;
}

async function executeScheduledTask(task: ScheduledTask): Promise<string> {
  const exportWorkspace = async () => {
    await workspaceStore.flush();
    if (desktopBridge) await desktopBridge.writeScheduledExport();
    else await workspaceActions.exportData();
    return "Export completed";
  };
  const refreshSchool = async () => {
    if (!desktopBridge?.reloadSchoolWeb)
      throw new Error("School refresh requires the desktop app.");
    const [lms, kyomu] = await Promise.all([
      desktopBridge.reloadSchoolWeb("science-lms"),
      desktopBridge.reloadSchoolWeb("kyomu"),
    ]);
    if (!lms && !kyomu)
      throw new Error("Open LMS or 教務 Web once before scheduling refreshes.");
    globalThis.setTimeout(() => void workspaceStore.reload(), 2_000);
    return "School refresh requested";
  };

  if (task.action === "export") return exportWorkspace();
  if (task.action === "school-refresh") return refreshSchool();
  const workspace = workspaceStore.getState().workspace;
  const hook = workspace?.settings.customization?.hooks.find(
    (candidate) => candidate.id === task.hookId && candidate.enabled,
  );
  if (!hook) throw new Error("The scheduled hook is missing or disabled.");
  // A scheduled hook invokes its declared app action. Its free-form script is
  // never eval'd in the trusted main window.
  if (hook.action === "school-sync") return refreshSchool();
  if (hook.action === "export") return exportWorkspace();
  throw new Error(
    "Cloud hooks require interactive authorization and must be run manually.",
  );
}

const pages: Page[] = ["today", "schedule", "courses", "web"];
const navLabels = {
  en: {
    today: "Today",
    schedule: "Schedule",
    courses: "Courses",
    web: "Web",
    settings: "Settings",
  },
  ja: {
    today: "今日",
    schedule: "予定",
    courses: "授業",
    web: "Web",
    settings: "設定",
  },
  zh: {
    today: "今天",
    schedule: "课表",
    courses: "课程",
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
  const data = sourceData
    ? {
        ...sourceData,
        courses: sourceData.courses.map((course) => ({
          ...course,
          color: displayColor(course),
          title:
            course.overrides?.title ??
            decodeEntities(
              pickLocalized(
                course.names?.title,
                sourceData.settings.locale,
                course.title,
              ),
            ),
          instructor:
            course.overrides?.instructor ??
            pickLocalized(
              course.names?.instructor,
              sourceData.settings.locale,
              course.instructor,
            ),
        })),
      }
    : undefined;
  useEffect(() => {
    void i18n.changeLanguage(data?.settings.locale === "ja" ? "ja" : "en");
    document.documentElement.lang =
      data?.settings.locale === "ja" ? "ja" : "en";
  }, [data?.settings.locale]);
  const [page, setPage] = useState<Page>("today");
  const visitedPages = useRef(new Set<Page>());
  const pageRef = useRef<Page>("today");
  const navigateRef = useRef<(next: Page) => Promise<void>>(
    async () => undefined,
  );
  const navigationSequence = useRef(0);
  const [section, setSection] = useState<Section>("accounts");
  const [webContext, setWebContext] = useState<{
    page: "today" | "schedule" | "courses";
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
  const [planEditor, setPlanEditor] = useState<Plan | "new" | null>(null);
  const [intensiveCourseId, setIntensiveCourseId] = useState<string | null>(
    null,
  );
  const [termDialog, setTermDialog] = useState(false);
  // Course finder conditions; shared with the Courses timetable so slot clicks set times.
  const [finderKeys, setFinderKeys] = useState<string[]>([]);
  // The quarter the finder's conditions were made for; switching quarters resets them.
  const [finderTerm, setFinderTerm] = useState<TermId | null>(null);
  // A course opened from the Courses timetable, shown over the syllabus panel.
  const [panelCourseId, setPanelCourseId] = useState<string | null>(null);
  // A search result opened from the syllabus panel, shown over the timetable.
  const [offeringPanel, setOfferingPanel] = useState<FinderCourse | null>(null);
  // Courses the last finder add pushed off the timetable, so Undo can bring them back.
  const replacedByAdd = useRef<string[]>([]);
  const [toast, setToast] = useState("");
  const [pulling, setPulling] = useState(false);
  const pullBusy = useRef(false);
  const kyomuBusy = useRef(false);
  const refreshStale = useRef<(profile: "science" | "tokyo-tech") => Promise<void>>(
    async () => undefined,
  );
  // TTL refresh once an account is signed in (launch, renewal), and when the
  // window comes back after a while.
  useEffect(() => {
    if (!desktopBridge?.onSchoolSession) return;
    const active = new Set<"science" | "tokyo-tech">();
    let lastFocus = 0;
    const offSession = desktopBridge.onSchoolSession((event) => {
      if (event.state !== "active") return;
      active.add(event.profile);
      void refreshStale.current(event.profile).catch(() => undefined);
    });
    const onFocus = () => {
      if (document.visibilityState === "hidden" || Date.now() - lastFocus < 10 * 60_000) return;
      lastFocus = Date.now();
      for (const profile of active) void refreshStale.current(profile).catch(() => undefined);
    };
    addEventListener("focus", onFocus);
    return () => {
      offSession();
      removeEventListener("focus", onFocus);
    };
  }, []);
  const [kyomuRefreshing, setKyomuRefreshing] = useState(false);
  const [mergeDiff, setMergeDiff] = useState<MergeDiff | null>(null);
  const [schoolRecords, setSchoolRecords] = useState<SchoolRecords>();
  const loadSchoolRecords = () =>
    desktopBridge?.schoolRecords
      ? desktopBridge
          .schoolRecords()
          .then((records) => {
            setSchoolRecords(records);
            return records;
          })
          .catch(() => undefined)
      : Promise.resolve(undefined);
  useEffect(() => {
    void loadSchoolRecords();
    // Browsing LMS / 教務Web in Web runs the extractors; pick up their rows.
    return desktopBridge?.onSchoolIngest?.((outcome) => {
      if (outcome.state !== "ok") return;
      void loadSchoolRecords();
      if (pageRef.current !== "web") void workspaceStore.reload();
    });
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
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
      void desktopBridge
        .syllabusCourse(
          course.url!,
          sourceData.settings.locale === "ja" ? "ja" : "en",
        )
        .catch(() => undefined);
  }, [
    sourceData?.selectedTerm,
    sourceData?.settings.locale,
    sourceData?.courses,
    nativeCapabilities?.syllabusSearch,
  ]);
  useEffect(() => {
    void workspaceStore.initialize();
  }, []);

  useEffect(() => {
    if (!sourceData) return;
    const updateState = () => {
      const tasks = workspaceStore.getState().workspace?.scheduledTasks ?? [];
      setSchedulerState(tasks.some((task) => task.enabled) ? "active" : "off");
    };
    updateState();
    const unsubscribe = workspaceStore.subscribe(updateState);
    const runtime = createScheduledTaskRuntime({
      getTasks: () =>
        workspaceStore.getState().workspace?.scheduledTasks ?? [],
      markStarted: (taskId, occurrenceAt, runAt) =>
        workspaceActions.recordScheduledTaskRun(
          taskId,
          occurrenceAt,
          runAt,
          "Running",
        ),
      execute: executeScheduledTask,
      markFinished: (taskId, occurrenceAt, runAt, result) =>
        workspaceActions.recordScheduledTaskRun(
          taskId,
          occurrenceAt,
          runAt,
          result,
        ),
      startedAt: APP_LAUNCHED_AT,
    });
    return () => {
      runtime.stop();
      unsubscribe();
    };
  }, [sourceData?.deviceId]);

  // Sign saved school accounts in once at launch. After that, a school page
  // redirected to sign-in is renewed natively, so there is no polling.
  useEffect(() => {
    void signInAtLaunch(desktopBridge);
  }, []);
  // Coming back to the window re-checks the school sessions (at most every 2 min).
  useEffect(() => checkSessionsOnFocus(desktopBridge), []);

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
    document.getElementById("user-app-css")?.remove();
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
      if ((e.metaKey || e.ctrlKey) && ["1", "2", "3", "4"].includes(e.key)) {
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
  const settingsDepartment = data.settings.departmentId;
  const ownDepartment = departmentById(settingsDepartment);
  const courseTerm = coursesTerm(data, currentTerm);
  const finderTermCourses = timetableCourses(data, courseTerm);
  const finderContext: FinderContext = {
    currentQuarter: Number(currentTerm.slice(-2, -1)),
    recommended: new Map(
      data.recommendedCourses
        .filter(
          (item) =>
            !item.deletedAt &&
            item.admissionYear === data.academicProfile.admissionYear &&
            Boolean(settingsDepartment) &&
            (item.departmentId === settingsDepartment ||
              settingsDepartment!.endsWith(`:${item.departmentId}`)),
        )
        .map((item) => [item.code, { group: item.group, mark: item.mark }]),
    ),
    departments: [...SYLLABUS_DEPARTMENTS],
    busy: finderTermCourses.flatMap((course) =>
      course.meetings.flatMap((meeting) =>
        meeting.periods.length
          ? meeting.days.map((day) => ({
              day,
              from: Math.min(...meeting.periods),
              to: Math.max(...meeting.periods),
            }))
          : [],
      ),
    ),
  };
  const finderPlaced = new Set(finderTermCourses.map((course) => course.code));
  const finderSlots = new Set(
    factorsOf(finderKeys, finderContext)
      .factors.filter((factor) => factor.type === "time")
      .flatMap((factor) =>
        factor.alts.flatMap(
          (condition) =>
            condition.values as { day?: number; from?: number; to?: number }[],
        ),
      )
      .filter((value) => value.day && value.from)
      .map((value) => `${value.day}:${value.from}-${value.to}`),
  );
  // A timetable click picks one slot (or 集中): it replaces the time choice,
  // a second click clears it. Typed conditions can still combine several.
  const isTimeKey = (key: string) => {
    const type = factorsOf([key], finderContext).factors[0]?.type;
    return type === "time" || key === "集中";
  };
  // ⌘/Ctrl-click adds the slot instead of replacing the time choice.
  const pickFinderTime = (key: string, append = false) =>
    setFinderKeys((current) =>
      current.includes(key)
        ? current.filter((item) => item !== key)
        : normalizedKeys(
            [...(append ? current : current.filter((item) => !isTimeKey(item))), key],
            finderContext,
          ),
    );
  const toggleFinderSlot = (day: number, from: number, to: number, append: boolean) => {
    // With a course open on the right, an empty cell just closes it.
    if (panelCourseId) return setPanelCourseId(null);
    const name = FINDER_DAYS[day - 1];
    if (name) pickFinderTime(`${name}${from}-${to}`, append);
  };
  // Start from the quarter being planned ("curr" when it is the current one)
  // and your own department; remove the department pill to search everywhere.
  if (finderTerm !== courseTerm) {
    setFinderTerm(courseTerm);
    setPanelCourseId(null);
    setOfferingPanel(null);
    setFinderKeys([
      courseTerm === currentTerm ? "curr" : `${Number(courseTerm.slice(-2, -1))}Q`,
      ...(ownDepartment?.name.ja ? [ownDepartment.name.ja] : []),
    ]);
  }
  const panelCourse = panelCourseId
    ? data.courses.find((c) => c.id === panelCourseId && !c.deletedAt)
    : undefined;
  // Only local drafts can be replaced; courses registered at school stay put.
  const replaceable = finderTermCourses.filter((course) =>
    data.planItems.some(
      (item) => item.courseId === course.id && item.termId === courseTerm && !item.deletedAt,
    ),
  );
  const termCredits = finderTermCourses.reduce((sum, course) => sum + course.credits, 0);
  const addFinderCourses = (rows: FinderCourse[]) =>
    academicUpdate((draft) => {
      const quarter = courseTerm.slice(5) as Quarter;
      const replaced: string[] = [];
      for (const row of rows) {
        // Add replaces: a planned course in the same periods leaves the timetable.
        // Courses registered at school stay; only local drafts are replaced.
        for (const victim of clashingCourses(row, timetableCourses(draft, courseTerm)))
          if (removeSyllabusDraft(draft, victim.id, courseTerm)) replaced.push(victim.id);
        const course = catalogCourse(
          row,
          data.settings.locale === "ja" ? "ja" : "en",
        );
        // Always into the quarter on screen, so it shows up on this timetable.
        addSyllabusDraft(draft, {
          ...course,
          year: Number(courseTerm.slice(0, 4)),
          quarter,
        });
      }
      replacedByAdd.current = replaced;
      return draft;
    });
  const removeFromTimetable = (course: Course) => {
    const school = data.memberships.some(
      (item) =>
        item.courseId === course.id &&
        !item.deletedAt &&
        item.evidence?.provider !== "manual",
    );
    if (
      school &&
      !confirm(
        "Remove this course from the local timetable? This does not withdraw your school registration.",
      )
    )
      return;
    academicUpdate((d) => {
      const target = d.courses.find((c) => c.id === course.id && !c.deletedAt);
      if (!target) return d;
      if (isSyllabusCourse(target) && !school) removeSyllabusDraft(d, target.id, target.termId);
      else setTimetable(d, target, false);
      return d;
    });
    setPanelCourseId(null);
  };

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
          creditSplit: { lecture: 0, exercise: 0, lab: 0 },
          instructionLanguage: "other",
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
        existing && draft.language === existing.instructionLanguage
          ? course.instructionLanguage
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
        instructionLanguage: language,
        creditSplit: {
          lecture: draft.credits,
          exercise: 0,
          lab: 0,
        },
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
        ...d.planItems.filter((x) => x.courseId === course.id),
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
  const savePlan = (draft: PlanDraft, existing?: Plan) =>
    academicUpdate((workspace) => {
      const now = stamp();
      const target = existing
        ? workspace.plans.find((item) => item.id === existing.id)
        : undefined;
      if (target)
        Object.assign(target, draft, {
          rev: target.rev + 1,
          updatedAt: now,
          updatedBy: workspace.deviceId,
        });
      else
        workspace.plans.push({
          ...draft,
          id: crypto.randomUUID(),
          rev: 1,
          updatedAt: now,
          updatedBy: workspace.deviceId,
        });
      return workspace;
    });
  const deletePlan = (plan: Plan) =>
    academicUpdate((workspace) => {
      const target = workspace.plans.find((item) => item.id === plan.id);
      if (target) {
        target.deletedAt = stamp();
        target.updatedAt = target.deletedAt;
        target.updatedBy = workspace.deviceId;
        target.rev += 1;
      }
      return workspace;
    });
  const openWeb = (url: string, from: "today" | "schedule" | "courses" = "courses") => {
    const labels = { today: "Today", schedule: "Schedule", courses: "Courses" } as const;
    setWebContext({ page: from, label: labels[from], url });
    void navigateTo("web");
  };
  /** LMS pull, then a review dialog when the timetable and LMS differ. */
  // One school pull at a time, whether the person or a TTL started it.
  // `quiet` (TTL refresh) reports only a review dialog, never a toast.
  const pullLms = async (scope: "term" | "all", quiet = false, term = courseTerm) => {
    if (!desktopBridge?.pullSchool) return quiet || setToast("LMS pull needs the desktop app.");
    if (pullBusy.current) return;
    pullBusy.current = true;
    setPulling(true);
    try {
      const problem = pullProblem(await desktopBridge.pullSchool("science-lms"));
      if (problem) return quiet || setToast(problem);
      await workspaceStore.reload();
      const records = await loadSchoolRecords();
      const lmsCourses = records?.lmsCourses ?? [];
      const years = [...new Set(lmsCourses.map((row) => row.year).filter((year): year is number => Boolean(year)))];
      const loadOfferings = async () =>
        new Map(
          (
            await Promise.all(
              years.map((year) => desktopBridge?.catalogOfferings?.(year).catch(() => []) ?? []),
            )
          )
            .flat()
            .map((row) => [row.id, row]),
        );
      let offerings = await loadOfferings();
      // Schedules come from the syllabus; fetch details the catalog lacks.
      const missing = lmsCourses
        .map((row) => row.offeringId)
        .filter((id): id is string => Boolean(id) && offerings.get(id!)?.detailed !== true);
      if (missing.length && desktopBridge.catalogSyncOfferings) {
        for (let index = 0; index < missing.length; index += 20)
          await desktopBridge.catalogSyncOfferings(missing.slice(index, index + 20)).catch(() => []);
        offerings = await loadOfferings();
      }
      const workspace = workspaceStore.getState().workspace;
      if (!workspace) return;
      const terms =
        scope === "term"
          ? [term]
          : [
              ...new Set([
                ...lmsCourses.flatMap((row) =>
                  lmsTerms(row, row.offeringId ? offerings.get(row.offeringId) : undefined),
                ),
                ...allTerms(workspace, currentTerm),
              ]),
            ].sort();
      const diff = diffLms(workspace, lmsCourses, terms, offerings);
      if (hasDifferences(diff)) setMergeDiff(diff);
      else {
        // Still link LMS assignments and materials to the matching courses.
        academicUpdate((draft) =>
          applyLmsMerge(draft, diff, new Set(), new Set()),
        );
        workspaceActions.markTimetableReviewed();
        if (!quiet) setToast(`Timetable matches LMS (${diff.common.length})`);
      }
    } catch (error) {
      if (!quiet) setToast(error instanceof Error ? error.message : String(error));
    } finally {
      pullBusy.current = false;
      setPulling(false);
    }
  };
  /** Assignments and materials only; the timetable stays as it is. */
  const fetchLms = async (quiet = false) => {
    if (!desktopBridge?.pullSchool || pullBusy.current) return;
    pullBusy.current = true;
    setPulling(true);
    try {
      const problem = pullProblem(await desktopBridge.pullSchool("science-lms"));
      await workspaceStore.reload();
      void loadSchoolRecords();
      if (!quiet) setToast(problem ?? "LMS updated");
    } catch (error) {
      if (!quiet) setToast(error instanceof Error ? error.message : String(error));
    } finally {
      pullBusy.current = false;
      setPulling(false);
    }
  };
  const refreshKyomu = async (quiet = false) => {
    if (!desktopBridge?.pullSchool || kyomuBusy.current) return;
    kyomuBusy.current = true;
    setKyomuRefreshing(true);
    try {
      const problem = pullProblem(await desktopBridge.pullSchool("kyomu"));
      await workspaceStore.reload();
      void loadSchoolRecords();
      if (!quiet) setToast(problem ?? "教務Web updated");
    } catch (error) {
      if (!quiet) setToast(error instanceof Error ? error.message : String(error));
    } finally {
      kyomuBusy.current = false;
      setKyomuRefreshing(false);
    }
  };
  /** Pulls whatever cache is older than its TTL, for a signed-in account. */
  refreshStale.current = async (profile) => {
    if (readOnly) return;
    const records = await loadSchoolRecords();
    if (!records) return;
    const stale = staleSources(
      records.lastRuns,
      workspaceStore.getState().workspace?.importStatus.timetableReviewedAt,
    );
    if (profile === "science") {
      if (stale.timetable) await pullLms("term", true, currentTerm);
      else if (stale.assignments) await fetchLms(true);
    } else if (stale.kyomu) await refreshKyomu(true);
  };
  /** Opens 教務Web registration with this quarter's timetable as a queue. */
  const openRegistration = async (term: TermId) => {
    const quarter = term.slice(5);
    const items = timetableCourses(data, term)
      .filter((course) => course.code)
      .map((course) => {
        const meeting = course.meetings.find((item) => item.days.length && item.periods.length);
        const day = meeting?.days[0];
        return {
          offeringId: /^\d{9}$/.test(course.offeringId ?? "") ? course.offeringId! : "",
          code: course.code.slice(0, 32),
          title: course.title.slice(0, 200),
          day,
          slot: meeting && day
            ? `${quarter} ${WEEKDAYS[day - 1]}${Math.min(...meeting.periods)}-${Math.max(...meeting.periods)}`
            : `${quarter} ${t("集中・時間未定")}`,
        };
      })
      .slice(0, 40);
    try {
      await desktopBridge?.setKyomuGuide?.(items);
    } catch (error) {
      setToast(error instanceof Error ? error.message : String(error));
    }
    openWeb(KYOMU_REGISTRATION);
  };
  const lmsUrlFor = (course: Course) => {
    if (course.source.provider === "lms" && course.url?.includes("lms.s.isct.ac.jp")) return course.url;
    const row = schoolRecords?.lmsCourses.find(
      (item) =>
        (course.offeringId && item.offeringId === course.offeringId) ||
        (course.source.provider === "lms" && String(item.lmsId) === course.source.externalId),
    );
    return row?.url;
  };

  const navigateTo = async (next: Page) => {
    if (next === pageRef.current) return;
    const request = ++navigationSequence.current;

    const leavingWeb = pageRef.current === "web" && next !== "web";
    if (leavingWeb) {
      try {
        await hideNativeSchoolWeb();
        void desktopBridge?.setKyomuGuide?.([]).catch(() => undefined);
        // LMS/教務 extractors commit directly to SQLite while the official
        // page is open. Reload once on return so the UI reflects that real
        // signed-in snapshot; no mock/import bridge is involved.
        await workspaceStore.reload();
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

  // Pages mount on first visit and then stay mounted, hidden while inactive, so
  // their scroll position, inputs and open panels survive switching pages.
  visitedPages.current.add(page);
  const keepAlive = (name: Page, node: React.ReactNode) =>
    visitedPages.current.has(name) ? (
      <div
        className={`pageSlot${page === name ? "" : " inactive"}`}
        aria-hidden={page !== name}
        key={name}
      >
        <PageErrorBoundary name={name}>{node}</PageErrorBoundary>
      </div>
    ) : null;

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
        className={`workspaceMain${page === "web" ? " webMain" : page === "settings" ? " settingsMain" : page === "courses" ? " coursesMain" : ""}`}
      >
        {keepAlive("today", (
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
            openPlan={(plan) => !readOnly && setPlanEditor(plan)}
            openTask={(assignmentId) => {
              const assignment = data.assignments.find(
                (item) => item.id === assignmentId,
              );
              if (assignment?.sourceUrl) {
                setWebContext({
                  page: "today",
                  label: "Today",
                  url: assignment.sourceUrl,
                });
                void navigateTo("web");
              } else if (assignment) {
                setDrawerTab("assignments");
                setDrawerId(assignment.courseId);
              }
            }}
            hideAssignment={(assignmentId) =>
              academicUpdate((draft) => {
                const assignment = draft.assignments.find(
                  (item) => item.id === assignmentId,
                );
                if (assignment) {
                  assignment.hidden = true;
                  assignment.rev += 1;
                  assignment.updatedAt = stamp();
                  assignment.updatedBy = draft.deviceId;
                }
                return draft;
              })
            }
            openSchedule={() => void navigateTo("schedule")}
          />
        ))}
        {keepAlive("schedule", (
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
            openPlan={(plan) => !readOnly && setPlanEditor(plan)}
            newPlan={() => !readOnly && setPlanEditor("new")}
          />
        ))}
        {keepAlive("courses", (
          <Courses
            side={
              <CourseFinder
                key={courseTerm}
                year={Number(courseTerm.slice(0, 4))}
                term={courseTerm}
                summary={`${finderTermCourses.length} ${t("on timetable")} · ${termCredits} ${t("credits")}`}
                locale={data.settings.locale}
                load={(year) =>
                  desktopBridge?.catalogOfferings
                    ? desktopBridge.catalogOfferings(year)
                    : Promise.resolve([])
                }
                context={finderContext}
                onTimetable={finderPlaced}
                keys={finderKeys}
                setKeys={setFinderKeys}
                openCourse={(course) => {
                  setPanelCourseId(null);
                  setOfferingPanel(course);
                }}
                activeId={offeringPanel?.id}
                replaces={(course) => clashingCourses(course, replaceable).length > 0}
                readOnly={readOnly}
                add={addFinderCourses}
                undo={(rows) =>
                  academicUpdate((draft) => {
                    for (const row of rows) {
                      const course = draft.courses.find(
                        (item) =>
                          isSyllabusCourse(item) &&
                          item.source.externalId === row.id &&
                          item.termId === courseTerm &&
                          !item.deletedAt,
                      );
                      if (course)
                        removeSyllabusDraft(draft, course.id, course.termId);
                    }
                    // Bring back what the add replaced.
                    const now = stamp();
                    for (const courseId of replacedByAdd.current) {
                      const item = draft.planItems
                        .filter((x) => x.courseId === courseId && x.termId === courseTerm && x.deletedAt)
                        .sort((x, y) => (y.deletedAt ?? "").localeCompare(x.deletedAt ?? ""))[0];
                      if (item) {
                        item.deletedAt = undefined;
                        item.rev += 1;
                        item.updatedAt = now;
                        item.updatedBy = draft.deviceId;
                      }
                    }
                    replacedByAdd.current = [];
                    return draft;
                  })
                }
                manual={
                  readOnly
                    ? undefined
                    : () => setCourseEditor({ term: courseTerm })
                }
                query={desktopBridge?.catalogQuery}
                syncDetails={desktopBridge?.catalogSyncOfferings}
              />
            }
            leftPanel={
              offeringPanel && (
                <OfferingPanel
                  key={offeringPanel.id}
                  course={offeringPanel}
                  locale={data.settings.locale}
                  placed={finderPlaced.has(offeringPanel.code)}
                  replaces={clashingCourses(offeringPanel, replaceable).map((c) => c.title)}
                  readOnly={readOnly}
                  close={() => setOfferingPanel(null)}
                  add={() => addFinderCourses([offeringPanel])}
                />
              )
            }
            panel={
              panelCourse && (
                <CoursePanel
                  key={panelCourse.id}
                  course={panelCourse}
                  data={data}
                  readOnly={readOnly}
                  close={() => setPanelCourseId(null)}
                  remove={() => removeFromTimetable(panelCourse)}
                  editIntensive={() => setIntensiveCourseId(panelCourse.id)}
                  lmsUrl={lmsUrlFor(panelCourse)}
                  openWeb={(url) => openWeb(url)}
                  fetchLms={desktopBridge?.pullSchool ? () => void fetchLms() : undefined}
                  fetching={pulling}
                />
              )
            }
            data={data}
            currentTerm={currentTerm}
            readOnly={readOnly}
            update={academicUpdate}
            open={(courseId) => {
              setOfferingPanel(null);
              setPanelCourseId(courseId);
            }}
            openEvent={(event, term) =>
              !readOnly && setEventEditor({ event, term })
            }
            onSlot={toggleFinderSlot}
            activeSlots={finderSlots}
            onIntensive={(append) => pickFinderTime("集中", append)}
            intensiveActive={finderKeys.includes("集中")}
            addTerm={() => setTermDialog(true)}
            pullLms={desktopBridge?.pullSchool && !readOnly ? (scope) => void pullLms(scope) : undefined}
            pulling={pulling}
            register={desktopBridge?.setKyomuGuide ? (term) => void openRegistration(term) : undefined}
            academic={
              <AcademicSummary
                summaries={data.gpaSummary}
                termCredits={termCredits}
                refreshing={kyomuRefreshing}
                refresh={desktopBridge?.pullSchool ? () => void refreshKyomu() : undefined}
                openOfficial={() => openWeb(KYOMU_GPA)}
              />
            }
            exportIcs={(term) => {
              void invoke<string>("export_quarter_ics", { term })
                .then((calendar) => {
                  const url = URL.createObjectURL(
                    new Blob([calendar], { type: "text/calendar" }),
                  );
                  const link = document.createElement("a");
                  link.href = url;
                  link.download = `campus-life-${term}.ics`;
                  link.click();
                  URL.revokeObjectURL(url);
                })
                .catch((error) =>
                  setToast(
                    error instanceof Error
                      ? error.message
                      : "Calendar export failed.",
                  ),
                );
            }}
          />
        ))}
        {keepAlive("web", (
          <Web
            key={webContext?.url ?? "web"}
            active={page === "web"}
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
        ))}
        {keepAlive("settings", (
          <Settings
            platform={platform}
            data={data}
            update={update}
            advancedCalendarUpdate={advancedCalendarUpdate}
            caps={caps}
            section={section}
            setSection={setSection}
            notify={setToast}
            runScheduledTask={executeScheduledTask}
            showLoginPage={(profile) => {
              sessionStorage.setItem(
                "stws-active-school-site",
                profile === "science" ? "science-portal" : "tokyo-portal",
              );
              void navigateTo("web");
            }}
          />
        ))}
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
          editIntensive={() => setIntensiveCourseId(drawerCourse.id)}
          lmsUrl={lmsUrlFor(drawerCourse)}
          openWeb={(url) => {
            const from = pageRef.current;
            setDrawerId(null);
            openWeb(url, from === "schedule" ? "schedule" : "today");
          }}
          fetchLms={desktopBridge?.pullSchool ? () => void fetchLms() : undefined}
          fetching={pulling}
        />
      )}
      {mergeDiff && (
        <LmsMergeDialog
          diff={mergeDiff}
          close={() => {
            setMergeDiff(null);
            workspaceActions.markTimetableReviewed();
          }}
          apply={(add, keep, retime) => {
            const diff = mergeDiff;
            setMergeDiff(null);
            workspaceActions.markTimetableReviewed();
            academicUpdate((draft) => applyLmsMerge(draft, diff, add, keep, retime));
            setToast("Timetable updated from LMS");
          }}
        />
      )}
      {planEditor && (
        <PlanEditor
          key={planEditor === "new" ? "new" : planEditor.id}
          opened
          plan={planEditor === "new" ? undefined : planEditor}
          close={() => setPlanEditor(null)}
          save={(draft) => {
            savePlan(draft, planEditor === "new" ? undefined : planEditor);
            setPlanEditor(null);
            setToast("Plan saved");
          }}
          remove={
            planEditor === "new"
              ? undefined
              : () => {
                  deletePlan(planEditor);
                  setPlanEditor(null);
                  setToast("Plan deleted");
                }
          }
        />
      )}
      {intensiveCourseId &&
        (() => {
          const course = data.courses.find(
            (item) => item.id === intensiveCourseId,
          );
          if (!course) return null;
          const existing = data.intensiveSchedules.find(
            (item) => item.courseId === course.id && !item.deletedAt,
          );
          return (
            <IntensiveEditor
              opened
              value={existing}
              close={() => setIntensiveCourseId(null)}
              save={(value) => {
                academicUpdate((workspace) => {
                  const now = stamp();
                  const target = workspace.intensiveSchedules.find(
                    (item) => item.courseId === course.id && !item.deletedAt,
                  );
                  if (target)
                    Object.assign(target, value, {
                      rev: target.rev + 1,
                      updatedAt: now,
                      updatedBy: workspace.deviceId,
                    });
                  else
                    workspace.intensiveSchedules.push({
                      ...value,
                      id: crypto.randomUUID(),
                      courseId: course.id,
                      rev: 1,
                      updatedAt: now,
                      updatedBy: workspace.deviceId,
                    });
                  return workspace;
                });
                setIntensiveCourseId(null);
                setToast("Intensive dates saved");
              }}
            />
          );
        })()}
      {courseEditor && (
        <CourseEditor
          key={courseEditor.course?.id ?? "new"}
          course={courseEditor.course}
          fetchSyllabus={
            nativeCapabilities?.syllabusSearch === true && desktopBridge
              ? (url) =>
                  desktopBridge!.syllabusCourse(
                    url,
                    data.settings.locale === "ja" ? "ja" : "en",
                  )
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
