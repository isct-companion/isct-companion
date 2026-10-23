import { useDesktopUpdates } from "./useDesktopUpdates";
import { formatJst } from "../lib/civil-date";
import { CalendarView } from "../pages/today/CalendarView";
import { DataNotice } from "../components/DataNotice";
import { ResizableWorkspace } from "../components/ResizableWorkspace";
import { useMediaQuery } from "@mantine/hooks";
import { refresh, syncAvailable, useOpenHint, useSync } from "./useSync";
import { pageProgress, runsFor } from "../platform/sync";
import { useAnimatedNavigate } from "./navigation";
import { SchoolStatusLine } from "./SchoolStatusLine";
import { useCoreLifecycle } from "./useCoreLifecycle";
import { useWorkspaceRuntime } from "./useWorkspaceRuntime";
import { useAppearance } from "./useAppearance";
import {
  scheduleQuery,
  useScheduleQuery,
  useTimetableViews,
  type WorkspaceOverview,
} from "../platform/core-query";
import { useFinder } from "../pages/courses/useFinder";
import type { FinderRowView } from "../platform/finder-query";
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
import { t } from "../i18n";
import { useTranslation } from "react-i18next";
import { desktopBridge, hideNativeSchoolWeb, saveExportFile } from "./native";
import { type Page, type Section } from "./shared";

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
import { workspaceActions, workspaceStore } from "../store/workspace";
import { useWorkspace } from "../store/useWorkspace";
import { detectPlatform, resolveCapabilities } from "../platform/capabilities";
import {
  useSchoolSession,
  type SchoolSessionState,
} from "../platform/school-session";
import { appTheme, cssVariablesResolver } from "./theme";

import { performAction } from "../platform/actions";

import { CourseFinder } from "../pages/courses/CourseFinder";
import { AcademicSummary } from "../pages/courses/AcademicSummary";
import { resolvedTeachingPeriods } from "../lib/timetable-layout";

import { CoursePanel, OfferingPanel } from "../pages/courses/CoursePanel";
import { PageErrorBoundary } from "./PageErrorBoundary";
import { traceRouter } from "./interaction-trace";
import { DAYS as FINDER_DAYS, type FinderCourse } from "../lib/course-query";

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

function sessionStatusLabel(state: SchoolSessionState | undefined) {
  if (state === "active") return t("Signed in");
  if (state === "maintenance") return t("Under maintenance");
  if (state === "failed") return t("Sign-in failed");
  if (state === "not-configured") return t("Not configured");
  return t("Checking session");
}

