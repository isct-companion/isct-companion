import { clearZoom } from "../components/timetable/course-zoom";
import {
  BookOpen,
  CalendarDays,
  Files,
  Globe,
  House,
  Info,
  SlidersHorizontal,
  Wrench,
} from "lucide-react";
import { useDesktopUpdates } from "./useDesktopUpdates";
import { CalendarView } from "../pages/today/CalendarView";
import { DataNotice } from "../components/DataNotice";
import {
  ResizableWorkspace,
  WorkspaceSidebarToggle,
} from "../components/ResizableWorkspace";
import { TitlebarNav } from "../components/TitleRow";
import { usePanelRef } from "react-resizable-panels";
import { WelcomeSheet } from "../components/guide";
import { refresh, syncAvailable } from "../platform/sync-intents";
import { useOpenHint } from "./useOpenHint";
import { useSync } from "../store/sync";
import { useAnimatedNavigate } from "./navigation";
import { SchoolStatusDot } from "./SchoolStatus";
import { useCoreLifecycle } from "./useCoreLifecycle";
import { useWorkspaceRuntime } from "./useWorkspaceRuntime";
import { useAppearance } from "./useAppearance";
import {
  scheduleQuery,
  useScheduleQuery,
  type WorkspaceOverview,
} from "../platform/core-query";
import { Notifications } from "../components/notifications/notifications";
import {
  createHashRouter,
  RouterProvider,
  useBlocker,
  useLocation,
  useMatch,
  useParams,
} from "react-router";
import {
  paths,
  routePatterns,
  parseRoute,
  routeRedirect,
  workspaceRoutes,
} from "./routes";
import { notify } from "../components/notifications/notify";
import { UiButton } from "../components/ui";
import { ConfirmDialogs } from "../components/confirm";
import { t } from "../i18n";
import { useTranslation } from "react-i18next";
import { desktopBridge } from "../platform/native";
import { hideNativeSchoolWeb } from "../platform/school-web-owner";
import { type Page, type Section } from "../lib/route-contract";
import { Today } from "../pages/today/TodayPage";
import type { CourseEntry } from "../features/lms";
import {
  LectureWorkspacePage,
  lectureOpenView,
} from "../pages/lectures/LectureWorkspacePage";
import { useCoursePlanningTerm } from "../pages/courses/useCoursePlanningTerm";
import { CoursePlanningPage } from "../pages/courses/CoursePlanningPage";
import { CourseEditors } from "../pages/courses/CourseEditors";
import { courseEditors } from "../store/course-editors";
import { Web } from "../pages/web/WebPage";
import { Settings } from "../pages/settings/SettingsPage";
import { saveVpnOnDemand } from "../store/vpn-settings";
import { vpnNoticeText } from "../lib/vpn-text";
import { About } from "../pages/about/AboutPage";
import { Tools } from "../pages/tools/ToolsPage";
import { PastExams } from "../pages/past-exams/PastExamsPage";
import { availableTools } from "../platform/available-tools";
import { useEffect, useMemo, useRef, useState } from "react";
import { workspaceActions, workspaceStore } from "../store/workspace";
import { useWorkspace } from "../store/useWorkspace";
import { detectPlatform, resolveCapabilities } from "../platform/capabilities";
import { useSchoolSession } from "../platform/school-session";
import type { SchoolProfileSession } from "../generated/persistence";
import { onWidgetRoute } from "../platform/widget-links";
import { performAction } from "../platform/actions";
import { PageErrorBoundary } from "./PageErrorBoundary";
import { KeptPage } from "./KeptPage";
import { traceRouter } from "./interaction-trace";
import type { ClassGroupChoice, TermId, Workspace } from "../lib/model";
import { tokyoDate } from "../lib/civil-date";
import { wallNow } from "../platform/use-clock";
import { useRecords } from "../platform/records";
import { ClassGroupPrompt } from "../pages/courses/ClassGroupPicker";
import type { WorkspaceIntent } from "../lib/workspace-intents";

/** Credentials are saved and the school has not turned them down. */
const connectedSession = (session: SchoolProfileSession | undefined) =>
  session !== undefined &&
  session.state !== "not-configured" &&
  session.state !== "refused";

