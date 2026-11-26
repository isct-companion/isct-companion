import { refresh } from "../../platform/sync-intents";
import { pageProgress, runsFor } from "../../platform/sync";
import {
  scheduleQuery,
  type WorkspaceOverview,
} from "../../platform/core-query";
import { useFinder } from "./useFinder";
import type { FinderRowView } from "../../platform/finder-query";
import { notify } from "../../components/notifications/notify";
import { confirmAction } from "../../components/confirm";
import { t } from "../../i18n";
import { desktopBridge } from "../../platform/native";
import { saveExportFile } from "../../platform/save-export";
import { wallNow } from "../../platform/use-clock";
import type { CourseEntry } from "../../features/lms";
import { Courses } from "./CoursesPage";
import { useState } from "react";
import type { workspaceActions } from "../../store/workspace";
import { CourseFinder } from "./CourseFinder";
import { AcademicSummary } from "./AcademicSummary";
import { CoursePanel, OfferingPanel } from "./CoursePanel";
import { DAYS as FINDER_DAYS, type FinderCourse } from "../../lib/course-query";
import type { Course, TermId } from "../../lib/model";
import { courseEditors } from "../../store/course-editors";
import { useOpenHint } from "../../app/useOpenHint";
import type { useCoursePlanningTerm } from "./useCoursePlanningTerm";
import type { ShellRecords } from "../../platform/records";
import type { SyncSnapshot, SyncView } from "../../generated/persistence";

const KYOMU_REGISTRATION =
  "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0.aspx";
const KYOMU_GPA =
  "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_7.aspx";

/** The Courses route: quarter planning, the one course finder, and the
 * course and offering panels. Its editors open through `courseEditors`. */
