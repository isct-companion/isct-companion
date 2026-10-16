import {
  scheduleQuery,
  useScheduleQuery,
  type WorkspaceOverview,
} from "../platform/core-query";
import { useFinderEditQueue, useFinderQuery } from "../platform/finder-query";
import { contentLanguage, pickLocalized } from "../lib/model";
import { decodeEntities } from "../lib/text";
import { ModalsProvider } from "@mantine/modals";
import { Notifications } from "@mantine/notifications";
import {
  createHashRouter,
  RouterProvider,
  useBlocker,
  useLocation,
  useMatch,
  useNavigate,
  useParams,
  useSearchParams,
} from "react-router";
import {
  paths,
  routePatterns,
  parseRoute,
  parseDrawer,
  routeRedirect,
  workspaceRoutes,
} from "./routes";
import { notify } from "./notify";
import { AppShell, MantineProvider } from "@mantine/core";
import { UiButton } from "../components/mantine";
import { confirmAction } from "../components/confirm";
import i18n, { t } from "../i18n";
import { useTranslation } from "react-i18next";
import {
  desktopBridge,
  hideNativeSchoolWeb,
  official,
  saveExportFile,
} from "./native";
import { cleanMaterials, retentionDays } from "../features/lms/files";
import { errorText, jstDate, type Page, type Section } from "./shared";

import { Today } from "../pages/today/TodayPage";
import type { CourseEntry } from "../features/lms/LmsWorkspace";
import { LectureDesk } from "../pages/lectures/LectureDesk";
import { Courses } from "../pages/courses/CoursesPage";
import { CourseDrawer } from "../pages/courses/CourseDrawer";
import { Web } from "../pages/web/WebPage";
import { Settings } from "../pages/settings/SettingsPage";
import { PlanEditor, type PlanDraft } from "../pages/schedule/PlanEditor";
import { IntensiveEditor } from "../pages/courses/IntensiveEditor";

