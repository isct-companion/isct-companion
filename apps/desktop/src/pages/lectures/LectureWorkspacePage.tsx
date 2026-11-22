import { formatJst } from "../../lib/civil-date";
import { refresh } from "../../platform/sync-intents";
import { pageProgress, runsFor } from "../../platform/sync";
import { notify } from "../../components/notifications/notify";
import { t } from "../../i18n";
import type { CourseEntry } from "../../features/lms";
import { LectureDesk } from "./LectureDesk";
import { lazy, Suspense } from "react";
import type { workspaceActions } from "../../store/workspace";
import type { Course, TermId } from "../../lib/model";
import { tokyoDate } from "../../lib/civil-date";
import type { WorkspaceIntent } from "../../lib/workspace-intents";
import type { ShellRecords } from "../../platform/records";
import type {
  CoursePlacementActions,
  SyncSnapshot,
  SyncView,
} from "../../generated/persistence";

const LmsWorkspace = lazy(() =>
  import("../../features/lms").then((module) => ({
    default: module.LmsWorkspace,
  })),
);

function lmsRecordFor(schoolRecords: ShellRecords | undefined, course: Course) {
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
}

export function lectureOpenView(
  active: boolean,
  lectureEntry: CourseEntry | null,
  sourceData: ShellRecords | undefined,
): SyncView | null {
  const openedCourse =
    active && lectureEntry && sourceData
      ? sourceData.courses.find(
          (item) => item.id === lectureEntry.courseId && !item.deletedAt,
        )
      : undefined;
  const openedRecord = openedCourse && lmsRecordFor(sourceData, openedCourse);
  return !active
    ? null
    : openedRecord
      ? { kind: "course", lmsCourseId: openedRecord.lmsId }
      : lectureEntry
        ? null
        : { kind: "assignments" };
}

export function LectureWorkspacePage({
  data,
  placements,
  lectureRailHovered,
  todayTerm,
  examEntry,
  openExam,
  retainedLecture,
  active,
  assignmentsRequest,
  readOnly,
  academicIntent,
  academicIntentOrThrow,
  openLecture,
  openAccounts,
  openPastExams,
  openWeb,
  schoolWebAvailable,
  canSync,
  sync,
}: {
  data: ShellRecords;
  placements: Readonly<Record<string, CoursePlacementActions>>;
  lectureRailHovered: boolean;
  todayTerm: TermId;
  examEntry: { key: string; term: TermId; request: number } | null;
  openExam: (key: string, term: string) => void;
  retainedLecture: CourseEntry | null;
  active: boolean;
  assignmentsRequest?: number;
  readOnly: boolean;
  academicIntent: (intent: WorkspaceIntent) => void;
  academicIntentOrThrow: typeof workspaceActions.mutate;
  openLecture: (
    courseId: string,
    context?: Omit<CourseEntry, "courseId" | "request">,
  ) => void;
  openAccounts: () => void;
  openPastExams?: (courseId: string) => void;
  openWeb: (url: string) => void;
  schoolWebAvailable: boolean;
  canSync: boolean;
  sync: SyncSnapshot;
}) {
  const schoolRecords = data;
  return (
    <LectureDesk
      railHovered={lectureRailHovered}
      term={todayTerm}
      examEntry={examEntry}
      selectExam={openExam}
      entry={
        retainedLecture &&
        data.courses.some(
          (c) => c.id === retainedLecture.courseId && !c.deletedAt,
        )
          ? retainedLecture
          : null
      }
      active={active}
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
        const record = lmsRecordFor(schoolRecords, course);
        const timetable = placements[course.id]?.timetable;
        return (
          <Suspense fallback={<p role="status">{t("common.loading")}</p>}>
            <LmsWorkspace
              openAccounts={openAccounts}
              course={course}
              entry={entry}
              active={active}
              headerStart={sidebarToggle}
              openPastExams={openPastExams && (() => openPastExams(course.id))}
              restoreTimetable={
                !readOnly && timetable?.kind === "lms" && timetable.canRestore
                  ? () =>
                      academicIntent({
                        kind: "setCourseOnTimetable",
                        courseId: course.id,
                        onTimetable: true,
                      })
                  : undefined
              }
              openWeb={(url) => openWeb(url)}
              schoolWebAvailable={schoolWebAvailable}
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
                            ? t("app.todaysTimeClassIsRecorded", {
                                time: format({}),
                                lesson: label,
                              })
                            : t("app.yourNextClassDateIs", {
                                date: format({
                                  month: "numeric",
                                  day: "numeric",
                                  weekday: "short",
                                }),
                                lesson: label,
                              }),
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
}