export function CoursePlanningPage({
  quarter,
  active,
  locationKey,
  overview,
  data,
  workspaceView,
  sync,
  readOnly,
  canSync,
  openWeb,
  openPastExams,
  openLecture,
  academicIntentOrThrow,
}: {
  quarter: ReturnType<typeof useCoursePlanningTerm>;
  active: boolean;
  locationKey: string;
  overview: WorkspaceOverview | undefined;
  data: ShellRecords;
  workspaceView: WorkspaceOverview;
  sync: SyncSnapshot;
  readOnly: boolean;
  canSync: boolean;
  openWeb: (url: string) => void;
  openPastExams?: (courseId: string) => void;
  openLecture: (
    courseId: string,
    context?: Omit<CourseEntry, "courseId" | "request">,
  ) => void;
  academicIntentOrThrow: typeof workspaceActions.mutate;
}) {
  const { courseViewTerm, cachedTimetable, setPreviewTerm } = quarter;
  // The quarter the finder's conditions were made for; switching quarters resets them.
  const [finderTerm, setFinderTerm] = useState<TermId | null>(null);
  const finderCourseTerm = courseViewTerm ?? "2026-1Q";
  const finderYear = Number(finderCourseTerm.slice(0, 4));
  const finderSession = JSON.stringify([locationKey, finderCourseTerm]);
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
    enabled: Boolean(overview && active && finderActive),
    defaultKeys:
      (cachedTimetable?.overview ?? overview)?.defaultFinderKeys ?? [],
  });
  // A course opened from the Courses timetable, shown over the syllabus panel.
  const [panelCourseId, setPanelCourseId] = useState<string | null>(null);
  // A search result opened from the syllabus panel, shown over the timetable.
  const [offeringPanel, setOfferingPanel] = useState<FinderRowView | null>(
    null,
  );
  // Shown as the finder has it now (a language change re-reads it); the row
  // as clicked only once it has left the results.
  const shownOffering =
    offeringPanel &&
    (finder.rows.find((row) => row.course.id === offeringPanel.course.id) ??
      offeringPanel);

  const finderView: SyncView | null =
    active && finderActive ? { kind: "finder", year: finderYear } : null;
  useOpenHint(finderView);

  const schoolRecords = data;
  // Pages only render Core's runs; the planner decides what is read.
  const kyomuProgress = pageProgress(runsFor(sync, ["kyomu"]));
  // 教務 pages share one run, so a registrations read shows with the others.
  const timetableProgress = pageProgress(
    runsFor(sync, ["lms.enrolment", "kyomu.page"]),
  );
  const currentTerm = workspaceView.currentTerm;
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
  const panelPlacement = panelCourse
    ? workspaceView.placements[panelCourse.id]
    : undefined;
  const termCredits = workspaceView.termCredits;
  const addFinderCourses = async (rows: FinderCourse[]) => {
    await academicIntentOrThrow({
      kind: "addCatalogCourses",
      termId: courseTerm,
      offerings: rows,
    });
  };
  const removePlan = async (course: Course) => {
    try {
      await academicIntentOrThrow({
        kind: "removeCatalogPlans",
        courseIds: [course.id],
        termId: courseTerm,
      });
      setPanelCourseId(null);
    } catch (error) {
      notify.error(error);
    }
  };
  const showOnTimetable = async (course: Course, on: boolean) => {
    if (
      !on &&
      workspaceView.placements[course.id]?.timetable?.kind === "lms" &&
      !(await confirmAction(
        t("courses.hideLmsTimetableTitle", { title: course.title }),
        t("courses.hideLmsTimetable"),
      ))
    )
      return;
    try {
      await academicIntentOrThrow({
        kind: "setCourseOnTimetable",
        courseId: course.id,
        onTimetable: on,
      });
      setPanelCourseId(null);
    } catch (error) {
      notify.error(error);
    }
  };

  const copyEvents = (from: TermId, term: TermId) =>
    academicIntentOrThrow({
      kind: "copyEvents",
      fromTermId: from,
      termId: term,
    });
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
              ? `${quarter} ${FINDER_DAYS[day - 1]}${Math.min(...meeting.periods)}-${Math.max(...meeting.periods)}`
              : `${quarter} ${t("timetable.intensiveUnscheduled")}`,
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
  const timetablePage = (
    <div className="quarterPage">
      {" "}
      <Courses
        side={
          <CourseFinder
            key={finderSession}
            enabled={finderActive}
            activate={activateFinder}
            year={Number(courseTerm.slice(0, 4))}
            term={courseTerm}
            summary={`${finderTermCourses.length} ${t("course.status.onTimetable")} · ${termCredits} ${t("common.creditsUnit")}`}
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
              readOnly
                ? undefined
                : () => courseEditors.editCourse({ term: courseTerm })
            }
          />
        }
        leftPanel={
          shownOffering && (
            <OfferingPanel
              key={shownOffering.course.id}
              course={shownOffering.course}
              placed={finder.placedOfferingIds.has(shownOffering.course.id)}
              replaces={shownOffering.replaces.map(
                (id) =>
                  data.courses.find((course) => course.id === id)?.title ?? "",
              )}
              readOnly={readOnly}
              close={() => setOfferingPanel(null)}
              add={() => {
                void addFinderCourses([shownOffering.course]).catch(
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
              readOnly={readOnly}
              close={() => setPanelCourseId(null)}
              timetable={panelPlacement?.timetable}
              removePlan={
                workspaceView.term === courseTerm &&
                panelPlacement?.canRemovePlan
                  ? () => void removePlan(panelCourse)
                  : undefined
              }
              showOnTimetable={(on) => void showOnTimetable(panelCourse, on)}
              edit={() =>
                courseEditors.editCourse({
                  course: panelCourse,
                  term: courseTerm,
                })
              }
              editIntensive={() => courseEditors.editIntensive(panelCourse.id)}
              openWeb={(url) => openWeb(url)}
              openPastExams={
                openPastExams && (() => openPastExams(panelCourse.id))
              }
              openLecture={() =>
                openLecture(panelCourse.id, { term: panelCourse.termId })
              }
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
          !readOnly && courseEditors.editEvent({ event, term })
        }
        copyEvents={
          readOnly
            ? undefined
            : (from, term) =>
                copyEvents(from, term).then(
                  () => notify.info(t("app.weeklyEventsCopied")),
                  notify.error,
                )
        }
        onSlot={toggleFinderSlot}
        activeSlots={finder.slots}
        onIntensive={(append) => pickFinderTime("集中", append)}
        intensiveActive={finder.keys.includes("集中")}
        addTerm={() => courseEditors.addTerm(true)}
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
            `IsctCompanion-${term}${scope === "courses-and-events" ? "-with-events" : ""}.ics`,
            () =>
              scheduleQuery<string>({
                kind: "quarterIcs",
                term,
                scope,
                generatedAt: new Date(wallNow()).toISOString(),
              }),
            notify.info,
          )
        }
      />
    </div>
  );
  return timetablePage;
}