import {
  Activity,
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

const LmsWorkspace = lazy(() =>
  import("../features/lms/LmsWorkspace").then((module) => ({
    default: module.LmsWorkspace,
  })),
);
import { bootstrapReminders } from "../platform/reminders";
import { workspaceActions, workspaceStore } from "../store/workspace";
import { useWorkspace } from "../store/useWorkspace";
import {
  detectPlatform,
  resolveCapabilities,
  type CapabilityRuntime,
} from "../platform/capabilities";
import {
  useSchoolSession,
  type SchoolSessionState,
} from "../platform/school-session";
import { appTheme, cssVariablesResolver } from "./theme";

import { performAction } from "../platform/actions";
import {
  schoolPullForDisplay,
  type NativeCapabilitySnapshot,
  type SchoolPullOptions,
  type SchoolSite,
  type PullSnapshot,
} from "../platform/desktop-bridge";

import { subscriptionNetwork } from "../platform/subscription-polling";

import { CourseFinder } from "../pages/courses/CourseFinder";
import { LmsMergeDialog } from "../pages/courses/LmsMergeDialog";
import { AcademicSummary } from "../pages/courses/AcademicSummary";
import { type MergeDiff } from "../lib/lms-merge";

import { CoursePanel, OfferingPanel } from "../pages/courses/CoursePanel";
import { PageErrorBoundary } from "./PageErrorBoundary";
import { useFormFactor } from "./form-factor";
import { MobileShell } from "../layouts/compact/MobileShell";
import { openExternalUrl } from "../components/ExternalLink";
import { DAYS as FINDER_DAYS, type FinderCourse } from "../lib/course-query";
import { isSyllabusCourse } from "../lib/syllabus";

import {
  CourseEditor,
  EventEditor,
  TermDialog,
  type CourseDraft,
  type EventDraft,
} from "../pages/courses/editors";
import type {
  ClassGroupChoice,
  Course,
  PersonalEvent,
  Plan,
  TermId,
  Workspace,
} from "../lib/model";
import { tokyoDate } from "../lib/civil-date";
import { ClassGroupPrompt } from "../pages/courses/ClassGroupPicker";
import type { WorkspaceIntent } from "../lib/workspace-intents";

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

async function refreshReferenceSubscriptions(force = false): Promise<string> {
  if (!desktopBridge?.refreshReferenceSubscriptions)
    return "Paused: desktop app required";
  const current = workspaceStore.getState().workspace;
  if (!current) throw new Error("Workspace is not ready.");
  const network = subscriptionNetwork(detectPlatform());
  if (network === "offline") return "Paused: offline";
  if (
    network === "cellular" &&
    !current.settings.subscriptionRefresh.allowCellular
  )
    return "Paused: waiting for Wi-Fi";
  await workspaceStore.flush();
  try {
    return await desktopBridge.refreshReferenceSubscriptions(force);
  } finally {
    // Failed sources may still accompany successful commits from other sources.
    await workspaceStore.reload();
  }
}

/** A school service named in the route, or null when it names none. */
const schoolSite = (value?: string): SchoolSite | null =>
  value && value in official ? (value as SchoolSite) : null;
const pages: Page[] = ["home", "lectures", "web"];
// Home orients (today and the week); Lectures is where a course is studied.
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

export function WorkspaceApp() {
  useTranslation();
  const formFactor = useFormFactor();
  const scienceSession = useSchoolSession(desktopBridge, "science");
  const tokyoTechSession = useSchoolSession(desktopBridge, "tokyo-tech");
  const store = useWorkspace((state) => state);
  const sourceData = store.workspace;
  const overview = useScheduleQuery<WorkspaceOverview>(
    { kind: "workspaceOverview", term: "" },
    Boolean(sourceData),
  );
  const data = sourceData
    ? {
        ...sourceData,
        courses: sourceData.courses.map((course) => ({
          ...course,
          color: overview.value?.colors[course.id] ?? course.color,
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
    void i18n.changeLanguage(data?.settings.locale ?? "en");
    document.documentElement.lang = data?.settings.locale ?? "en";
  }, [data?.settings.locale]);
  const [editMode, setEditMode] = useState(false);
  const navigate = useNavigate();
  const location = useLocation();
  const params = useParams();
  const homeMatch = useMatch(`${routePatterns.home}/*`);
  const lecturesMatch = useMatch(`${routePatterns.lectures}/*`);
  const webMatch = useMatch(`${routePatterns.web}/*`);
  const settingsMatch = useMatch(`${routePatterns.settings}/*`);
  const weekMatch = useMatch(routePatterns.week);
  const page: Page = lecturesMatch
    ? "lectures"
    : webMatch
      ? "web"
      : settingsMatch
        ? "settings"
        : "home";
  const section = (params.section ?? "accounts") as Section;
  const homeView = homeMatch && weekMatch ? "week" : "day";
  const [searchParams, setSearchParams] = useSearchParams();
  const { courseId: drawerId, tab: drawerTab } = parseDrawer(searchParams);
  const closeDrawer = useCallback(() => {
    if (!searchParams.has("course") && !searchParams.has("tab")) return;
    setSearchParams(
      (current) => {
        const next = new URLSearchParams(current);
        next.delete("course");
        next.delete("tab");
        return next;
      },
      { state: location.state },
    );
  }, [searchParams, setSearchParams, location.state]);
  const [mountedPages, setMountedPages] = useState<Page[]>([page]);
  if (!mountedPages.includes(page)) setMountedPages([...mountedPages, page]);
  const pageRef = useRef<Page>(page);
  pageRef.current = page;
  // Switching pages returns to where each page was left (open lecture,
  // settings section, week view); the course drawer query is not carried.
  const pageLocations = useRef<Partial<Record<Page, string>>>({});
  pageLocations.current[page] = location.pathname;
  const pagePath = (next: Page) =>
    pageLocations.current[next] ?? paths.page(next);
  // The path of each history entry, so going up can reuse the entry below.
  const historyPaths = useRef<string[]>([]);
  {
    const index = window.history.state?.idx ?? 0;
    historyPaths.current[index] = location.pathname;
    historyPaths.current.length = index + 1;
  }
  // On mobile, back means up one level (to `path`), never "wherever I came
  // from". When the entry below is already that parent, step back to it so
  // the system back button keeps a clean history; otherwise replace.
  const closeTo = (path: string) => {
    if (formFactor !== "mobile") return void navigate(path);
    const index = window.history.state?.idx ?? 0;
    if (index > 0 && historyPaths.current[index - 1] === path)
      void navigate(-1);
    else void navigate(path, { replace: true });
  };
  // Official-page extractors write to SQLite; reload only after an ingestion.
  const ingestedOnWeb = useRef(false);
  // Tapping the Web tab again while browsing asks Web for its service menu.
  const [webSitesRequest, setWebSitesRequest] = useState(0);
  const [webContext, setWebContext] = useState<{
    page: "home" | "lectures";
    label: string;
    url: string;
    request: number;
  } | null>(null);
  // Timing context and repeated entrances stay in history state, outside the URL.
  const lectureEntry = useMemo<CourseEntry | null>(
    () =>
      params.courseId
        ? {
            ...location.state?.lectureContext,
            courseId: params.courseId,
            assignmentId: params.aid,
            request: location.state?.lectureRequest ?? Date.now(),
          }
        : null,
    [params.courseId, params.aid, location.state?.lectureRequest],
  );
  // Hidden Activities keep their last route props so another page's params
  // cannot replace retained subpages. Visible navigation always reads the URL.
  const renderedRoutes = useRef({
    homeView,
    section,
    settingsOpen: Boolean(params.section),
    webSite: schoolSite(params.site),
    lectureEntry,
  });
  if (page === "web") renderedRoutes.current.webSite = schoolSite(params.site);
  if (page === "home") renderedRoutes.current.homeView = homeView;
  if (page === "settings") {
    renderedRoutes.current.section = section;
    renderedRoutes.current.settingsOpen = Boolean(params.section);
  }
  if (page === "lectures") renderedRoutes.current.lectureEntry = lectureEntry;
  const retainedLecture = renderedRoutes.current.lectureEntry;
  const blocker = useBlocker(
    ({ nextLocation }) =>
      (page === "web" || (page === "lectures" && Boolean(params.courseId))) &&
      parseRoute(nextLocation.pathname).page !== page,
  );
  useEffect(() => {
    if (blocker.state !== "blocked") return;
    let cancelled = false;
    void hideNativeSchoolWeb()
      .then(() => {
        if (cancelled) return;
        if (page === "web") {
          void desktopBridge?.setKyomuGuide?.([]).catch(() => undefined);
          if (ingestedOnWeb.current) {
            ingestedOnWeb.current = false;
            void workspaceStore.reload().catch(notify.error);
          }
        }
        blocker.proceed();
      })
      .catch((error) => {
        if (cancelled) return;
        notify.error(error);
        blocker.reset();
      });
    return () => {
      cancelled = true;
    };
  }, [blocker, page]);
  useEffect(() => {
    if (!sourceData) return;
    const target = routeRedirect(location.pathname + location.search, {
      courses: sourceData.courses,
      stacked: formFactor === "mobile",
    });
    if (target) void navigate(target, { replace: true });
  }, [
    location.pathname,
    location.search,
    sourceData?.courses,
    navigate,
    formFactor,
  ]);
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
  const finderYear = Number(overview.value?.coursesTerm.slice(0, 4) ?? 2026);
  const finderCourseTerm = overview.value?.coursesTerm ?? "2026-1Q";
  const finderView = useFinderQuery(
    { year: finderYear, term: finderCourseTerm, keys: finderKeys },
    0,
    Boolean(overview.value && editMode && page === "home"),
  );
  const finderEdits = useFinderEditQueue({
    year: finderYear,
    term: finderCourseTerm,
    keys: finderKeys,
    setKeys: setFinderKeys,
  });
  // A course opened from the Courses timetable, shown over the syllabus panel.
  const [panelCourseId, setPanelCourseId] = useState<string | null>(null);
  // A search result opened from the syllabus panel, shown over the timetable.
  const [offeringPanel, setOfferingPanel] = useState<FinderCourse | null>(null);
  // View progress also includes the timetable review after the native pull.
  const [lmsViewProgress, setLmsProgress] = useState<number | null>(null);
  const [nativePulls, setNativePulls] = useState<PullSnapshot[]>([]);
  const [nativeLmsProgress, setNativeLmsProgress] = useState(5);
  const activeLms = schoolPullForDisplay(nativePulls, "science-lms");
  const activeKyomu = schoolPullForDisplay(nativePulls, "kyomu");
  const lmsProgress = activeLms
    ? Math.max(lmsViewProgress ?? 5, nativeLmsProgress)
    : lmsViewProgress;
  const [kyomuViewProgress, setKyomuProgress] = useState<number | null>(null);
  const kyomuProgress = kyomuViewProgress ?? (activeKyomu ? 5 : null);
  useEffect(() => desktopBridge?.onPullState?.(setNativePulls), []);
  useEffect(() => setNativeLmsProgress(5), [activeLms?.id]);
  useEffect(() => {
    if (activeLms?.state === "blocked")
      setLmsPhase(`Waiting · ${activeLms.blockedBy ?? "school view"}`);
    else if (activeLms?.state === "queued")
      setLmsPhase("Waiting for school refresh");
  }, [activeLms?.id, activeLms?.state, activeLms?.blockedBy]);
  const [lmsPhase, setLmsPhase] = useState("Ready");
  const [loginSteps, setLoginSteps] = useState<
    Partial<Record<"science" | "tokyo-tech", string>>
  >({});
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
      setNativeLmsProgress((current) =>
        Math.max(current, start + (end - start) * ratio),
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
  useEffect(() => {
    if (!desktopBridge?.schoolRefreshEvent) return;
    const report = (kind: "foreground" | "background") => {
      void desktopBridge
        ?.schoolRefreshEvent?.({
          kind,
          network: subscriptionNetwork(detectPlatform()),
        })
        .catch(() => undefined);
      void desktopBridge?.cloudLifecycle?.(kind).catch(() => undefined);
    };
    const focus = () => {
      if (document.visibilityState !== "hidden") report("foreground");
    };
    const visibility = () =>
      report(
        document.visibilityState === "hidden" ? "background" : "foreground",
      );
    focus();
    addEventListener("focus", focus);
    addEventListener("online", focus);
    addEventListener("offline", visibility);
    document.addEventListener("visibilitychange", visibility);
    return () => {
      removeEventListener("focus", focus);
      removeEventListener("online", focus);
      removeEventListener("offline", visibility);
      document.removeEventListener("visibilitychange", visibility);
    };
  }, []);
  const [mergeDiff, setMergeDiff] = useState<MergeDiff | null>(null);
  const [groupPrompt, setGroupPrompt] = useState<ClassGroupChoice | null>(null);
  /** After a pull settles the timetable, ask once for the physics group of
   * a current or later semester that has none. */
  const askClassGroup = async () => {
    // The question is a convenience; a failed read must not fail the pull.
    const choices = await scheduleQuery<ClassGroupChoice[]>({
      kind: "classGroups",
      today: tokyoDate(new Date()),
    }).catch(() => []);
    setGroupPrompt(choices.find((choice) => choice.needsChoice) ?? null);
  };
  const schoolRecords = useWorkspace((state) => state.schoolRecords);
  const loadSchoolRecords = () => workspaceActions.refreshSchool();
  useEffect(
    () =>
      desktopBridge?.onSchoolIngest?.((outcome) => {
        if (outcome.state === "ok" && pageRef.current === "web")
          ingestedOnWeb.current = true;
        if (outcome.state === "ok")
          void workspaceActions.refreshSchool().catch(() => undefined);
      }),
    [],
  );
  const [schedulerState, setSchedulerState] =
    useState<CapabilityRuntime["background"]["scheduler"]>("off");
  const [nativeCapabilities, setNativeCapabilities] =
    useState<NativeCapabilitySnapshot>();
  const [nativeCapabilitiesReady, setNativeCapabilitiesReady] = useState(false);
  useEffect(() => {
    if (!desktopBridge) return;
    void desktopBridge
      .getRuntimeCapabilities()
      .then(setNativeCapabilities)
      .catch(() => setNativeCapabilities(undefined))
      .finally(() => setNativeCapabilitiesReady(true));
  }, []);
  // Courses of the quarter on screen, a 3Q–4Q course in each of its quarters.
  const termCourseKey = overview.value?.termCourseIds.join("\n");
  useEffect(() => {
    if (!sourceData || !desktopBridge || !nativeCapabilities?.syllabusSearch)
      return;
    const termCourses = new Set(termCourseKey?.split("\n") ?? []);
    for (const course of sourceData.courses.filter(
      (course) =>
        !course.deletedAt &&
        termCourses.has(course.id) &&
        isSyllabusCourse(course) &&
        course.url,
    ))
      void desktopBridge
        .syllabusCourse(course.url!, contentLanguage(sourceData.settings))
        .catch(() => undefined);
  }, [
    termCourseKey,
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
      let cancelled = false;
      let dispose: (() => void) | undefined;
      void desktopBridge
        .onWorkspaceChanged((event) => {
          void workspaceStore.handleWorkspaceChanged(event).catch(notify.error);
        })
        .then((unlisten) => {
          if (cancelled) {
            unlisten();
            return;
          }
          dispose = unlisten;
          // Listening is asynchronous. Recover the snapshot only after actual
          // registration, closing the startup gap between read and subscribe.
          return workspaceStore.reload();
        })
        .catch(notify.error);
      return () => {
        cancelled = true;
        dispose?.();
        unsubscribe();
      };
    }
    // Preview has no runtime; all task admission and timing belong to Core.
    return unsubscribe;
  }, [sourceData?.deviceId]);

  useEffect(() => {
    if (sourceData && desktopBridge)
      void bootstrapReminders(desktopBridge).catch(notify.error);
  }, [sourceData?.deviceId]);

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
    if (store.error && sourceData)
      notify.error(store.error, () => void workspaceStore.reload());
  }, [store.error, Boolean(sourceData)]);
  useEffect(() => {
    if (pageRef.current !== "web" && !params.courseId)
      void hideNativeSchoolWeb().catch(notify.error);
  }, []);
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeDrawer();
      if ((e.metaKey || e.ctrlKey) && ["1", "2", "3", "4"].includes(e.key)) {
        e.preventDefault();
        const next: Page =
          e.key === "4" ? "settings" : pages[Number(e.key) - 1];
        void navigate(pagePath(next));
      }
    };
    addEventListener("keydown", key);
    return () => removeEventListener("keydown", key);
  }, [navigate, closeDrawer]);
  // Pulls across courses leave material details out (files, pages,
  // announcements); opening a lecture fills in its own, once per session.
  useEffect(() => {
    if (page !== "lectures" || !lectureEntry || !sourceData) return;
    const course = sourceData.courses.find(
      (item) => item.id === lectureEntry.courseId && !item.deletedAt,
    );
    const record =
      course &&
      schoolRecords?.lmsCourses.find(
        (item) =>
          (course.offeringId && item.offeringId === course.offeringId) ||
          String(item.lmsId) === course.source.externalId,
      );
    if (!record) return;
    void desktopBridge
      ?.schoolRefreshEvent?.({ kind: "lecture-opened", courseId: record.lmsId })
      .catch(() => undefined);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page, lectureEntry?.courseId, Boolean(schoolRecords)]);
  if (!data || !overview.value)
    return (
      <div className="loading">
        {store.error ?? overview.error ?? "Opening your workspace…"}
        {(store.error || overview.error) && (
          <UiButton
            onClick={() =>
              store.error
                ? void workspaceStore.initialize()
                : overview.retry?.()
            }
          >
            Retry
          </UiButton>
        )}
      </div>
    );

  const workspaceView = overview.value;
  const update = (edit: (data: Workspace) => Workspace) => {
    void workspaceActions.updateSettings(edit).catch(notify.error);
  };
  const platform = detectPlatform();
  const readOnly = platform === "browser-preview";
  const academicIntent = (intent: WorkspaceIntent) =>
    performAction("edit-academic-data", platform, () =>
      workspaceActions.mutate(intent),
    ).catch((error) => notify.error(error));
  const academicIntentOrThrow = (intent: WorkspaceIntent) =>
    performAction("edit-academic-data", platform, () =>
      workspaceActions.mutate(intent),
    );
  const advancedCalendarUpdate = (fn: (d: Workspace) => Workspace) => {
    void performAction("edit-calendar-advanced", platform, () =>
      workspaceActions.editCalendar(fn),
    ).catch((error) => notify.error(error));
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
  const navigation = {
    home: t("Home"),
    lectures: t("Lectures"),
    web: t("Web"),
    settings: t("Settings"),
  };
  const currentTerm = workspaceView.currentTerm;
  const todayTerm = workspaceView.todayTerm;
  const drawerCourse = drawerId
    ? data.courses.find((c) => c.id === drawerId && !c.deletedAt)
    : undefined;
  const courseTerm = workspaceView.coursesTerm;
  const finderTermCourses = workspaceView.courses;
  const finderPlaced = new Set(finderView.value?.onTimetable ?? []);
  const finderSlots = new Set(finderView.value?.activeSlots ?? []);
  const pickFinderTime = (key: string, append = false) => {
    void finderEdits
      .run({ kind: "toggleTime", key, append })
      .catch(notify.error);
  };
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
    setFinderKeys(workspaceView.defaultFinderKeys);
  }
  const panelCourse = panelCourseId
    ? data.courses.find((c) => c.id === panelCourseId && !c.deletedAt)
    : undefined;
  const termCredits = workspaceView.termCredits;
  const addFinderCourses = async (rows: FinderCourse[]) => {
    await academicIntentOrThrow({
      kind: "addCatalogCourses",
      termId: courseTerm,
      offerings: rows,
    });
  };
  const removeFromTimetable = async (course: Course) => {
    const school = data.memberships.some(
      (item) =>
        item.courseId === course.id &&
        !item.deletedAt &&
        item.evidence?.provider !== "manual",
    );
    if (
      school &&
      !(await confirmAction(
        t(
          "Remove this course from the local timetable? This does not withdraw your school registration.",
        ),
        t("Remove"),
      ))
    )
      return;
    try {
      await academicIntentOrThrow({
        kind: "setCourseOnTimetable",
        courseId: course.id,
        onTimetable: false,
      });
      setPanelCourseId(null);
    } catch (error) {
      notify.error(error);
    }
  };

  const saveCourse = async (
    draft: CourseDraft,
    term: TermId,
    existing?: Course,
  ) => {
    await academicIntentOrThrow({
      kind: "saveCourse",
      draft: {
        title: draft.title.trim(),
        code: draft.code.trim(),
        instructor: draft.instructor.trim(),
        credits: draft.credits,
        instructionLanguage: draft.language,
        color: draft.color,
        url: draft.url.trim() || undefined,
        meetings: draft.meetings,
        onTimetable: draft.onTimetable,
      },
      termId: term,
      existingId: existing?.id,
    });
  };
  const deleteCourse = (course: Course) =>
    academicIntentOrThrow({
      kind: "deleteCourseBatch",
      courseIds: [course.id],
    });
  const toggleTimetable = async (course: Course) => {
    await academicIntent({
      kind: "setCourseOnTimetable",
      courseId: course.id,
      onTimetable: !workspaceView.onTimetableCourseIds.includes(course.id),
    });
  };
  const saveEvent = (
    draft: EventDraft,
    term: TermId,
    existing?: PersonalEvent,
  ) =>
    academicIntentOrThrow({
      kind: "saveEvent",
      draft,
      termId: term,
      existingId: existing?.id,
    });
  const deleteEvent = (event: PersonalEvent) =>
    academicIntentOrThrow({ kind: "deleteEvent", eventId: event.id });
  const savePlan = (draft: PlanDraft, existing?: Plan) =>
    academicIntentOrThrow({
      kind: "savePlan",
      draft,
      existingId: existing?.id,
    });
  const deletePlan = (plan: Plan) =>
    academicIntentOrThrow({ kind: "deletePlan", planId: plan.id });
  const openWeb = (url: string, from: "home" | "lectures" = "home") => {
    // Without an embedded school view the system browser takes over.
    if (
      platform === "native-mobile" &&
      nativeCapabilities?.schoolWeb !== true
    ) {
      void openExternalUrl(url).catch(notify.error);
      return;
    }
    setWebContext({
      page: from,
      label: navigation[from],
      url,
      request: Date.now(),
    });
    // Phones browse inside a service; Web corrects the service once resolved.
    void navigate(
      formFactor === "mobile" ? paths.web("science-lms") : paths.web(),
    );
  };
  /** Opens a lecture on Lecture Desk, optionally at one assignment. */
  const openLecture = (
    courseId: string,
    context?: Omit<CourseEntry, "courseId" | "request">,
  ) => {
    const { assignmentId, ...timing } = context ?? {};
    void navigate(paths.lectures(courseId, assignmentId), {
      state: { lectureRequest: Date.now(), lectureContext: timing },
    });
  };
  /** LMS pull, then a review dialog when the timetable and LMS differ. */
  // All triggers use the native coordinator and receive the shared result.
  // `quiet` (TTL refresh) reports only a review dialog, never a toast.
  const pullLms = async (
    scope: "term" | "all",
    quiet = false,
    term = courseTerm,
  ) => {
    if (!desktopBridge?.pullSchool)
      return quiet || notify.info("LMS pull needs the desktop app.");
    setLmsPhase(
      scope === "all" ? "Scanning all LMS quarters" : `Scanning LMS · ${term}`,
    );
    setLmsProgress(5);
    try {
      const outcomes = await pullScienceLms();
      const problem = pullProblem(outcomes);
      if (problem) {
        setLmsPhase(problem);
        return quiet || notify.info(problem);
      }
      setLmsPhase("LMS scan complete · reading course data");
      setLmsProgress(45);
      let preview = await desktopBridge.lmsMergePreview(scope, term);
      const missing = preview.detailIds;
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
      }
      if (missing.length)
        preview = await desktopBridge.lmsMergePreview(scope, term);
      setLmsPhase("Comparing LMS with your timetable");
      setLmsProgress(95);
      const diff: MergeDiff = {
        ...preview,
        conflicts: new Map(Object.entries(preview.conflicts)),
      };
      if (preview.hasDifferences) {
        setMergeDiff(diff);
        setLmsPhase("Review ready · timetable changes found");
      } else {
        await academicIntentOrThrow({
          kind: "applyLmsMerge",
          terms: diff.terms,
          add: [],
          keep: [],
          retime: [],
        });
        setLmsPhase(`Up to date · ${diff.common.length} courses matched`);
        if (!quiet)
          notify.info(`Timetable matches LMS (${diff.common.length})`);
        await askClassGroup();
      }
    } catch (error) {
      setLmsPhase(`Update failed · ${errorText(error) ?? t("unknown error")}`);
      if (!quiet) notify.error(error);
    } finally {
      setLmsProgress(null);
    }
  };
  /** Assignments and materials only; the timetable stays as it is. */
  const fetchLms = async (quiet = false, lmsCourseId?: number) => {
    if (!desktopBridge?.pullSchool) return false;
    setLmsPhase(
      lmsCourseId
        ? "Refreshing the current LMS course"
        : "Refreshing LMS assignments and materials",
    );
    setLmsProgress(5);
    try {
      const outcomes = await pullScienceLms({ lmsCourseId });
      const problem = pullProblem(outcomes);
      setLmsPhase(problem ?? "Saving LMS changes");
      setLmsProgress(90);

      if (!problem) setLmsPhase("LMS updated");
      if (!quiet) notify.info(problem ?? "LMS updated");
      return !problem;
    } catch (error) {
      setLmsPhase(`Update failed · ${errorText(error) ?? t("unknown error")}`);
      if (!quiet) notify.error(error);
      return false;
    } finally {
      setLmsProgress(null);
    }
  };
  const refreshKyomu = async (quiet = false) => {
    if (!desktopBridge?.pullSchool) return;
    setKyomuProgress(5);
    try {
      const outcomes = await workspaceActions.pullSchool("kyomu");
      const problem = pullProblem(outcomes);

      if (!quiet) notify.info(problem ?? "教務Web updated");
    } catch (error) {
      if (!quiet) notify.error(error);
    } finally {
      setKyomuProgress(null);
    }
  };
  /** Opens 教務Web registration with this quarter's timetable as a queue. */
  const openRegistration = async (term: TermId) => {
    const quarter = term.slice(5);
    const items = (
      await scheduleQuery<Course[]>({ kind: "scheduledCourses", term })
    )
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
      notify.error(error);
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

  const navigatePage = (next: Page) => {
    if (next === "web") setWebContext(null);
    void navigate(pagePath(next));
  };

  const keep = (name: Page, node: React.ReactNode) =>
    mountedPages.includes(name) && (
      <Activity mode={page === name ? "visible" : "hidden"} key={name}>
        <PageErrorBoundary name={name}>{node}</PageErrorBoundary>
      </Activity>
    );
  const lecturesPage = (
    <LectureDesk
      data={data}
      term={todayTerm}
      entry={
        retainedLecture &&
        data.courses.some(
          (c) => c.id === retainedLecture.courseId && !c.deletedAt,
        )
          ? retainedLecture
          : null
      }
      active={page === "lectures"}
      restoreAssignment={
        readOnly
          ? undefined
          : (assignmentId) =>
              void academicIntent({
                kind: "setAssignmentMark",
                assignmentId,
                hidden: false,
              })
      }
      select={(courseId, assignmentId) => {
        if (
          lectureEntry?.courseId !== courseId ||
          lectureEntry?.assignmentId !== assignmentId
        )
          openLecture(courseId, { assignmentId });
      }}
      clear={() =>
        void hideNativeSchoolWeb()
          .then(() => closeTo(paths.lectures()))
          .catch(notify.error)
      }
      renderWorkspace={(entry, active) => {
        const course = data.courses.find(
          (c) => c.id === entry.courseId && !c.deletedAt,
        );
        if (!course) return null;
        const record = schoolRecords?.lmsCourses.find(
          (item) =>
            (course.offeringId && item.offeringId === course.offeringId) ||
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
              openOfficial={
                formFactor === "mobile"
                  ? (url) => openWeb(url, "lectures")
                  : undefined
              }
              schoolWebAvailable={nativeCapabilities?.schoolWeb === true}
              close={() =>
                void hideNativeSchoolWeb()
                  .then(() => closeTo(paths.lectures()))
                  .catch(notify.error)
              }
              refresh={
                desktopBridge?.pullSchool && record
                  ? () => void fetchLms(false, record.lmsId)
                  : undefined
              }
              progress={lmsProgress}
              markLesson={
                readOnly
                  ? undefined
                  : async (sectionId, occurrence, label) => {
                      try {
                        await academicIntentOrThrow({
                          kind: "recordLesson",
                          courseId: course.id,
                          occurrence,
                          sectionId,
                        });
                        // Say which class the mark is for: today's, or a later one.
                        const when = new Date(occurrence);
                        const format = (options: Intl.DateTimeFormatOptions) =>
                          new Intl.DateTimeFormat(data.settings.locale, {
                            ...options,
                            hour: "2-digit",
                            minute: "2-digit",
                            hourCycle: "h23",
                            timeZone: "Asia/Tokyo",
                          }).format(when);
                        notify.info(
                          occurrence.startsWith(jstDate(new Date()))
                            ? t(
                                "Today's {{time}} class is recorded as {{lesson}}.",
                                { time: format({}), lesson: label },
                              )
                            : t(
                                "Your next class, {{date}}, is set to {{lesson}}.",
                                {
                                  date: format({
                                    month: "numeric",
                                    day: "numeric",
                                    weekday: "short",
                                  }),
                                  lesson: label,
                                },
                              ),
                        );
                      } catch (error) {
                        notify.error(error);
                      }
                    }
              }
            />
          </Suspense>
        );
      }}
    />
  );
  // Phones view the timetable; planning stays on the workstation.
  const planning = editMode && formFactor !== "mobile";
  const homePage = (
    <div
      className={`homePage ${renderedRoutes.current.homeView}${planning ? " planning" : ""}`}
    >
      <div className="homeDay">
        <Today
          active={page === "home"}
          data={data}
          term={todayTerm}
          open={(courseId, _tab, context) => openLecture(courseId, context)}
          openEvent={(event) =>
            !readOnly && setEventEditor({ event, term: event.termId })
          }
          openPlan={(plan) => !readOnly && setPlanEditor(plan)}
          openTask={(assignmentId) => {
            const assignment = data.assignments.find(
              (item) => item.id === assignmentId,
            );
            if (assignment) openLecture(assignment.courseId, { assignmentId });
          }}
          pinAssignment={(assignmentId) =>
            void academicIntent({
              kind: "setAssignmentMark",
              assignmentId,
              pinned: !data.assignments.find((item) => item.id === assignmentId)
                ?.pinned,
            })
          }
          hideAssignment={(assignmentId) =>
            void academicIntent({
              kind: "setAssignmentMark",
              assignmentId,
              hidden: true,
            })
          }
          openSchedule={() => void navigate(paths.home("week"))}
        />
      </div>
      <div className="homeWeek">
        <UiButton
          className="textButton homeBack"
          onClick={() => closeTo(paths.home())}
        >
          ‹ {t("Today")}
        </UiButton>
        <Courses
          compact={formFactor === "mobile"}
          editing={planning}
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
              keys={finderKeys}
              edit={finderEdits.run}
              replaceKeys={finderEdits.replace}
              openCourse={(course) => {
                setPanelCourseId(null);
                setOfferingPanel(course);
              }}
              activeId={offeringPanel?.id}
              readOnly={readOnly}
              add={addFinderCourses}
              undo={(rows) =>
                academicIntentOrThrow({
                  kind: "undoCatalogCourses",
                  termId: courseTerm,
                  offeringIds: rows.map((row) => row.id),
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
                replaces={(
                  finderView.value?.replacements[offeringPanel.id] ?? []
                ).map(
                  (id) =>
                    data.courses.find((course) => course.id === id)?.title ??
                    "",
                )}
                readOnly={readOnly}
                close={() => setOfferingPanel(null)}
                add={() => {
                  void addFinderCourses([offeringPanel]).catch(notify.error);
                }}
              />
            )
          }
          panel={
            panelCourse && (
              <CoursePanel
                key={panelCourse.id}
                course={panelCourse}
                data={data}
                readOnly={readOnly || !planning}
                close={() => setPanelCourseId(null)}
                remove={() => void removeFromTimetable(panelCourse)}
                editIntensive={() => setIntensiveCourseId(panelCourse.id)}
                lmsUrl={lmsUrlFor(panelCourse)}
                openWeb={(url) => openWeb(url)}
                fetchLms={
                  desktopBridge?.pullSchool ? () => void fetchLms() : undefined
                }
                progress={lmsProgress}
              />
            )
          }
          data={data}
          currentTerm={currentTerm}
          readOnly={readOnly}
          mutate={async (intent) => {
            try {
              return await academicIntentOrThrow(intent);
            } catch (error) {
              notify.error(error);
              throw error;
            }
          }}
          open={(courseId) => {
            if (planning) {
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
          exportIcs={(term, scope = "courses") =>
            saveExportFile(
              `IsctCompanion-${term}${scope === "courses-and-plans" ? "-with-plans" : ""}.ics`,
              () =>
                scheduleQuery<string>({
                  kind: "quarterIcs",
                  term,
                  scope,
                  generatedAt: new Date().toISOString(),
                }),
              notify.info,
            )
          }
        />
      </div>
    </div>
  );
  const webPage = (
    <Web
      key={webContext?.url ?? "web"}
      layout={formFactor === "mobile" ? "mobile" : "workstation"}
      mobileSite={renderedRoutes.current.webSite}
      sitesRequest={webSitesRequest}
      openMobileSite={(site, replace) =>
        site
          ? void navigate(paths.web(site), { replace })
          : closeTo(paths.web())
      }
      active={page === "web"}
      platform={platform}
      data={data}
      update={update}
      notify={notify.info}
      schoolWebAvailable={nativeCapabilities?.schoolWeb === true}
      requestedUrl={webContext?.url}
      request={webContext?.request}
      returnLabel={webContext?.label}
      onReturn={
        webContext
          ? () => {
              const destination = webContext.page;
              setWebContext(null);
              void navigate(pagePath(destination));
            }
          : undefined
      }
    />
  );
  const settingsPage = (
    <Settings
      mutate={workspaceActions.mutate}
      platform={platform}
      data={data}
      update={update}
      advancedCalendarUpdate={advancedCalendarUpdate}
      caps={caps}
      capabilitiesReady={
        platform !== "native-desktop" || nativeCapabilitiesReady
      }
      section={renderedRoutes.current.section}
      layout={
        formFactor !== "mobile"
          ? "split"
          : renderedRoutes.current.settingsOpen
            ? "detail"
            : "list"
      }
      back={() => closeTo(paths.settings())}
      setSection={(next) =>
        void navigate(paths.settings(next), {
          replace: formFactor !== "mobile",
        })
      }
      notify={notify.info}
      runScheduledTask={
        desktopBridge?.runScheduledTask
          ? async (task) => {
              const result = await desktopBridge!.runScheduledTask(task.id);
              await workspaceStore.reload();
              return result;
            }
          : undefined
      }
      refreshReferences={
        desktopBridge ? () => refreshReferenceSubscriptions(true) : undefined
      }
      matrixOcrAvailable={nativeCapabilities?.matrixOcr === true}
      showLoginPage={(profile) => {
        sessionStorage.setItem(
          "isct-companion-active-school-site",
          profile === "science" ? "science-portal" : "tokyo-portal",
        );
        void navigate(paths.web());
      }}
    />
  );
  const overlays = (
    <>
      {drawerCourse && (
        <CourseDrawer
          course={drawerCourse}
          data={data}
          readOnly={readOnly || (page === "home" && !planning)}
          onTimetable={workspaceView.onTimetableCourseIds.includes(
            drawerCourse.id,
          )}
          initialTab={drawerTab}
          onTabChange={(tab) =>
            void navigate(
              paths.drawer(location.pathname + location.search, drawerId, tab),
              { replace: true, state: location.state },
            )
          }
          close={() => closeDrawer()}
          edit={() => {
            closeDrawer();
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
            openWeb(url, from === "lectures" ? "lectures" : "home");
          }}
          fetchLms={
            desktopBridge?.pullSchool ? () => void fetchLms() : undefined
          }
          progress={lmsProgress}
        />
      )}
      {/* Editors show a failed submission inline and stay open. */}
      {mergeDiff && (
        <LmsMergeDialog
          diff={mergeDiff}
          close={async () => {
            await workspaceActions.markTimetableReviewed();
            setMergeDiff(null);
            await askClassGroup();
          }}
          apply={async (add, keep, retime) => {
            const diff = mergeDiff;
            await academicIntentOrThrow({
              kind: "applyLmsMerge",
              terms: diff.terms,
              add: [...add],
              keep: [...keep],
              retime: [...retime],
            });
            setMergeDiff(null);
            notify.info("Timetable updated from LMS");
            await askClassGroup();
          }}
        />
      )}
      {groupPrompt && (
        <ClassGroupPrompt
          choice={groupPrompt}
          later={() => setGroupPrompt(null)}
          choose={(group) => {
            setGroupPrompt(null);
            void workspaceActions
              .setClassGroup(groupPrompt.semester, group)
              .catch(notify.error);
          }}
        />
      )}
      {planEditor && (
        <PlanEditor
          key={planEditor === "new" ? "new" : planEditor.id}
          opened
          plan={planEditor === "new" ? undefined : planEditor}
          close={() => setPlanEditor(null)}
          save={async (draft) => {
            await savePlan(
              draft,
              planEditor === "new" ? undefined : planEditor,
            );
            setPlanEditor(null);
            notify.info("Plan saved");
          }}
          remove={
            planEditor === "new"
              ? undefined
              : async () => {
                  await deletePlan(planEditor);
                  setPlanEditor(null);
                  notify.info("Plan deleted");
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
                await academicIntentOrThrow({
                  kind: "saveIntensiveSchedule",
                  courseId: course.id,
                  draft: value,
                });
                setIntensiveCourseId(null);
                notify.info("Intensive dates saved");
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
            courseEditor.course
              ? workspaceView.onTimetableCourseIds.includes(
                  courseEditor.course.id,
                )
              : true
          }
          onClose={() => setCourseEditor(null)}
          onSave={async (draft) => {
            await saveCourse(draft, courseEditor.term, courseEditor.course);
            setCourseEditor(null);
            notify.info(courseEditor.course ? "Course saved" : "Course added");
          }}
          onDelete={
            courseEditor.course
              ? async () => {
                  await deleteCourse(courseEditor.course!);
                  setCourseEditor(null);
                  notify.info("Course deleted");
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
          onSave={async (draft) => {
            await saveEvent(draft, eventEditor.term, eventEditor.event);
            setEventEditor(null);
            notify.info("Event saved");
          }}
          onDelete={
            eventEditor.event
              ? async () => {
                  await deleteEvent(eventEditor.event!);
                  setEventEditor(null);
                  notify.info("Event deleted");
                }
              : undefined
          }
        />
      )}
      {termDialog && (
        <TermDialog
          existing={workspaceView.allTerms}
          defaultYear={Number(currentTerm.slice(0, 4))}
          onClose={() => setTermDialog(false)}
          onAdd={async (term) => {
            try {
              await academicIntentOrThrow({ kind: "addTerm", termId: term });
              setTermDialog(false);
            } catch (error) {
              notify.error(error);
            }
          }}
        />
      )}
    </>
  );
  const sessionLabel = (profile: "science" | "tokyo-tech") =>
    profile === "science"
      ? sessionStatusLabel(scienceSession, loginSteps.science)
      : sessionStatusLabel(tokyoTechSession, loginSteps["tokyo-tech"]);

  // The screen pushed over the current tab's root, closed by an edge swipe.
  const mobileBack =
    page === "settings" && params.section
      ? () => closeTo(paths.settings())
      : page === "home" && homeView === "week"
        ? () => closeTo(paths.home())
        : page === "lectures" && params.courseId
          ? () =>
              void hideNativeSchoolWeb()
                .then(() => closeTo(paths.lectures()))
                .catch(notify.error)
          : page === "web" && params.site
            ? () => closeTo(paths.web())
            : undefined;
  if (formFactor === "mobile")
    return (
      <MobileShell
        webMenu={
          params.site
            ? () => setWebSitesRequest((count) => count + 1)
            : undefined
        }
        back={mobileBack}
        page={page}
        density={data.settings.density}
        labels={navigation}
        icons={icons}
        openPage={(next) =>
          next === "settings"
            ? void navigate(pagePath("settings"))
            : navigatePage(next)
        }
        openRoot={(root) =>
          root === "lectures"
            ? void hideNativeSchoolWeb()
                .then(() => closeTo(paths.lectures()))
                .catch(notify.error)
            : closeTo(paths.page(root))
        }
        pages={{
          home: keep("home", homePage),
          lectures: keep("lectures", lecturesPage),
          web: keep("web", webPage),
          settings: keep("settings", settingsPage),
        }}
        overlays={overlays}
        status={{
          accounts: [
            {
              profile: "science",
              name: "Science Tokyo",
              state: scienceSession,
              label: sessionLabel("science"),
            },
            {
              profile: "tokyo-tech",
              name: "TokyoTech",
              state: tokyoTechSession,
              label: sessionLabel("tokyo-tech"),
            },
          ],
          lmsPhase,
          progress: lmsProgress,
          refresh:
            desktopBridge?.pullSchool && !readOnly
              ? () => void fetchLms()
              : undefined,
          openAccounts: () => void navigate(paths.settings("accounts")),
        }}
      />
    );

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
          onClick={() => void navigate(pagePath("settings"))}
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
            onClick={() => void navigate(pagePath("settings"))}
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
        {keep("lectures", lecturesPage)}
        {keep("home", homePage)}
        {keep("web", webPage)}
        {keep("settings", settingsPage)}
      </AppShell.Main>
      {overlays}
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
    </AppShell>
  );
}

let defaultRouter: ReturnType<typeof createHashRouter> | undefined;
const getDefaultRouter = () =>
  (defaultRouter ??= createHashRouter(workspaceRoutes(WorkspaceApp)));

export default function App({
  router: providedRouter,
}: {
  router?: ReturnType<typeof createHashRouter>;
} = {}) {
  // Router construction installs history listeners; reuse it through StrictMode renders.
  const [router] = useState(() => providedRouter ?? getDefaultRouter());
  return (
    <MantineProvider
      theme={appTheme}
      cssVariablesResolver={cssVariablesResolver}
      defaultColorScheme="light"
    >
      <Notifications
        position="bottom-center"
        className="appNotifications"
        limit={1}
        transitionDuration={0}
      />
      <ModalsProvider>
        <RouterProvider router={router} />
      </ModalsProvider>
    </MantineProvider>
  );
}