const pages: Page[] = ["home", "timetable", "lectures", "web"];
// Home orients (today and the week); Lectures is where a course is studied.
const icons: Record<Page, React.ReactElement> = {
  timetable: (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <rect x="3" y="5" width="18" height="16" rx="2" />
      <path d="M7 3v4M17 3v4M3 11h18M9 11v10M15 11v10" />
    </svg>
  ),
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
  const wideHome = useMediaQuery("(min-width: 1001px)");
  const scienceSession = useSchoolSession(desktopBridge, "science");
  const tokyoTechSession = useSchoolSession(desktopBridge, "tokyo-tech");
  const store = useWorkspace((state) => state);
  const sourceData = store.workspace;
  useDesktopUpdates(
    Boolean(sourceData),
    sourceData?.settings.automaticAppUpdates,
  );
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
  const [lectureRailHovered, setLectureRailHovered] = useState(false);
  // Today's "All assignments" asks Lecture Desk for the quarter's list.
  const [assignmentsRequest, setAssignmentsRequest] = useState<number>();
  const [previewTerm, setPreviewTerm] = useState<TermId | null>(null);
  const navigate = useAnimatedNavigate();
  const location = useLocation();
  const params = useParams();
  const timetableMatch = useMatch(routePatterns.timetable);
  const homeMatch = useMatch(`${routePatterns.home}/*`);
  const lecturesMatch = useMatch(`${routePatterns.lectures}/*`);
  const webMatch = useMatch(`${routePatterns.web}/*`);
  const settingsMatch = useMatch(`${routePatterns.settings}/*`);
  const weekMatch = useMatch(routePatterns.week);
  const page: Page = timetableMatch
    ? "timetable"
    : lecturesMatch
      ? "lectures"
      : webMatch
        ? "web"
        : settingsMatch
          ? "settings"
          : "home";
  const courseViewTerm = previewTerm ?? overview.value?.coursesTerm;
  const timetableViews = useTimetableViews(
    overview.value?.allTerms ?? [],
    page === "timetable",
  );
  const cachedTimetable = courseViewTerm
    ? timetableViews.get(courseViewTerm)
    : undefined;
  useEffect(() => {
    const report = (message: string) => {
      void desktopBridge
        ?.recordApplicationError?.(message)
        .catch(() => undefined);
    };
    const error = (event: ErrorEvent) => report(event.message);
    const rejection = (event: PromiseRejectionEvent) =>
      report(
        event.reason instanceof Error
          ? event.reason.message
          : String(event.reason),
      );
    window.addEventListener("error", error);
    window.addEventListener("unhandledrejection", rejection);
    return () => {
      window.removeEventListener("error", error);
      window.removeEventListener("unhandledrejection", rejection);
    };
  }, []);
  useEffect(() => {
    if (previewTerm === overview.value?.coursesTerm) setPreviewTerm(null);
  }, [previewTerm, overview.value?.coursesTerm]);
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
  const [webContext, setWebContext] = useState<{
    page: "home" | "lectures" | "settings";
    label: string;
    url: string;
    request: number;
  } | null>(null);
  const otpSetupRequest = useRef<{ id: number | null; shown: boolean } | null>(
    null,
  );
  useEffect(() => {
    const request = otpSetupRequest.current;
    if (!request) return;
    if (page === "web") request.shown = true;
    else if (request.shown) {
      otpSetupRequest.current = null;
      if (request.id !== null)
        void desktopBridge
          ?.cancelSchoolOtpSetup(request.id)
          .catch(notify.error);
    }
  }, [page]);
  useEffect(
    () => () => {
      const request = otpSetupRequest.current;
      otpSetupRequest.current = null;
      if (request?.id != null)
        void desktopBridge?.cancelSchoolOtpSetup(request.id).catch(() => {});
    },
    [],
  );
  const [otpSetupRevision, setOtpSetupRevision] = useState(0);
  const startOtpSetup = async () => {
    if (!desktopBridge || otpSetupRequest.current) return;
    const request = { id: null as number | null, shown: false };
    otpSetupRequest.current = request;
    const open = async (url: string) => {
      setWebContext({
        page: "settings",
        label: navigation.settings,
        url,
        request: Date.now(),
      });
      await navigate(paths.web());
    };
    try {
      const id = await desktopBridge.beginSchoolOtpSetup();
      if (otpSetupRequest.current !== request) {
        await desktopBridge.cancelSchoolOtpSetup(id);
        return;
      }
      request.id = id;
      await open("https://isct.ex-tic.com/idm/user/mfa_setting/");
      const setup = await desktopBridge.schoolOtpSetup(id);
      if (otpSetupRequest.current !== request) return;
      otpSetupRequest.current = null;
      if (setup.registered) {
        await hideNativeSchoolWeb();
        setWebContext(null);
        await navigate(pagePath("settings"));
        if (
          !(await confirmAction(
            t(
              "App Authentication is already registered at school. Reset it? The old OTP setup will stop working. After automatic setup finishes, copy the new setup from this app to replace the school entry in every other authenticator.",
            ),
            t("Continue to reset"),
            { title: t("Reset school OTP?") },
          ))
        )
          return;
      }
      const enrollment = await desktopBridge.beginSchoolOtpSetup();
      request.id = enrollment;
      request.shown = false;
      otpSetupRequest.current = request;
      await open("https://isct.ex-tic.com/idm/user/mfa_setting/");
      await desktopBridge.completeSchoolOtpSetup(enrollment, setup.registered);
      if (otpSetupRequest.current !== request) return;
      otpSetupRequest.current = null;
      await hideNativeSchoolWeb();
      setWebContext(null);
      await navigate(pagePath("settings"));
      setOtpSetupRevision((revision) => revision + 1);
      notify.info("School OTP is configured and saved.");
    } catch (error) {
      if (otpSetupRequest.current !== request) return;
      otpSetupRequest.current = null;
      if (!(
        error &&
        typeof error === "object" &&
        "kind" in error &&
        error.kind === "cancelled"
      ))
        notify.error(
          t("Could not finish OTP setup. Check the school page and try again."),
        );
    }
  };
  // Hidden Activities keep their last route props so another page's params
  // cannot replace retained subpages. Visible navigation always reads the URL.
  const renderedRoutes = useRef({
    homeView,
    section,
    lectureEntry: null as CourseEntry | null,
    examEntry: null as { key: string; term: TermId; request: number } | null,
  });
  // Timing context and repeated entrances stay in history state, outside the URL.
  // Returning through the page's remembered URL carries no state; it resumes
  // the retained entrance rather than starting a new one.
  const lectureEntry = useMemo<CourseEntry | null>(() => {
    if (!params.courseId) return null;
    const retained = renderedRoutes.current.lectureEntry;
    return {
      ...location.state?.lectureContext,
      courseId: params.courseId,
      assignmentId: params.aid,
      request:
        location.state?.lectureRequest ??
        (retained?.courseId === params.courseId &&
        retained.assignmentId === params.aid
          ? retained.request
          : Date.now()),
    };
  }, [params.courseId, params.aid, location.state?.lectureRequest]);
  const examEntry = useMemo(() => {
    if (!params.examKey || !params.examTerm) return null;
    const retained = renderedRoutes.current.examEntry;
    return {
      key: params.examKey,
      term: params.examTerm as TermId,
      request:
        location.state?.lectureRequest ??
        (retained?.key === params.examKey && retained.term === params.examTerm
          ? retained.request
          : Date.now()),
    };
  }, [params.examKey, params.examTerm, location.state?.lectureRequest]);
  if (page === "home") renderedRoutes.current.homeView = homeView;
  if (page === "settings") renderedRoutes.current.section = section;
  if (page === "lectures") {
    renderedRoutes.current.lectureEntry = lectureEntry;
    renderedRoutes.current.examEntry = examEntry;
  }
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
    });
    if (target) void navigate(target, { replace: true });
  }, [location.pathname, location.search, sourceData?.courses, navigate]);
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
  // The quarter the finder's conditions were made for; switching quarters resets them.
  const [finderTerm, setFinderTerm] = useState<TermId | null>(null);
  const finderCourseTerm = courseViewTerm ?? "2026-1Q";
  const finderYear = Number(finderCourseTerm.slice(0, 4));
  const finderSession = JSON.stringify([location.key, finderCourseTerm]);
  const [activeFinderSession, setActiveFinderSession] = useState<string | null>(
    null,
  );
  const finderActive = activeFinderSession === finderSession;
  const activateFinder = () => setActiveFinderSession(finderSession);
  // The one finder owner: the search panel and the timetable's slot
  // highlight read the same Core request.
  const finder = useFinder({
    year: finderYear,
    term: finderCourseTerm,
    session: finderSession,
    enabled: Boolean(overview.value && page === "timetable" && finderActive),
    defaultKeys:
      (cachedTimetable?.overview ?? overview.value)?.defaultFinderKeys ?? [],
  });
  // A course opened from the Courses timetable, shown over the syllabus panel.
  const [panelCourseId, setPanelCourseId] = useState<string | null>(null);
  // A search result opened from the syllabus panel, shown over the timetable.
  const [offeringPanel, setOfferingPanel] = useState<FinderRowView | null>(
    null,
  );
  const schoolRecords = useWorkspace((state) => state.schoolRecords);
  const sync = useSync();
  // Pages only render Core's runs; the planner decides what is read.
  const lmsProgress = pageProgress(runsFor(sync, ["lms"]));
  const kyomuProgress = pageProgress(runsFor(sync, ["kyomu"]));
  // 教務 pages share one run, so a registrations read shows with the others.
  const timetableProgress = pageProgress(
    runsFor(sync, ["lms.enrolment", "kyomu.page"]),
  );
  useCoreLifecycle();
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
  // The ingest's commit patches the records through `workspace-changed`.
  useEffect(
    () =>
      desktopBridge?.onSchoolIngest?.((outcome) => {
        if (outcome.site === "science-lms" && outcome.state === "ok")
          notify.updated(outcome.changed);
      }),
    [],
  );
  const { schedulerState, nativeCapabilities, nativeCapabilitiesReady } =
    useWorkspaceRuntime(sourceData);
  useAppearance(data);
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
      if (
        (e.metaKey || e.ctrlKey) &&
        ["1", "2", "3", "4", "5"].includes(e.key)
      ) {
        e.preventDefault();
        const next: Page =
          e.key === "5" ? "settings" : pages[Number(e.key) - 1];
        void navigate(pagePath(next));
      }
    };
    addEventListener("keydown", key);
    return () => removeEventListener("keydown", key);
  }, [navigate, closeDrawer]);
  const lmsRecordFor = (course: Course) => {
    const records = schoolRecords?.lmsCourses ?? [];
    return (
      records.find((item) => item.lmsId === course.lmsCourseId) ??
      records.find(
        (item) =>
          (course.offeringId && item.offeringId === course.offeringId) ||
          (course.source.provider === "lms" &&
            String(item.lmsId) === course.source.externalId),
      )
    );
  };
  const openedCourse =
    page === "lectures" && lectureEntry && sourceData
      ? sourceData.courses.find(
          (item) => item.id === lectureEntry.courseId && !item.deletedAt,
        )
      : undefined;
  const openedRecord = openedCourse && lmsRecordFor(openedCourse);
  // Each visible view tells Core once; Core's open threshold decides what
  // is read. A page shown again sends its view again.
  useOpenHint(page === "home" ? { kind: "today" } : null);
  useOpenHint(
    page !== "lectures"
      ? null
      : openedRecord
        ? { kind: "course", lmsCourseId: openedRecord.lmsId }
        : lectureEntry
          ? null
          : { kind: "assignments" },
  );
  useOpenHint(page === "timetable" ? { kind: "timetable" } : null);
  useOpenHint(page === "timetable" ? { kind: "grades" } : null);
  useOpenHint(
    page === "timetable" && finderActive
      ? { kind: "finder", year: finderYear }
      : null,
  );
  // A 教務Web read that lands may settle a semester's physics group.
  const kyomuUpdated =
    sync.sources.find((source) => source.source === "kyomu")?.lastUpdated ??
    null;
  const seenKyomu = useRef<string | null | undefined>(undefined);
  useEffect(() => {
    if (!sync.sources.length) return;
    if (
      seenKyomu.current !== undefined &&
      kyomuUpdated &&
      kyomuUpdated !== seenKyomu.current
    )
      void askClassGroup();
    seenKyomu.current = kyomuUpdated;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kyomuUpdated, sync.sources.length]);
  if (!data || !overview.value)
    return (
      <div className="loading">
        {t(store.error ?? overview.error ?? "Opening your workspace…")}
        {(store.error || overview.error) && (
          <UiButton
            onClick={() =>
              store.error
                ? void workspaceStore.initialize()
                : overview.retry?.()
            }
          >
            {t("Retry")}
          </UiButton>
        )}
      </div>
    );

  const workspaceView =
    page === "timetable"
      ? (cachedTimetable?.overview ?? overview.value)
      : overview.value;
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
  const advancedCalendarUpdate = (fn: (d: Workspace) => Workspace) =>
    performAction("edit-calendar-advanced", platform, () =>
      workspaceActions.editCalendar(fn),
    );
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
    home: t("Today"),
    timetable: t("Timetable"),
    lectures: t("Lectures"),
    web: t("Web"),
    settings: t("Settings"),
  };
  const currentTerm = workspaceView.currentTerm;
  const todayTerm = workspaceView.todayTerm;
  const drawerCourse = drawerId
    ? data.courses.find((c) => c.id === drawerId && !c.deletedAt)
    : undefined;
  const courseTerm = courseViewTerm ?? workspaceView.coursesTerm;
  const finderTermCourses = workspaceView.courses;
  const pickFinderTime = (key: string, append = false, slot?: string) => {
    activateFinder();
    finder.toggleTime(key, append, slot);
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
    if (name)
      pickFinderTime(`${name}${from}-${to}`, append, `${day}:${from}-${to}`);
  };
  // Start from the quarter being planned ("curr" when it is the current one)
  // and your own department; remove the department pill to search everywhere.
  if (finderTerm !== courseTerm) {
    setFinderTerm(courseTerm);
    setPanelCourseId(null);
    setOfferingPanel(null);
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
    setWebContext({
      page: from,
      label: navigation[from],
      url,
      request: Date.now(),
    });
    void navigate(paths.web());
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
  const openExam = (key: string, term: string) => {
    void navigate(paths.exam(key, term), {
      state: { lectureRequest: Date.now() },
    });
  };
  // Refresh buttons ask Core for a scope; progress and failures arrive in
  // the sync snapshot.
  const canSync = syncAvailable() && !readOnly;
  const fetchLms = () => refresh({ kind: "site", site: "science-lms" });
  const refreshKyomu = () => refresh({ kind: "site", site: "kyomu" });
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
      railHovered={lectureRailHovered}
      data={data}
      term={todayTerm}
      examEntry={renderedRoutes.current.examEntry}
      selectExam={openExam}
      entry={
        retainedLecture &&
        data.courses.some(
          (c) => c.id === retainedLecture.courseId && !c.deletedAt,
        )
          ? retainedLecture
          : null
      }
      active={page === "lectures"}
      assignmentsRequest={assignmentsRequest}
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
      select={(courseId, assignmentId, term) =>
        openLecture(courseId, { assignmentId, term })
      }
      renderWorkspace={(entry, active, sidebarToggle) => {
        const course = data.courses.find(
          (c) => c.id === entry.courseId && !c.deletedAt,
        );
        if (!course) return null;
        const record = lmsRecordFor(course);
        return (
          <Suspense fallback={<p role="status">{t("Loading…")}</p>}>
            <LmsWorkspace
              course={course}
              data={data}
              record={record}
              entry={entry}
              active={active}
              navigationProvided
              headerStart={sidebarToggle}
              schoolWebAvailable={nativeCapabilities?.schoolWeb === true}
              close={() =>
                void hideNativeSchoolWeb()
                  .then(() => navigate(paths.lectures()))
                  .catch(notify.error)
              }
              refresh={
                canSync && record
                  ? () => refresh({ kind: "course", lmsCourseId: record.lmsId })
                  : undefined
              }
              progress={
                record
                  ? pageProgress(
                      runsFor(
                        sync,
                        ["lms.course", "lms.assign"],
                        String(record.lmsId),
                      ),
                    )
                  : null
              }
              updatedAt={
                record
                  ? schoolRecords?.freshness[`lms.course/${record.lmsId}`]
                      ?.fetchedAt
                  : null
              }
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
                          formatJst(when, data.settings.locale, {
                            ...options,
                            hour: "2-digit",
                            minute: "2-digit",
                          });
                        notify.info(
                          occurrence.startsWith(tokyoDate(new Date()))
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
  const showSchoolSetup =
    !readOnly &&
    !data.importStatus.timetableReviewedAt &&
    !data.courses.some((course) => !course.deletedAt) &&
    tokyoTechSession !== "active";
  const homePage = (
    <div
      className={`homePage ${renderedRoutes.current.homeView}${showSchoolSetup ? " needsSetup" : ""}`}
    >
      {showSchoolSetup && (
        <aside className="homeSetup" aria-label={t("School account setup")}>
          <div>
            <b>{t("Connect your school accounts")}</b>
            <p>
              {t(
                "Connect Tokyo Tech for your timetable and course search, and Science Tokyo for course materials and assignments.",
              )}
            </p>
          </div>
          <UiButton
            className="button primary"
            onClick={() => void navigate(paths.settings("accounts"))}
          >
            {t("Connect school accounts")}
          </UiButton>
        </aside>
      )}
      <ResizableWorkspace
        id="home"
        enabled={wideHome}
        firstSize={360}
        firstMin={240}
        firstMax="50%"
        secondMin="45%"
      >
        <div className="homeDay">
          <Today
            active={page === "home"}
            data={data}
            term={todayTerm}
            openExam={openExam}
            open={(courseId, _tab, context) => openLecture(courseId, context)}
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
              void academicIntent({
                kind: "setAssignmentMark",
                assignmentId,
                hidden: true,
              })
            }
            openAssignments={() => {
              setAssignmentsRequest(Date.now());
              void navigate(paths.lectures());
            }}
            openSchedule={() => void navigate(paths.home("week"))}
          />
        </div>
        <div className="homeWeek">
          <UiButton
            className="textButton homeBack"
            onClick={() => navigate(paths.home())}
          >
            ‹ {t("Today")}
          </UiButton>
          <CalendarView
            data={data}
            mark={
              readOnly
                ? undefined
                : (block, as) =>
                    void workspaceActions
                      .mutate({ kind: "markSession", key: block.key, mark: as })
                      .catch(notify.error)
            }
            open={(block) => {
              if (block.kind === "exam" && block.termId)
                openExam(block.key, block.termId);
              else if (block.kind === "course")
                openLecture(block.id, {
                  term: block.termId,
                  date: block.date,
                  startTime: block.startTime,
                  endTime: block.endTime,
                });
              else if (block.kind === "plan") {
                const plan = data.plans.find((row) => row.id === block.id);
                if (plan && !readOnly) setPlanEditor(plan);
              } else {
                const event = data.events.find((row) => row.id === block.id);
                if (event && !readOnly)
                  setEventEditor({ event, term: event.termId });
              }
            }}
          />
        </div>
      </ResizableWorkspace>
    </div>
  );
  const timetablePage = (
    <div className="quarterPage">
      {" "}
      <Courses
        openPlan={(plan) => !readOnly && setPlanEditor(plan)}
        newPlan={() => !readOnly && setPlanEditor("new")}
        side={
          <CourseFinder
            key={finderSession}
            enabled={finderActive}
            activate={activateFinder}
            year={Number(courseTerm.slice(0, 4))}
            term={courseTerm}
            summary={`${finderTermCourses.length} ${t("on timetable")} · ${termCredits} ${t("credits")}`}
            language={contentLanguage(data.settings)}
            finder={finder}
            openCourse={(row) => {
              setPanelCourseId(null);
              setOfferingPanel(row);
            }}
            activeId={offeringPanel?.course.id}
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
              readOnly ? undefined : () => setCourseEditor({ term: courseTerm })
            }
          />
        }
        leftPanel={
          offeringPanel && (
            <OfferingPanel
              key={offeringPanel.course.id}
              course={offeringPanel.course}
              language={contentLanguage(data.settings)}
              placed={finder.placedCodes.has(offeringPanel.course.code)}
              replaces={(
                finder.rows.find(
                  (row) => row.course.id === offeringPanel.course.id,
                ) ?? offeringPanel
              ).replaces.map(
                (id) =>
                  data.courses.find((course) => course.id === id)?.title ?? "",
              )}
              readOnly={readOnly}
              close={() => setOfferingPanel(null)}
              add={() => {
                void addFinderCourses([offeringPanel.course]).catch(
                  notify.error,
                );
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
              readOnly={readOnly}
              close={() => setPanelCourseId(null)}
              remove={() => void removeFromTimetable(panelCourse)}
              editIntensive={() => setIntensiveCourseId(panelCourse.id)}
              openWeb={(url) => openWeb(url)}
              fetchLms={canSync ? fetchLms : undefined}
              progress={lmsProgress}
              conflicts={(cachedTimetable?.planning?.conflicts ?? []).filter(
                (conflict) => conflict.courseId === panelCourse.id,
              )}
              openCourse={(courseId) => setPanelCourseId(courseId)}
              findOtherTimes={() => {
                // Class groups share the code without the "-NN" class suffix.
                const key =
                  panelCourse.code.replace(/-\d+$/, "") || panelCourse.title;
                setPanelCourseId(null);
                finder.replace([key]);
                activateFinder();
              }}
            />
          )
        }
        data={data}
        currentTerm={currentTerm}
        term={courseTerm}
        overview={workspaceView}
        cachedPlanning={cachedTimetable?.planning}
        onSelectTerm={(term) => {
          setPreviewTerm(term);
          setActiveFinderSession(null);
        }}
        readOnly={readOnly}
        mutate={async (intent) => {
          if (intent.kind === "selectTerm") setActiveFinderSession(null);
          try {
            return await academicIntentOrThrow(intent);
          } catch (error) {
            if (intent.kind === "selectTerm")
              setPreviewTerm((term) => (term === intent.termId ? null : term));
            notify.error(error);
            throw error;
          }
        }}
        open={(courseId) => {
          setOfferingPanel(null);
          setPanelCourseId(courseId);
        }}
        openEvent={(event, term) =>
          !readOnly && setEventEditor({ event, term })
        }
        onSlot={toggleFinderSlot}
        activeSlots={finder.slots}
        onIntensive={(append) => pickFinderTime("集中", append)}
        intensiveActive={finder.keys.includes("集中")}
        addTerm={() => setTermDialog(true)}
        refreshSchedule={
          canSync ? () => refresh({ kind: "timetable" }) : undefined
        }
        pullProgress={timetableProgress}
        register={
          desktopBridge?.setKyomuGuide
            ? (term) => void openRegistration(term)
            : undefined
        }
        academic={
          <AcademicSummary
            summaries={data.gpaSummary}
            termCredits={termCredits}
            availability={schoolRecords?.pageAvailability?.["kyomu:gpa"]}
            progress={kyomuProgress}
            refresh={canSync ? refreshKyomu : undefined}
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
  );
  const webPage = (
    <Web
      key={webContext?.url ?? "web"}
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
      otpSetupRevision={otpSetupRevision}
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
      setSection={(next) =>
        void navigate(paths.settings(next), { replace: true })
      }
      notify={notify.info}
      runScheduledTask={
        desktopBridge?.runScheduledTask
          ? async (task) => {
              // The run's record arrives as a `scheduler` change.
              return desktopBridge!.runScheduledTask(task.id);
            }
          : undefined
      }
      refreshReferences={syncAvailable()}
      matrixOcrAvailable={nativeCapabilities?.matrixOcr === true}
      showLoginPage={(profile, targetUrl, setupOtp) => {
        if (setupOtp) {
          void startOtpSetup();
          return;
        }
        if (targetUrl) {
          setWebContext({
            page: "settings",
            label: navigation.settings,
            url: targetUrl,
            request: Date.now(),
          });
          void navigate(paths.web());
          return;
        }
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
          readOnly={readOnly}
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
          openWeb={(url) => {
            const from = pageRef.current;
            openWeb(url, from === "lectures" ? "lectures" : "home");
          }}
          fetchLms={canSync ? fetchLms : undefined}
          progress={lmsProgress}
        />
      )}
      {!readOnly && !data.settings.dataNoticeAcknowledgedAt && (
        <DataNotice
          acknowledge={() =>
            update((draft) => ({
              ...draft,
              settings: {
                ...draft.settings,
                dataNoticeAcknowledgedAt: new Date().toISOString(),
              },
            }))
          }
        />
      )}
      {/* Editors show a failed submission inline and stay open. */}
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
          teachingPeriods={resolvedTeachingPeriods(
            data.academicProfile.teachingPeriods,
          )}
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
  return (
    <AppShell
      className="app"
      data-density={data.settings.density}
      // The native window's minimum width always leaves room for the rail.
      navbar={{ width: 72, breakpoint: 0 }}
      padding={0}
    >
      <AppShell.Navbar className="rail">
        <nav aria-label={t("Pages")}>
          {pages.map((p, i) => (
            <UiButton
              className={page === p ? "active" : ""}
              onClick={() => navigatePage(p)}
              key={p}
              onPointerEnter={
                p === "lectures" ? () => setLectureRailHovered(true) : undefined
              }
              onPointerLeave={
                p === "lectures"
                  ? () => setLectureRailHovered(false)
                  : undefined
              }
              onFocus={
                p === "lectures" ? () => setLectureRailHovered(true) : undefined
              }
              onBlur={
                p === "lectures"
                  ? () => setLectureRailHovered(false)
                  : undefined
              }
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
      <AppShell.Main
        component="main"
        className={`workspaceMain${page === "home" ? " homeMain" : page === "timetable" ? " timetableMain" : page === "lectures" ? " lecturesMain" : page === "web" ? " webMain" : page === "settings" ? " settingsMain" : ""}`}
      >
        {keep("lectures", lecturesPage)}
        {keep("home", homePage)}
        {keep("timetable", timetablePage)}
        {keep("web", webPage)}
        {keep("settings", settingsPage)}
      </AppShell.Main>
      {overlays}
      <aside
        className="schoolStatusBar"
        aria-label={t("School connection status")}
      >
        <div className="schoolStatusAccounts">
          <span
            className="schoolStatusAccount"
            data-state={scienceSession ?? "checking"}
          >
            <i aria-hidden="true" />
            <span>
              <b>Science Tokyo</b>
              <small>{sessionStatusLabel(scienceSession)}</small>
            </span>
          </span>
          <span
            className="schoolStatusAccount"
            data-state={tokyoTechSession ?? "checking"}
          >
            <i aria-hidden="true" />
            <span>
              <b>TokyoTech</b>
              <small>{sessionStatusLabel(tokyoTechSession)}</small>
            </span>
          </span>
        </div>
        <SchoolStatusLine
          snapshot={sync}
          locale={data.settings.locale}
          refresh={canSync ? () => refresh({ kind: "everything" }) : undefined}
        />
      </aside>
    </AppShell>
  );
}

let defaultRouter: ReturnType<typeof createHashRouter> | undefined;
const getDefaultRouter = () => {
  if (defaultRouter) return defaultRouter;
  defaultRouter = createHashRouter(workspaceRoutes(WorkspaceApp));
  if (import.meta.env.DEV) traceRouter(defaultRouter);
  return defaultRouter;
};

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
        position="bottom-right"
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
