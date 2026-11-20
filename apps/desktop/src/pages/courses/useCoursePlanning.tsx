import { refresh } from "../../platform/sync-intents";
import { pageProgress, runsFor } from "../../platform/sync";
import {
  scheduleQuery,
  useTimetableViews,
  type WorkspaceOverview,
} from "../../platform/core-query";
import { useFinder } from "./useFinder";
import type { FinderRowView } from "../../platform/finder-query";
import { notify } from "../../components/notifications/notify";
import { confirmAction } from "../../components/confirm";
import { t } from "../../i18n";
import { desktopBridge } from "../../platform/native";
import { saveExportFile } from "../../platform/save-export";
import type { CourseEntry } from "../../features/lms";
import { Courses } from "./CoursesPage";
import { IntensiveEditor } from "./IntensiveEditor";
import { useEffect, useState } from "react";
import type { workspaceActions } from "../../store/workspace";
import { CourseFinder } from "./CourseFinder";
import { AcademicSummary } from "./AcademicSummary";
import { resolvedTeachingPeriods } from "../../lib/timetable-layout";
import { CoursePanel, OfferingPanel } from "./CoursePanel";
import { DAYS as FINDER_DAYS, type FinderCourse } from "../../lib/course-query";
import {
  CourseEditor,
  EventEditor,
  TermDialog,
  type CourseDraft,
  type EventDraft,
} from "./editors";
import type { Course, PersonalEvent, TermId } from "../../lib/model";
import type { ShellRecords } from "../../platform/records";
import type { SyncSnapshot, SyncView } from "../../platform/sync";

const KYOMU_REGISTRATION =
  "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0.aspx";
const KYOMU_GPA =
  "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_7.aspx";

export function useCoursePlanningTerm(
  overview: WorkspaceOverview | undefined,
  active: boolean,
) {
  const [previewTerm, setPreviewTerm] = useState<TermId | null>(null);
  const courseViewTerm = previewTerm ?? overview?.coursesTerm;
  const timetableViews = useTimetableViews(
    overview?.allTerms ?? [],
    active,
  );
  const cachedTimetable = courseViewTerm
    ? timetableViews.get(courseViewTerm)
    : undefined;
  useEffect(() => {
    if (previewTerm === overview?.coursesTerm) setPreviewTerm(null);
  }, [previewTerm, overview?.coursesTerm]);

  const workspaceView = active
    ? (cachedTimetable?.overview ?? overview)
    : overview;
  return { courseViewTerm, cachedTimetable, workspaceView, setPreviewTerm };
}

export function useCoursePlanning({
  quarter,
  active,
  locationKey,
  overview,
}: {
  quarter: ReturnType<typeof useCoursePlanningTerm>;
  active: boolean;
  locationKey: string;
  overview: WorkspaceOverview | undefined;
}) {
  const { courseViewTerm, cachedTimetable, setPreviewTerm } = quarter;
  const [courseEditor, setCourseEditor] = useState<{
    course?: Course;
    term: TermId;
  } | null>(null);
  const [eventEditor, setEventEditor] = useState<{
    event?: PersonalEvent;
    term: TermId;
  } | null>(null);
  const [intensiveCourseId, setIntensiveCourseId] = useState<string | null>(
    null,
  );
  const [termDialog, setTermDialog] = useState(false);
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

  // The shell renders editor overlays beside its retained pages, so they
  // stay mounted when Today opens an event editor.
  const render = ({
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
  }) => {
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
          t("app.removeThisCourseFromThe"),
          t("common.remove"),
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
                readOnly ? undefined : () => setCourseEditor({ term: courseTerm })
              }
            />
          }
          leftPanel={
            shownOffering && (
              <OfferingPanel
                key={shownOffering.course.id}
                course={shownOffering.course}
                placed={finder.placedCodes.has(shownOffering.course.code)}
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
                remove={() => void removeFromTimetable(panelCourse)}
                edit={() =>
                  setCourseEditor({
                    course: panelCourse,
                    term: panelCourse.termId,
                  })
                }
                editIntensive={() => setIntensiveCourseId(panelCourse.id)}
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
            !readOnly && setEventEditor({ event, term })
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
              `IsctCompanion-${term}${scope === "courses-and-events" ? "-with-events" : ""}.ics`,
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
    const editorOverlays = (
      <>
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
                  notify.info(t("app.intensiveDatesSaved"));
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
              notify.info(
                courseEditor.course ? t("app.courseSaved") : t("app.courseAdded"),
              );
            }}
            onDelete={
              courseEditor.course
                ? async () => {
                    await deleteCourse(courseEditor.course!);
                    setCourseEditor(null);
                    notify.info(t("app.courseDeleted"));
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
              notify.info(t("app.eventSaved"));
            }}
            onDelete={
              eventEditor.event
                ? async () => {
                    await deleteEvent(eventEditor.event!);
                    setEventEditor(null);
                    notify.info(t("app.eventDeleted"));
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
    return { timetablePage, editorOverlays };
  };

  return { finderView, setEventEditor, render };
}
