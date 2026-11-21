import { notify } from "../../components/notifications/notify";
import { t } from "../../i18n";
import { resolvedTeachingPeriods } from "../../lib/timetable-layout";
import type { WorkspaceOverview } from "../../platform/core-query";
import type { ShellRecords } from "../../platform/records";
import { courseEditors, useCourseEditors } from "../../store/course-editors";
import type { workspaceActions } from "../../store/workspace";
import { CourseEditor, EventEditor, TermDialog } from "./editors";
import { IntensiveEditor } from "./IntensiveEditor";

/** The course, event, intensive-dates and new-term editors. The shell mounts
 * this once; pages open an editor through `courseEditors`. */
export function CourseEditors({
  data,
  workspaceView,
  academicIntentOrThrow,
}: {
  data: ShellRecords;
  workspaceView: WorkspaceOverview;
  academicIntentOrThrow: typeof workspaceActions.mutate;
}) {
  const {
    course: courseEditor,
    event: eventEditor,
    intensiveCourseId,
    addingTerm,
  } = useCourseEditors();
  const intensiveCourse = intensiveCourseId
    ? data.courses.find((item) => item.id === intensiveCourseId)
    : undefined;
  const currentTerm = workspaceView.currentTerm;
  return (
    <>
      {intensiveCourse && (
        <IntensiveEditor
          opened
          value={data.intensiveSchedules.find(
            (item) => item.courseId === intensiveCourse.id && !item.deletedAt,
          )}
          close={() => courseEditors.editIntensive(null)}
          save={async (value) => {
            await academicIntentOrThrow({
              kind: "saveIntensiveSchedule",
              courseId: intensiveCourse.id,
              draft: value,
            });
            courseEditors.editIntensive(null);
            notify.info(t("app.intensiveDatesSaved"));
          }}
        />
      )}
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
          onClose={() => courseEditors.editCourse(null)}
          onSave={async (draft) => {
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
              termId: courseEditor.term,
              existingId: courseEditor.course?.id,
            });
            courseEditors.editCourse(null);
            notify.info(
              courseEditor.course ? t("app.courseSaved") : t("app.courseAdded"),
            );
          }}
          onDelete={
            courseEditor.course
              ? async () => {
                  await academicIntentOrThrow({
                    kind: "deleteCourseBatch",
                    courseIds: [courseEditor.course!.id],
                  });
                  courseEditors.editCourse(null);
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
          onClose={() => courseEditors.editEvent(null)}
          onSave={async (draft) => {
            await academicIntentOrThrow({
              kind: "saveEvent",
              draft,
              termId: eventEditor.term,
              existingId: eventEditor.event?.id,
            });
            courseEditors.editEvent(null);
            notify.info(t("app.eventSaved"));
          }}
          onDelete={
            eventEditor.event
              ? async () => {
                  await academicIntentOrThrow({
                    kind: "deleteEvent",
                    eventId: eventEditor.event!.id,
                  });
                  courseEditors.editEvent(null);
                  notify.info(t("app.eventDeleted"));
                }
              : undefined
          }
        />
      )}
      {addingTerm && (
        <TermDialog
          existing={workspaceView.allTerms}
          defaultYear={Number(currentTerm.slice(0, 4))}
          onClose={() => courseEditors.addTerm(false)}
          onAdd={async (term) => {
            try {
              await academicIntentOrThrow({ kind: "addTerm", termId: term });
              courseEditors.addTerm(false);
            } catch (error) {
              notify.error(error);
            }
          }}
        />
      )}
    </>
  );
}
