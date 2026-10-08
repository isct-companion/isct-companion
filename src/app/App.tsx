import { exportQuarterIcs } from "../core/calendar-ics";
import { contentLanguage, pickLocalized } from "../core/model";
import { displayColor } from "../core/course-colors";
import { decodeEntities } from "../core/text";
import { AppShell, MantineProvider } from "@mantine/core";
import { invoke } from "@tauri-apps/api/core";
import { UiButton } from "../components/mantine";
import i18n, { t } from "../i18n";
import { useTranslation } from "react-i18next";
import { desktopBridge, hideNativeSchoolWeb } from "./native";
import { cleanMaterials, retentionDays } from "../pages/today/lms/files";
import { jstDate, stamp, type Page, type Section } from "./shared";
import {
  onTimetable,
  setTimetable,
  allTerms,
  timetableCourses,
} from "../components/timetable/workspace";

import { Today } from "../pages/today/TodayPage";
import type { CourseEntry } from "../pages/today/lms/LmsWorkspace";
import { LectureDesk } from "../pages/lectures/LectureDesk";
import { Courses } from "../pages/courses/CoursesPage";
import { CourseDrawer } from "../pages/courses/CourseDrawer";
import { Web } from "../pages/web/WebPage";
import { Settings } from "../pages/settings/SettingsPage";
import { PlanEditor, type PlanDraft } from "../pages/schedule/PlanEditor";
import { IntensiveEditor } from "../pages/courses/IntensiveEditor";

import { lazy, Suspense, useEffect, useRef, useState } from "react";

const LmsWorkspace = lazy(() =>
  import("../pages/today/lms/LmsWorkspace").then((module) => ({
    default: module.LmsWorkspace,
  })),
);
import { startReminderRuntime } from "../platform/reminders";
import { workspaceActions, workspaceStore } from "../store/workspace";
import { useWorkspace } from "../store/useWorkspace";
import { detectPlatform, resolveCapabilities } from "../platform/capabilities";
import {
  checkSessionsOnFocus,
  signInAtLaunch,
  useSchoolSession,
  type SchoolSessionState,
} from "../platform/school-session";
import { appTheme, cssVariablesResolver } from "./theme";

import { performAction } from "../platform/actions";
import {
  catalogCourse,
  type NativeCapabilitySnapshot,
  type SchoolPullOptions,
} from "../platform/desktop-bridge";

import type { SchedulerRuntimeState } from "../platform/scheduler";
import { createScheduledTaskRuntime } from "../platform/scheduled-tasks";
import {
  pollReferenceSubscriptions,
  subscriptionNetwork,
  subscriptionPollDue,
  type ReferenceProbeResult,
  type ReferenceRefreshResult,
} from "../platform/subscription-polling";

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

import { staleSources } from "../platform/school-freshness";
import { declaredTermCredits } from "../core/declared-credits";
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
import { newUserRecordId } from "../storage/ids";

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
    ? "Sign-in required. Check the account in Settings → School accounts."
    : failed.message;
}

const LOGIN_STEP_LABELS: Record<string, string> = {
  agree: "Accepting portal terms",
  identifier: "Entering account",
  password: "Checking password",
  method: "Selecting sign-in method",
  otp: "Verifying one-time code",
  matrix: "Verifying matrix code",
};

function loginStepLabel(step: string) {
  return LOGIN_STEP_LABELS[step] ?? step;
}

function sessionStatusLabel(
  state: SchoolSessionState | undefined,
  detail?: string,
) {
  if (state === "active") return "Signed in";
  if (state === "failed") return detail || "Sign-in failed";
  if (state === "not-configured") return "Not configured";
  return detail || "Checking session";
}

function pullScienceLms(options?: SchoolPullOptions) {
  return workspaceActions.pullSchool("science-lms", options);
}

let referenceRefreshInFlight: Promise<string> | undefined;

async function refreshReferenceSubscriptions(force = false): Promise<string> {
  if (referenceRefreshInFlight) return referenceRefreshInFlight;
  referenceRefreshInFlight = (async () => {
    if (!desktopBridge) return "Paused: desktop app required";
    const current = workspaceStore.getState().workspace;
    if (!current) throw new Error("Workspace is not ready.");
    if (!force && !subscriptionPollDue(current))
      return "Up to date · daily check not due";
    const network = subscriptionNetwork(detectPlatform());
    if (network === "offline") return "Paused: offline";
    if (
      network === "cellular" &&
      !current.settings.subscriptionRefresh.allowCellular
    )
      return "Paused: waiting for Wi-Fi";

    const checkedAt = new Date().toISOString();
    const results = await pollReferenceSubscriptions(
      current,
      (indexUrl) =>
        invoke<ReferenceProbeResult>("probe_reference_source", { indexUrl }),
      (kind, indexUrl, year, expectedIndexHash, knownHash) =>
        invoke<ReferenceRefreshResult>("refresh_reference_source", {
          kind,
          indexUrl,
          year,
          expectedIndexHash,
          knownHash,
        }),
    );
    if (results.some((item) => (item.result?.changed ?? 0) > 0))
      await workspaceStore.reload();
    workspaceActions.recordReferencePoll(results, checkedAt);
    await workspaceStore.flush();
    const failed = results.filter((item) => item.error);
    if (failed.length)
      throw new Error(
        `${failed.length} reference source${failed.length === 1 ? "" : "s"} failed`,
      );
    const changed = results.reduce(
      (total, item) => total + (item.result?.changed ?? 0),
      0,
    );
    return changed
      ? `Updated ${changed} reference dataset${changed === 1 ? "" : "s"}`
      : "Reference data is up to date";
  })().finally(() => {
    referenceRefreshInFlight = undefined;
  });
  return referenceRefreshInFlight;
}