const pages: Page[] = [
  "home",
  "timetable",
  "lectures",
  "web",
  "tools",
  "pastExams",
];
// Home orients (today and the week); Lectures is where a course is studied.
const icons: Record<Page, React.ReactElement> = {
  timetable: <CalendarDays aria-hidden strokeWidth={1.75} />,
  home: <House aria-hidden strokeWidth={1.75} />,
  lectures: <BookOpen aria-hidden strokeWidth={1.75} />,
  web: <Globe aria-hidden strokeWidth={1.75} />,
  tools: <Wrench aria-hidden strokeWidth={1.75} />,
  pastExams: <Files aria-hidden strokeWidth={1.75} />,
  settings: <SlidersHorizontal aria-hidden strokeWidth={1.75} />,
  about: <Info aria-hidden strokeWidth={1.75} />,
};

/** What the shell itself renders: course names and colours, the editors it
 * opens, sync and import state, and the school records its status line
 * shows. */
const SHELL_RECORDS = [
  "courses",
  "memberships",
  "events",
  "intensiveSchedules",
  "assignments",
  "gpaSummary",
  "importStatus",
  "syncStatus",
  "academicProfile",
  "lmsCourses",
  "freshness",
  "pageAvailability",
] as const;

export function WorkspaceApp() {
  useTranslation();
  // Today folds away beside the calendar as the lecture list does.
  const todayPanel = usePanelRef();
  const [todayFolded, setTodayFolded] = useState(false);
  const scienceSession = useSchoolSession(desktopBridge, "science");
  const tokyoTechSession = useSchoolSession(desktopBridge, "tokyo-tech");
  // The shell reads only the records it renders itself; each page asks
  // Core for its own (`platform/records`).
  const storeReady = useWorkspace((state) => state.status === "ready");
  const storeError = useWorkspace((state) => state.error);
  const data = useRecords(SHELL_RECORDS, storeReady);
  const sourceData = data;
  useDesktopUpdates(
    Boolean(sourceData),
    sourceData?.settings.automaticAppUpdates,
  );
  const overview = useScheduleQuery<WorkspaceOverview>(
    { kind: "workspaceOverview", term: "" },
    Boolean(sourceData),
  );
  const [lectureRailHovered, setLectureRailHovered] = useState(false);
  // Today's "All assignments" asks Lecture Desk for the quarter's list.
  const [assignmentsRequest, setAssignmentsRequest] = useState<number>();
  const navigate = useAnimatedNavigate();
  const openAccounts = () => void navigate(paths.settings("accounts"));
  const widgetNavigate = useRef(navigate);
  useEffect(() => {
    widgetNavigate.current = navigate;
  }, [navigate]);
  useEffect(() => {
    if (!storeReady) return;
    return onWidgetRoute((route) => {
      switch (route.kind) {
        case "course":
          void widgetNavigate.current(paths.lectures(route.courseId));
          break;
        case "exam":
          void widgetNavigate.current(paths.exam(route.key, route.term));
          break;
        case "assignment":
          if (route.courseId) {
            void widgetNavigate.current(
              paths.lectures(route.courseId, route.id),
            );
            break;
          }
          setAssignmentsRequest(Date.now());
          void widgetNavigate.current(paths.lectures());
          break;
        case "assignments":
        case "unavailableAssignment":
          setAssignmentsRequest(Date.now());
          void widgetNavigate.current(paths.lectures());
          break;
        case "sync":
          void widgetNavigate.current(paths.settings("accounts"));
          break;
        case "unavailableCourse":
        case "timetable":
          void widgetNavigate.current(paths.timetable());
          break;
      }
    });
  }, [storeReady]);
  const location = useLocation();
  const params = useParams();
  const timetableMatch = useMatch(routePatterns.timetable);
  const lecturesMatch = useMatch(`${routePatterns.lectures}/*`);
  const webMatch = useMatch(`${routePatterns.web}/*`);
  const toolsMatch = useMatch(routePatterns.tools);
  const pastExamsMatch = useMatch(`${routePatterns.pastExams}/*`);
  const settingsMatch = useMatch(`${routePatterns.settings}/*`);
  const aboutMatch = useMatch(routePatterns.about);
  const page: Page = timetableMatch
    ? "timetable"
    : lecturesMatch
      ? "lectures"
      : webMatch
        ? "web"
        : toolsMatch
          ? "tools"
          : pastExamsMatch
            ? "pastExams"
            : settingsMatch
              ? "settings"
              : aboutMatch
                ? "about"
                : "home";
  const planningTerm = useCoursePlanningTerm(
    overview.value,
    page === "timetable",
  );
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
  // Campus VPN reminders come whichever page is open; the system
  // notification is worded in the UI language.
  useEffect(
    () =>
      desktopBridge?.onVpnNotice?.((notice) => {
        const [title, body] = vpnNoticeText(notice);
        void desktopBridge?.notify(title, body).catch(() => undefined);
        // Retrying cannot fix it; the next launch must not try again.
        if (notice.kind === "demandStopped") void saveVpnOnDemand(false);
      }),
    [],
  );
  const section = (params.section ?? "accounts") as Section;
  const [mountedPages, setMountedPages] = useState<Page[]>([page]);
  if (!mountedPages.includes(page)) setMountedPages([...mountedPages, page]);
  const pageRef = useRef<Page>(page);
  pageRef.current = page;
  // Switching pages returns to where each page was left (open lecture,
  // settings section); the course drawer query is not carried.
  const pageLocations = useRef<Partial<Record<Page, string>>>({});
  pageLocations.current[page] = location.pathname;
  // A course zoom stays paired while the course is shown, so back reverses
  // it; once a page other than the course has played its transition, it ends.
  useEffect(() => {
    if (page === "lectures") return;
    const timer = setTimeout(clearZoom, 400);
    return () => clearTimeout(timer);
  }, [page]);
  const pagePath = (next: Page) =>
    pageLocations.current[next] ?? paths.page(next);
  const [webContext, setWebContext] = useState<{
    page: "home" | "lectures";
    label: string;
    url: string;
    request: number;
  } | null>(null);
  // Hidden Activities keep their last route props so another page's params
  // cannot replace retained subpages. Visible navigation always reads the URL.
  const renderedRoutes = useRef({
    section,
    lectureEntry: null as CourseEntry | null,
    examEntry: null as { key: string; term: TermId; request: number } | null,
    pastExamCourse: null as { courseId: string; key: string } | null,
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
  if (page === "settings") renderedRoutes.current.section = section;
  if (page === "lectures") {
    renderedRoutes.current.lectureEntry = lectureEntry;
    renderedRoutes.current.examEntry = examEntry;
  }
  if (page === "pastExams") {
    const courseId = params.examCourseId;
    const retained = renderedRoutes.current.pastExamCourse;
    renderedRoutes.current.pastExamCourse = courseId
      ? {
          courseId,
          key:
            retained?.courseId === courseId &&
            !location.state?.pastExamsShortcut
              ? retained.key
              : location.key,
        }
      : null;
  }
  const retainedLecture = renderedRoutes.current.lectureEntry;
  const pastExamCourse = renderedRoutes.current.pastExamCourse;
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
  const sync = useSync();
  useCoreLifecycle();
  const [groupPrompt, setGroupPrompt] = useState<ClassGroupChoice | null>(null);
  /** After a pull settles the timetable, ask once for the physics group of
   * a current or later semester that has none. */
  const askClassGroup = async () => {
    // The question is a convenience; a failed read must not fail the pull.
    const choices = await scheduleQuery<ClassGroupChoice[]>({
      kind: "classGroups",
      today: tokyoDate(new Date(wallNow())),
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
    if (storeError && sourceData)
      notify.error(storeError, () => void workspaceStore.reload());
  }, [storeError, Boolean(sourceData)]);
  useEffect(() => {
    if (pageRef.current !== "web" && !params.courseId)
      void hideNativeSchoolWeb().catch(notify.error);
  }, []);
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (
        (e.metaKey || e.ctrlKey) &&
        ["1", "2", "3", "4", "5", "6"].includes(e.key)
      ) {
        e.preventDefault();
        const next: Page =
          e.key === "6" ? "settings" : pages[Number(e.key) - 1];
        void navigate(pagePath(next));
      }
    };
    addEventListener("keydown", key);
    return () => removeEventListener("keydown", key);
  }, [navigate]);
  // Each visible view tells Core once; Core's open threshold decides what
  // is read. A page shown again sends its view again.
  useOpenHint(page === "home" ? { kind: "today" } : null);
  useOpenHint(lectureOpenView(page === "lectures", lectureEntry, sourceData));
  useOpenHint(page === "timetable" ? { kind: "timetable" } : null);
  useOpenHint(page === "timetable" ? { kind: "grades" } : null);
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
  const workspaceView = planningTerm.workspaceView;
  if (!data || !overview.value || !workspaceView)
    return (
      <div className="loading">
        {storeError ?? overview.error ?? t("app.openingYourWorkspace")}
        {(storeError || overview.error) && (
          <UiButton
            onClick={() =>
              storeError ? void workspaceStore.initialize() : overview.retry?.()
            }
          >
            {t("common.retry")}
          </UiButton>
        )}
      </div>
    );

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
      background: { scheduler: schedulerState },
      noBackendPush: true,
    },
  );
  const navigation = {
    home: t("common.today"),
    timetable: t("common.timetable"),
    lectures: t("common.lectures"),
    web: t("common.web"),
    tools: t("common.tools"),
    pastExams: t("examShare.title"),
    settings: t("common.settings"),
    about: t("settings.main.about"),
  };
  const todayTerm = workspaceView.todayTerm;
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
    // Within Lecture Desk only the course pane enters (LectureDesk); the
    // list beside it stays still, and no zoom pairs with the new course.
    const within = page === "lectures";
    if (within) clearZoom();
    void navigate(paths.lectures(courseId, assignmentId), {
      state: { lectureRequest: Date.now(), lectureContext: timing },
      ...(within && { viewTransition: false }),
    });
  };
  const openPastExams = desktopBridge?.examShare
    ? (courseId: string) =>
        void navigate(paths.pastExams(courseId), {
          state: { pastExamsShortcut: true },
        })
    : undefined;
  const openExam = (key: string, term: string) => {
    void navigate(paths.exam(key, term), {
      state: { lectureRequest: Date.now() },
      ...(page === "lectures" && { viewTransition: false }),
    });
  };
  // Refresh buttons ask Core for a scope; progress and failures arrive in
  // the sync snapshot.
  const canSync = syncAvailable() && !readOnly;
  const navigatePage = (next: Page) => {
    if (next === "web") setWebContext(null);
    void navigate(pagePath(next));
  };

  const keep = (name: Page, node: React.ReactNode) =>
    mountedPages.includes(name) && (
      <KeptPage visible={page === name} key={name}>
        <PageErrorBoundary name={name}>{node}</PageErrorBoundary>
      </KeptPage>
    );
  const timetablePage = (
    <CoursePlanningPage
      quarter={planningTerm}
      active={page === "timetable"}
      locationKey={location.key}
      overview={overview.value}
      data={data}
      workspaceView={workspaceView}
      sync={sync}
      readOnly={readOnly}
      canSync={canSync}
      openWeb={openWeb}
      openPastExams={openPastExams}
      openLecture={openLecture}
      academicIntentOrThrow={academicIntentOrThrow}
    />
  );
  const lecturesPage = (
    <LectureWorkspacePage
      data={data}
      placements={workspaceView.placements}
      lectureRailHovered={lectureRailHovered}
      todayTerm={todayTerm}
      examEntry={renderedRoutes.current.examEntry}
      openExam={openExam}
      retainedLecture={retainedLecture}
      active={page === "lectures"}
      assignmentsRequest={assignmentsRequest}
      readOnly={readOnly}
      academicIntent={academicIntent}
      academicIntentOrThrow={academicIntentOrThrow}
      openLecture={openLecture}
      openAccounts={openAccounts}
      openPastExams={openPastExams}
      openWeb={(url) => openWeb(url, "lectures")}
      schoolWebAvailable={nativeCapabilities?.schoolWeb === true}
      canSync={canSync}
      sync={sync}
    />
  );
  const showSchoolSetup =
    !readOnly &&
    !data.importStatus.timetableReviewedAt &&
    !data.courses.some((course) => !course.deletedAt) &&
    !connectedSession(tokyoTechSession);
  const todayToggle = (
    <WorkspaceSidebarToggle
      panel={todayPanel}
      collapsed={todayFolded}
      controls="home-today"
    />
  );
  const homePage = (
    <div
      className={`homePage${showSchoolSetup ? " needsSetup" : ""}`}
      data-sidebar-collapsed={todayFolded}
    >
      {showSchoolSetup && (
        <aside className="homeSetup" aria-label={t("app.schoolAccountSetup")}>
          <div>
            <b>{t("app.connectYourSchoolAccounts")}</b>
            <p>{t("app.connectTokyoTechForYour")}</p>
          </div>
          <UiButton
            className="button primary"
            onClick={() => void navigate(paths.settings("accounts"))}
          >
            {t("common.connectSchoolAccounts")}
          </UiButton>
        </aside>
      )}
      <ResizableWorkspace
        id="home"
        enabled
        firstSize={360}
        firstCollapsible
        firstPanelRef={todayPanel}
        onFirstCollapse={setTodayFolded}
        firstMin={240}
        firstMax="50%"
        secondMin="45%"
      >
        <div className="homeDay" id="home-today" inert={todayFolded}>
          <Today
            data={data}
            sidebar={todayFolded ? undefined : todayToggle}
            active={page === "home" && !todayFolded}
            term={todayTerm}
            openExam={openExam}
            open={(courseId, _tab, context) => openLecture(courseId, context)}
            openEvent={(event) =>
              !readOnly &&
              courseEditors.editEvent({ event, term: event.termId })
            }
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
          />
        </div>
        <div className="homeWeek">
          <CalendarView
            data={data}
            titleStart={
              todayFolded ? <TitlebarNav sidebar={todayToggle} /> : undefined
            }
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
              else {
                const event = data.events.find((row) => row.id === block.id);
                if (event && !readOnly)
                  courseEditors.editEvent({ event, term: event.termId });
              }
            }}
          />
        </div>
      </ResizableWorkspace>
    </div>
  );
  const webPage = (
    <Web
      openAccounts={openAccounts}
      key={webContext?.url ?? "web"}
      active={page === "web"}
      platform={platform}
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
  const aboutPage = <About data={data} update={update} />;
  const toolsPage = <Tools platform={platform} notify={notify.info} />;
  const settingsPage = (
    <Settings
      mutate={workspaceActions.mutate}
      platform={platform}
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
      showLoginPage={(profile, options) => {
        const site = profile === "science" ? "science-portal" : "tokyo-portal";
        sessionStorage.setItem("isct-companion-active-school-site", site);
        // An explicit entry is consumed on the next opening of this site.
        if (options) {
          const entry =
            "otpUsername" in options
              ? { kind: "otpSetup", username: options.otpUsername }
              : { kind: "unsigned" };
          sessionStorage.setItem(
            "isct-companion-school-entry",
            JSON.stringify(entry),
          );
        } else {
          sessionStorage.removeItem("isct-companion-school-entry");
        }
        void navigate(paths.web());
      }}
    />
  );
  const overlays = (
    <>
      <WelcomeSheet
        settings={data.settings}
        update={update}
        connected={
          connectedSession(scienceSession) || connectedSession(tokyoTechSession)
        }
        held={!readOnly && !data.settings.dataNoticeAcknowledgedAt}
        openAccounts={() => void navigate(paths.settings("accounts"))}
        openAgents={() => void navigate(paths.settings("agent"))}
      />
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
      <CourseEditors
        data={data}
        workspaceView={workspaceView}
        academicIntentOrThrow={academicIntentOrThrow}
      />
    </>
  );
  const railPages = pages.filter(
    (p) =>
      (p !== "tools" || availableTools(platform).length > 0) &&
      (p !== "pastExams" || desktopBridge !== undefined),
  );
  return (
    <div className="app" data-density={data.settings.density}>
      <aside className="rail">
        <nav aria-label={t("app.pages")}>
          {railPages.map((p) => (
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
              title={`${navigation[p]} (⌘${pages.indexOf(p) + 1})`}
            >
              {icons[p]}
              <span>{navigation[p]}</span>
            </UiButton>
          ))}
        </nav>
        <div className="railUtilities">
          {(["settings", "about"] as const).map((p) => (
            <UiButton
              key={p}
              className={`railUtility${page === p ? " active" : ""}`}
              onClick={() => void navigate(pagePath(p))}
              title={navigation[p]}
            >
              {icons[p]}
              <span>{navigation[p]}</span>
            </UiButton>
          ))}
          <SchoolStatusDot
            scienceSession={scienceSession}
            tokyoTechSession={tokyoTechSession}
            snapshot={sync}
            locale={data.settings.locale}
            refresh={
              canSync ? () => refresh({ kind: "everything" }) : undefined
            }
          />
        </div>
      </aside>
      <main
        className={`workspaceMain${page === "home" ? " homeMain" : page === "timetable" ? " timetableMain" : page === "lectures" ? " lecturesMain" : page === "web" ? " webMain" : page === "settings" ? " settingsMain" : page === "about" ? " aboutMain" : ""}`}
      >
        {keep("lectures", lecturesPage)}
        {keep("home", homePage)}
        {keep("timetable", timetablePage)}
        {keep("web", webPage)}
        {keep("tools", toolsPage)}
        {keep(
          "pastExams",
          <PastExams
            key={pastExamCourse?.key}
            courseId={pastExamCourse?.courseId}
            active={page === "pastExams"}
          />,
        )}
        {keep("settings", settingsPage)}
        {keep("about", aboutPage)}
      </main>
      {overlays}
    </div>
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
    <>
      <Notifications />
      <ConfirmDialogs />
      <RouterProvider router={router} />
    </>
  );
}