async function executeScheduledTask(
  task: ScheduledTask,
  force = false,
): Promise<string> {
  const exportWorkspace = async () => {
    await workspaceStore.flush();
    if (desktopBridge) await desktopBridge.writeScheduledExport();
    else await workspaceActions.exportData();
    return "Export completed";
  };
  const refreshSchool = async () => {
    if (!desktopBridge?.pullSchool)
      throw new Error("School refresh requires the desktop app.");
    const lmsOutcomes = await pullScienceLms();
    if (!lmsOutcomes.length) return "Skipped: school queue is busy";
    const lmsProblem = pullProblem(lmsOutcomes);
    if (lmsProblem) throw new Error(lmsProblem);
    const kyomuOutcomes = await workspaceActions.pullSchool("kyomu");
    if (!kyomuOutcomes.length) return "Skipped: school queue is busy";
    const kyomuProblem = pullProblem(kyomuOutcomes);
    if (kyomuProblem) throw new Error(kyomuProblem);
    await workspaceStore.reload();
    return "School refresh completed";
  };
  const refreshLms = async () => {
    if (!desktopBridge?.pullSchool)
      throw new Error("LMS refresh requires the desktop app.");
    if (!force && typeof document !== "undefined") {
      if (document.visibilityState !== "visible" || !document.hasFocus())
        return "Paused: app is not active";
    }
    if (subscriptionNetwork(detectPlatform()) !== "wifi")
      return "Paused: waiting for Wi-Fi";
    const outcomes = await pullScienceLms();
    if (!outcomes.length) return "Skipped: school queue is busy";
    const problem = pullProblem(outcomes);
    if (problem) throw new Error(problem);
    return "LMS background refresh completed";
  };

  if (task.action === "export") return exportWorkspace();
  if (task.action === "school-refresh") return refreshSchool();
  if (task.action === "lms-refresh") return refreshLms();
  if (task.action === "reference-refresh")
    return refreshReferenceSubscriptions(force);
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

const pages: Page[] = ["home", "lectures", "web"];
// Home orients (today and the week); Lectures is where a course is studied.
const navLabels = {
  en: {
    home: "Home",
    lectures: "Lectures",
    web: "Web",
    settings: "Settings",
  },
  ja: {
    home: "ホーム",
    lectures: "講義",
    web: "Web",
    settings: "設定",
  },
  zh: {
    home: "首页",
    lectures: "讲义",
    web: "网页",
    settings: "设置",
  },
} as const;
const icons: Record<Page, React.ReactElement> = {
  home: (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M4 10.5 12 4l8 6.5V19a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2z" />
      <path d="M9.5 21v-6h5v6" />
    </svg>
  ),
  lectures: (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M5 4h11a3 3 0 0 1 3 3v13H8a3 3 0 0 1-3-3z" />
      <path d="M5 17a3 3 0 0 1 3-3h11M9 8h6" />
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
  const scienceSession = useSchoolSession(desktopBridge, "science");
  const tokyoTechSession = useSchoolSession(desktopBridge, "tokyo-tech");
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
                contentLanguage(sourceData.settings),
                course.title,
              ),
            ),
          instructor:
            course.overrides?.instructor ??
            pickLocalized(
              course.names?.instructor,
              contentLanguage(sourceData.settings),
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
  const [editMode, setEditMode] = useState(false);
  const [page, setPage] = useState<Page>("home");
  const visitedPages = useRef(new Set<Page>());
  const pageRef = useRef<Page>("home");
  const navigateRef = useRef<(next: Page) => Promise<void>>(
    async () => undefined,
  );
  const navigationSequence = useRef(0);
  const [section, setSection] = useState<Section>("accounts");
  const [webContext, setWebContext] = useState<{
    page: "home" | "lectures";
    label: string;
    url: string;
  } | null>(null);
  // The lecture open on Lecture Desk; it stays open while other pages are used.
  const [lectureEntry, setLectureEntry] = useState<CourseEntry | null>(null);
  // Narrow windows show Home's day and week one at a time.
  const [homeView, setHomeView] = useState<"day" | "week">("day");
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
  // Pull progress, 0–100 while running (null when idle).
  const [lmsProgress, setLmsProgress] = useState<number | null>(null);
  const [lmsPhase, setLmsPhase] = useState("Ready");
  const [loginSteps, setLoginSteps] = useState<
    Partial<Record<"science" | "tokyo-tech", string>>
  >({});
  const [kyomuProgress, setKyomuProgress] = useState<number | null>(null);
  const pulling = lmsProgress !== null;
  useEffect(() => {
    const offLogin = desktopBridge?.onSchoolLoginStatus?.((event) => {
      setLoginSteps((current) => ({
        ...current,
        [event.profile]:
          event.state === "connected"
            ? "Signed in"
            : event.state === "progress"
              ? `Signing in · ${loginStepLabel(event.message)}`
              : event.state === "needs-user"
                ? "Waiting for you"
                : event.message,
      }));
    });
    const offIngest = desktopBridge?.onSchoolIngest?.((outcome) => {
      if (outcome.site === "kyomu" && outcome.state === "ok")
        setKyomuProgress((value) =>
          value === null ? value : Math.min(95, Math.max(value, 15) + 20),
        );
    });
    const offProgress = desktopBridge?.onSchoolPullProgress?.((progress) => {
      if (!pullBusy.current) return;
      const ratio = progress.total
        ? Math.min(1, progress.completed / progress.total)
        : 1;
      const ranges = {
        catalog: [5, 8],
        course: [8, 25],
        material: [25, 38],
        assignment: [38, 44],
      } as const;
      const [start, end] = ranges[progress.phase];
      setLmsProgress((current) =>
        current === null
          ? current
          : Math.max(current, start + (end - start) * ratio),
      );
      const course = decodeEntities(progress.courseTitle).trim();
      const item = decodeEntities(progress.itemTitle).trim();
      const context = [progress.term, course, item].filter(Boolean).join(" · ");
      const count = progress.total
        ? `${progress.completed}/${progress.total}`
        : "0/0";
      const label =
        progress.phase === "catalog"
          ? "Reading enrolled courses"
          : progress.phase === "course"
            ? "Loading course"
            : progress.phase === "material"
              ? "Reading material"
              : "Checking assignment";
      setLmsPhase(`${label} · ${context || "LMS"} · ${count}`);
    });
    return () => {
      offLogin?.();
      offIngest?.();
      offProgress?.();
    };
  }, []);
  const pullBusy = useRef(false);
  const fullLmsPull = useRef(false);
  const kyomuBusy = useRef(false);
  const refreshStale = useRef<
    (profile: "science" | "tokyo-tech") => Promise<void>
  >(async () => undefined);
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
      if (
        document.visibilityState === "hidden" ||
        Date.now() - lastFocus < 10 * 60_000
      )
        return;
      lastFocus = Date.now();
      for (const profile of active)
        void refreshStale.current(profile).catch(() => undefined);
    };
    addEventListener("focus", onFocus);
    return () => {
      offSession();
      removeEventListener("focus", onFocus);
    };
  }, []);
  const [mergeDiff, setMergeDiff] = useState<MergeDiff | null>(null);
  const schoolRecords = useWorkspace((state) => state.schoolRecords);
  const loadSchoolRecords = () => workspaceActions.refreshSchool();
  useEffect(
    () =>
      desktopBridge?.onSchoolIngest?.((outcome) => {
        if (outcome.state === "ok" && !pullBusy.current && !kyomuBusy.current)
          void workspaceActions.refreshSchool().catch(() => undefined);
      }),
    [],
  );
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
        .syllabusCourse(course.url!, contentLanguage(sourceData.settings))
        .catch(() => undefined);
  }, [
    sourceData?.selectedTerm,
    sourceData?.settings.locale,
    sourceData?.settings.contentLanguage,
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
    if (desktopBridge?.runScheduledTask) {
      const dispose = desktopBridge.onWorkspaceChanged(() => {
        void workspaceStore.reload().catch(() => undefined);
      });
      return () => {
        dispose();
        unsubscribe();
      };
    }
    // Browser preview has no Rust process; keep the lightweight simulator so
    // the settings UI remains testable without becoming production authority.
    const runtime = createScheduledTaskRuntime({
      getTasks: () => workspaceStore.getState().workspace?.scheduledTasks ?? [],
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

  // Class and assignment reminders, delivered while the app is open.
  useEffect(() => {
    if (!sourceData) return;
    return startReminderRuntime({
      getWorkspace: () => workspaceStore.getState().workspace,
      deliver: ({ title, body }) => {
        if (desktopBridge) return desktopBridge.notify(title, body);
        if ("Notification" in window && Notification.permission === "granted")
          new Notification(title, { body });
      },
    });
  }, [sourceData?.deviceId]);

  // Sign saved school accounts in once at launch. After that, a school page
  // redirected to sign-in is renewed natively, so there is no polling.
  useEffect(() => {
    void signInAtLaunch(desktopBridge);
  }, []);
  // Coming back to the window re-checks the school sessions (at most every 2 min).
  useEffect(() => checkSessionsOnFocus(desktopBridge), []);

  // Once per launch, after settings load: drop downloaded course files left unused.
  const cleaned = useRef(false);
  useEffect(() => {
    if (!data || cleaned.current) return;
    cleaned.current = true;
    void cleanMaterials(retentionDays(data.settings)).catch(() => undefined);
  }, [Boolean(data)]);
  // "Automatic" follows the system while the app is open, not only at launch.
  useEffect(() => {
    if (
      data?.settings.appearance !== "system" ||
      typeof matchMedia !== "function"
    )
      return;
    const query = matchMedia("(prefers-color-scheme: dark)");
    const follow = () => {
      document.documentElement.dataset.mantineColorScheme = query.matches
        ? "dark"
        : "light";
    };
    query.addEventListener("change", follow);
    return () => query.removeEventListener("change", follow);
  }, [data?.settings.appearance]);
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
        const next: Page =
          e.key === "4" ? "settings" : pages[Number(e.key) - 1];
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
  const currentTerm = data.currentTerm;
  // Today and Lecture Desk follow the current quarter once it has courses.
  const todayTerm = data.courses.some(
    (c) => c.termId === currentTerm && !c.deletedAt,
  )
    ? currentTerm
    : data.selectedTerm;
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
            [
              ...(append
                ? current
                : current.filter((item) => !isTimeKey(item))),
              key,
            ],
            finderContext,
          ),
    );
  const toggleFinderSlot = (
    day: number,
    from: number,
    to: number,
    append: boolean,
  ) => {
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
      courseTerm === currentTerm
        ? "curr"
        : `${Number(courseTerm.slice(-2, -1))}Q`,
      ...(ownDepartment?.name.ja ? [ownDepartment.name.ja] : []),
    ]);
  }
  const panelCourse = panelCourseId
    ? data.courses.find((c) => c.id === panelCourseId && !c.deletedAt)
    : undefined;
  // Only local drafts can be replaced; courses registered at school stay put.
  const replaceable = finderTermCourses.filter((course) =>
    data.planItems.some(
      (item) =>
        item.courseId === course.id &&
        item.termId === courseTerm &&
        !item.deletedAt,
    ),
  );
  // 教務Web's own count (e.g. half-credit PE) when it lists this year; else the timetable.
  const termCredits =
    declaredTermCredits(schoolRecords?.registrations ?? [], courseTerm) ??
    finderTermCourses.reduce((sum, course) => sum + course.credits, 0);
  const addFinderCourses = async (rows: FinderCourse[]) => {
    const ids = await Promise.all(
      rows.map(async () => ({
        course: await newUserRecordId(),
        planItem: await newUserRecordId(),
      })),
    );
    academicUpdate((draft) => {
      const quarter = courseTerm.slice(5) as Quarter;
      const replaced: string[] = [];
      for (const [index, row] of rows.entries()) {
        // Add replaces: a planned course in the same periods leaves the timetable.
        // Courses registered at school stay; only local drafts are replaced.
        for (const victim of clashingCourses(
          row,
          timetableCourses(draft, courseTerm),
        ))
          if (removeSyllabusDraft(draft, victim.id, courseTerm))
            replaced.push(victim.id);
        const course = catalogCourse(row, contentLanguage(data.settings));
        // Always into the quarter on screen, so it shows up on this timetable.
        addSyllabusDraft(
          draft,
          {
            ...course,
            year: Number(courseTerm.slice(0, 4)),
            quarter,
          },
          ids[index],
        );
      }
      replacedByAdd.current = replaced;
      return draft;
    });
  };
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
      if (isSyllabusCourse(target) && !school)
        removeSyllabusDraft(d, target.id, target.termId);
      else setTimetable(d, target, false);
      return d;
    });
    setPanelCourseId(null);
  };

  const saveCourse = async (
    draft: CourseDraft,
    term: TermId,
    existing?: Course,
  ) => {
    const [courseId, membershipId] = await Promise.all([
      newUserRecordId(),
      newUserRecordId(),
    ]);
    academicUpdate((d) => {
      const now = stamp();
      let course = existing
        ? d.courses.find((c) => c.id === existing.id)
        : undefined;
      if (!course) {
        course = {
          id: courseId,
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
      setTimetable(d, course, draft.onTimetable, membershipId);
      return d;
    });
  };
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
  const toggleTimetable = async (course: Course) => {
    const membershipId = await newUserRecordId();
    academicUpdate((d) => {
      const target = d.courses.find((c) => c.id === course.id);
      if (target)
        setTimetable(d, target, !onTimetable(d, target), membershipId);
      return d;
    });
  };
  const saveEvent = (
    draft: EventDraft,
    term: TermId,
    existing?: PersonalEvent,
  ) =>
    newUserRecordId().then((eventId) =>
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
            id: eventId,
            kind: "event",
            rev: 1,
            updatedAt: now,
            updatedBy: d.deviceId,
            termId: term,
          });
        return d;
      }),
    );
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
    newUserRecordId().then((planId) =>
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
            id: planId,
            rev: 1,
            updatedAt: now,
            updatedBy: workspace.deviceId,
          });
        return workspace;
      }),
    );
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
  const openWeb = (url: string, from: "home" | "lectures" = "home") => {
    setWebContext({ page: from, label: navigation[from], url });
    void navigateTo("web");
  };
  /** Opens a lecture on Lecture Desk, optionally at one assignment. */
  const openLecture = (
    courseId: string,
    context?: Omit<CourseEntry, "courseId" | "request">,
  ) => {
    setLectureEntry({ courseId, ...context, request: Date.now() });
    void navigateTo("lectures");
  };
  /** LMS pull, then a review dialog when the timetable and LMS differ. */
  // One school pull at a time, whether the person or a TTL started it.
  // `quiet` (TTL refresh) reports only a review dialog, never a toast.
  const pullLms = async (
    scope: "term" | "all",
    quiet = false,
    term = courseTerm,
  ) => {
    if (!desktopBridge?.pullSchool)
      return quiet || setToast("LMS pull needs the desktop app.");
    // A full timetable pull owns the school data pipeline. User and scheduled
    // refreshes arriving while it runs are deliberately dropped, not queued.
    if (pullBusy.current || kyomuBusy.current) return;
    fullLmsPull.current = true;
    pullBusy.current = true;
    setLmsPhase(
      scope === "all" ? "Scanning all LMS quarters" : `Scanning LMS · ${term}`,
    );
    setLmsProgress(5);
    try {
      const outcomes = await pullScienceLms();
      if (!outcomes.length) {
        setLmsPhase("Update skipped · full LMS pull already running");
        return;
      }
      const problem = pullProblem(outcomes);
      if (problem) {
        setLmsPhase(problem);
        return quiet || setToast(problem);
      }
      setLmsPhase("LMS scan complete · reading course data");
      setLmsProgress(45);
      const records = workspaceStore.getState().schoolRecords;
      const lmsCourses = records?.lmsCourses ?? [];
      const years = [
        ...new Set(
          lmsCourses
            .map((row) => row.year)
            .filter((year): year is number => Boolean(year)),
        ),
      ];
      const loadOfferings = async () =>
        new Map(
          (
            await Promise.all(
              years.map(
                (year) =>
                  desktopBridge?.catalogOfferings?.(year).catch(() => []) ?? [],
              ),
            )
          )
            .flat()
            .map((row) => [row.id, row]),
        );
      let offerings = await loadOfferings();
      // Schedules and credits come from the syllabus. Fetch the detail page of
      // every LMS course this pull compares that the catalog lacks (by id, so
      // it works for offerings never seen in a search).
      const wanted = (row: (typeof lmsCourses)[number]) =>
        scope === "all" ||
        lmsTerms(row).includes(term) ||
        (!lmsTerms(row).length && row.year === Number(term.slice(0, 4)));
      const missing = lmsCourses
        .filter(wanted)
        .map((row) => row.offeringId)
        .filter(
          (id): id is string =>
            Boolean(id) && offerings.get(id!)?.detailed !== true,
        );
      if (missing.length) {
        for (let index = 0; index < missing.length; index += 4) {
          setLmsPhase(
            `Loading syllabus details · ${Math.min(index, missing.length)}/${missing.length}`,
          );
          await Promise.all(
            missing
              .slice(index, index + 4)
              .map((id) =>
                desktopBridge
                  ?.syllabusCourse(id, contentLanguage(data.settings))
                  .catch(() => undefined),
              ),
          );
          setLmsProgress(
            45 + (45 * Math.min(index + 4, missing.length)) / missing.length,
          );
          setLmsPhase(
            `Loading syllabus details · ${Math.min(index + 4, missing.length)}/${missing.length}`,
          );
        }
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
                  lmsTerms(
                    row,
                    row.offeringId ? offerings.get(row.offeringId) : undefined,
                  ),
                ),
                ...allTerms(workspace, currentTerm),
              ]),
            ].sort();
      setLmsPhase("Comparing LMS with your timetable");
      setLmsProgress(95);
      const diff = diffLms(workspace, lmsCourses, terms, offerings);
      if (hasDifferences(diff)) {
        setMergeDiff(diff);
        setLmsPhase("Review ready · timetable changes found");
      } else {
        // Still link LMS assignments and materials to the matching courses.
        academicUpdate((draft) =>
          applyLmsMerge(draft, diff, new Set(), new Set()),
        );
        workspaceActions.markTimetableReviewed();
        setLmsPhase(`Up to date · ${diff.common.length} courses matched`);
        if (!quiet) setToast(`Timetable matches LMS (${diff.common.length})`);
      }
    } catch (error) {
      setLmsPhase(
        `Update failed · ${error instanceof Error ? error.message : String(error)}`,
      );
      if (!quiet)
        setToast(error instanceof Error ? error.message : String(error));
    } finally {
      fullLmsPull.current = false;
      pullBusy.current = false;
      setLmsProgress(null);
    }
  };
  /** Assignments and materials only; the timetable stays as it is. */
  const fetchLms = async (quiet = false, lmsCourseId?: number) => {
    if (
      !desktopBridge?.pullSchool ||
      fullLmsPull.current ||
      pullBusy.current ||
      kyomuBusy.current
    )
      return;
    pullBusy.current = true;
    setLmsPhase(
      lmsCourseId
        ? "Refreshing the current LMS course"
        : "Refreshing LMS assignments and materials",
    );
    setLmsProgress(5);
    try {
      const outcomes = await pullScienceLms({ lmsCourseId });
      if (!outcomes.length) {
        setLmsPhase("Update skipped · full LMS pull already running");
        return;
      }
      const problem = pullProblem(outcomes);
      setLmsPhase(problem ?? "Saving LMS changes");
      setLmsProgress(90);

      if (!problem) setLmsPhase("LMS updated");
      if (!quiet) setToast(problem ?? "LMS updated");
    } catch (error) {
      setLmsPhase(
        `Update failed · ${error instanceof Error ? error.message : String(error)}`,
      );
      if (!quiet)
        setToast(error instanceof Error ? error.message : String(error));
    } finally {
      pullBusy.current = false;
      setLmsProgress(null);
    }
  };
  const refreshKyomu = async (quiet = false) => {
    if (
      !desktopBridge?.pullSchool ||
      fullLmsPull.current ||
      pullBusy.current ||
      kyomuBusy.current
    )
      return;
    kyomuBusy.current = true;
    setKyomuProgress(5);
    try {
      const outcomes = await workspaceActions.pullSchool("kyomu");
      if (!outcomes.length) return;
      const problem = pullProblem(outcomes);

      if (!quiet) setToast(problem ?? "教務Web updated");
    } catch (error) {
      if (!quiet)
        setToast(error instanceof Error ? error.message : String(error));
    } finally {
      kyomuBusy.current = false;
      setKyomuProgress(null);
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
        const meeting = course.meetings.find(
          (item) => item.days.length && item.periods.length,
        );
        const day = meeting?.days[0];
        return {
          offeringId: /^\d{9}$/.test(course.offeringId ?? "")
            ? course.offeringId!
            : "",
          code: course.code.slice(0, 32),
          title: course.title.slice(0, 200),
          day,
          slot:
            meeting && day
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
    if (
      course.source.provider === "lms" &&
      course.url?.includes("lms.s.isct.ac.jp")
    )
      return course.url;
    const row = schoolRecords?.lmsCourses.find(
      (item) =>
        (course.offeringId && item.offeringId === course.offeringId) ||
        (course.source.provider === "lms" &&
          String(item.lmsId) === course.source.externalId),
    );
    return row?.url;
  };

  const navigateTo = async (next: Page) => {
    if (next === pageRef.current) return;
    const request = ++navigationSequence.current;

    if (pageRef.current === "lectures" && lectureEntry) {
      try {
        await hideNativeSchoolWeb();
      } catch (error) {
        setToast(String(error));
        return;
      }
      if (request !== navigationSequence.current) return;
    }
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
        className={`workspaceMain${page === "home" ? " homeMain" : page === "lectures" ? " lecturesMain" : page === "web" ? " webMain" : page === "settings" ? " settingsMain" : ""}`}
      >
        {keepAlive(
          "lectures",
          <LectureDesk
            data={data}
            term={todayTerm}
            entry={
              lectureEntry &&
              data.courses.some(
                (c) => c.id === lectureEntry.courseId && !c.deletedAt,
              )
                ? lectureEntry
                : null
            }
            active={page === "lectures"}
            select={(courseId, assignmentId) =>
              (assignmentId
                ? lectureEntry?.assignmentId !== assignmentId
                : lectureEntry?.courseId !== courseId ||
                  Boolean(lectureEntry.assignmentId)) &&
              setLectureEntry({ courseId, assignmentId, request: Date.now() })
            }
            clear={() =>
              void hideNativeSchoolWeb()
                .then(() => setLectureEntry(null))
                .catch((error) => setToast(String(error)))
            }
            renderWorkspace={(entry, active) => {
              const course = data.courses.find(
                (c) => c.id === entry.courseId && !c.deletedAt,
              );
              if (!course) return null;
              const record = schoolRecords?.lmsCourses.find(
                (item) =>
                  (course.offeringId &&
                    item.offeringId === course.offeringId) ||
                  String(item.lmsId) === course.source.externalId,
              );
              return (
                <Suspense fallback={<p role="status">{t("Loading…")}</p>}>
                  <LmsWorkspace
                    course={course}
                    data={data}
                    record={record}
                    entry={entry}
                    active={active}
                    navigationProvided
                    schoolWebAvailable={nativeCapabilities?.schoolWeb === true}
                    close={() => setLectureEntry(null)}
                    refresh={
                      desktopBridge?.pullSchool && record
                        ? () => void fetchLms(false, record.lmsId)
                        : undefined
                    }
                    progress={lmsProgress}
                    markLesson={
                      readOnly
                        ? undefined
                        : (sectionId, occurrence, label) => {
                            academicUpdate((draft) => {
                              const target = draft.courses.find((c) => c.id === course.id);
                              if (!target) return draft;
                              // One mark per class meeting; the latest marks are enough.
                              const log = (target.overrides?.lessonLog ?? []).filter(
                                (mark) => mark.occurrence !== occurrence,
                              );
                              target.overrides = {
                                ...target.overrides,
                                lessonLog: [...log, { occurrence, sectionId }].slice(-60),
                              };
                              target.rev += 1;
                              target.updatedAt = stamp();
                              target.updatedBy = draft.deviceId;
                              return draft;
                            });
                            // Say which class the mark is for: today's, or a later one.
                            const when = new Date(occurrence);
                            const format = (options: Intl.DateTimeFormatOptions) =>
                              new Intl.DateTimeFormat(data.settings.locale, { ...options, hour: "2-digit", minute: "2-digit", hourCycle: "h23", timeZone: "Asia/Tokyo" }).format(when);
                            setToast(
                              occurrence.startsWith(jstDate(new Date()))
                                ? t("Today's {{time}} class is recorded as {{lesson}}.", { time: format({}), lesson: label })
                                : t("Your next class, {{date}}, is set to {{lesson}}.", { date: format({ month: "numeric", day: "numeric", weekday: "short" }), lesson: label }),
                            );
                          }
                    }
                  />
                </Suspense>
              );
            }}
          />,
        )}
        {keepAlive(
          "home",
          <div
            className={`homePage ${homeView}${editMode ? " planning" : ""}`}
          >
            <div className="homeDay">
              <Today
                active={page === "home"}
                data={data}
                term={todayTerm}
                open={(courseId, _tab, context) =>
                  openLecture(courseId, context)
                }
                openEvent={(event) =>
                  !readOnly && setEventEditor({ event, term: event.termId })
                }
                openPlan={(plan) => !readOnly && setPlanEditor(plan)}
                openTask={(assignmentId) => {
                  const assignment = data.assignments.find(
                    (item) => item.id === assignmentId,
                  );
                  if (assignment)
                    openLecture(assignment.courseId, { assignmentId });
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
                openSchedule={() => setHomeView("week")}
              />
            </div>
            <div className="homeWeek">
              <UiButton
                className="textButton homeBack"
                onClick={() => setHomeView("day")}
              >
                ‹ {t("Today")}
              </UiButton>
            <Courses
              editing={editMode}
              onEditingChange={(value) => {
                setEditMode(value);
                setOfferingPanel(null);
                setPanelCourseId(null);
              }}
              openPlan={(plan) => !readOnly && setPlanEditor(plan)}
              newPlan={() => !readOnly && setPlanEditor("new")}
              side={
                <CourseFinder
                  key={courseTerm}
                  year={Number(courseTerm.slice(0, 4))}
                  term={courseTerm}
                  summary={`${finderTermCourses.length} ${t("on timetable")} · ${termCredits} ${t("credits")}`}
                  language={contentLanguage(data.settings)}
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
                  replaces={(course) =>
                    clashingCourses(course, replaceable).length > 0
                  }
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
                          .filter(
                            (x) =>
                              x.courseId === courseId &&
                              x.termId === courseTerm &&
                              x.deletedAt,
                          )
                          .sort((x, y) =>
                            (y.deletedAt ?? "").localeCompare(x.deletedAt ?? ""),
                          )[0];
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
                    language={contentLanguage(data.settings)}
                    placed={finderPlaced.has(offeringPanel.code)}
                    replaces={clashingCourses(offeringPanel, replaceable).map(
                      (c) => c.title,
                    )}
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
                    readOnly={readOnly || !editMode}
                    close={() => setPanelCourseId(null)}
                    remove={() => removeFromTimetable(panelCourse)}
                    editIntensive={() => setIntensiveCourseId(panelCourse.id)}
                    lmsUrl={lmsUrlFor(panelCourse)}
                    openWeb={(url) => openWeb(url)}
                    fetchLms={
                      desktopBridge?.pullSchool
                        ? () => void fetchLms()
                        : undefined
                    }
                    progress={lmsProgress}
                  />
                )
              }
              data={data}
              currentTerm={currentTerm}
              readOnly={readOnly}
              update={academicUpdate}
              open={(courseId) => {
                if (editMode) {
                  setOfferingPanel(null);
                  setPanelCourseId(courseId);
                } else openLecture(courseId);
              }}
              openEvent={(event, term) =>
                !readOnly && setEventEditor({ event, term })
              }
              onSlot={toggleFinderSlot}
              activeSlots={finderSlots}
              onIntensive={(append) => pickFinderTime("集中", append)}
              intensiveActive={finderKeys.includes("集中")}
              addTerm={() => setTermDialog(true)}
              pullLms={
                desktopBridge?.pullSchool && !readOnly
                  ? (scope) => void pullLms(scope)
                  : undefined
              }
              pullProgress={lmsProgress}
              register={
                desktopBridge?.setKyomuGuide
                  ? (term) => void openRegistration(term)
                  : undefined
              }
              academic={
                <AcademicSummary
                  summaries={data.gpaSummary}
                  termCredits={termCredits}
                  progress={kyomuProgress ?? (pulling ? 0 : null)}
                  refresh={
                    desktopBridge?.pullSchool
                      ? () => void refreshKyomu()
                      : undefined
                  }
                  openOfficial={() => openWeb(KYOMU_GPA)}
                />
              }
              exportIcs={(term, scope = "courses") => {
                try {
                  const calendar = exportQuarterIcs(data, term, scope);
                  const url = URL.createObjectURL(
                    new Blob([calendar], { type: "text/calendar" }),
                  );
                  const link = document.createElement("a");
                  link.href = url;
                  link.download = `IsctCompanion-${term}${scope === "courses-and-plans" ? "-with-plans" : ""}.ics`;
                  link.click();
                  setTimeout(() => URL.revokeObjectURL(url), 1000);
                } catch (error) {
                  setToast(
                    error instanceof Error
                      ? error.message
                      : "Calendar export failed.",
                  );
                }
              }}
            />
            </div>
          </div>,
        )}
        {keepAlive(
          "web",
          <Web
            key={webContext?.url ?? "web"}
            active={page === "web"}
            platform={platform}
            data={data}
            update={update}
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
          />,
        )}
        {keepAlive(
          "settings",
          <Settings
            platform={platform}
            data={data}
            update={update}
            academicUpdate={academicUpdate}
            advancedCalendarUpdate={advancedCalendarUpdate}
            caps={caps}
            section={section}
            setSection={setSection}
            notify={setToast}
            runScheduledTask={async (task) => {
              if (!desktopBridge?.runScheduledTask)
                return executeScheduledTask(task, true);
              const result = await desktopBridge.runScheduledTask(task.id);
              await workspaceStore.reload();
              return result;
            }}
            refreshReferences={
              desktopBridge
                ? () => refreshReferenceSubscriptions(true)
                : undefined
            }
            refreshSchool={
              desktopBridge?.pullSchool
                ? async () => {
                    await pullLms("term", false, currentTerm);
                    await refreshKyomu();
                  }
                : undefined
            }
            matrixOcrAvailable={nativeCapabilities?.matrixOcr === true}
            showLoginPage={(profile) => {
              sessionStorage.setItem(
                "isct-companion-active-school-site",
                profile === "science" ? "science-portal" : "tokyo-portal",
              );
              void navigateTo("web");
            }}
          />,
        )}
      </AppShell.Main>
      {drawerCourse && (
        <CourseDrawer
          course={drawerCourse}
          data={data}
          readOnly={readOnly || (page === "home" && !editMode)}
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
            openWeb(url, from === "lectures" ? "lectures" : "home");
          }}
          fetchLms={
            desktopBridge?.pullSchool ? () => void fetchLms() : undefined
          }
          progress={lmsProgress}
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
            academicUpdate((draft) =>
              applyLmsMerge(draft, diff, add, keep, retime),
            );
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
              save={async (value) => {
                const scheduleId = await newUserRecordId();
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
                      id: scheduleId,
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
                    contentLanguage(data.settings),
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
      <aside className="schoolStatusBar" aria-label="School connection status">
        <div className="schoolStatusAccounts">
          <span
            className="schoolStatusAccount"
            data-state={scienceSession ?? "checking"}
          >
            <i aria-hidden="true" />
            <span>
              <b>Science Tokyo</b>
              <small>
                {sessionStatusLabel(scienceSession, loginSteps.science)}
              </small>
            </span>
          </span>
          <span
            className="schoolStatusAccount"
            data-state={tokyoTechSession ?? "checking"}
          >
            <i aria-hidden="true" />
            <span>
              <b>TokyoTech</b>
              <small>
                {sessionStatusLabel(tokyoTechSession, loginSteps["tokyo-tech"])}
              </small>
            </span>
          </span>
        </div>
        <div className="lmsStatus" aria-live="polite">
          <div className="lmsStatusLine">
            <b>LMS</b>
            <span>{lmsPhase}</span>
            {pulling && <output>{Math.round(lmsProgress)}%</output>}
          </div>
          {pulling && (
            <progress aria-label={lmsPhase} max={100} value={lmsProgress} />
          )}
        </div>
      </aside>
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
