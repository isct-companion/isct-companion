import { DueChip } from "../../components/DueChip";
import type { Assignment, TermId, Workspace } from "../../lib/model";
import { assignmentRowStatus, submissionLabel } from "../../lib/academic";
import { AssignmentGroupList } from "../../components/assignments/AssignmentGroupList";
import { t } from "../../i18n";
import type { CourseEntry } from "../../features/lms";
import { type TaskView } from "../../platform/core-query";
import type { LectureSearch } from "../../generated/persistence";

export function LectureAssignments({
  data,
  taskView,
  shownTerm,
  search,
  needle,
  entry,
  choose,
  restoreAssignment,
}: {
  data: Workspace;
  taskView?: TaskView;
  shownTerm: TermId;
  search?: LectureSearch;
  needle: string;
  entry: CourseEntry | null;
  choose: (courseId: string, assignmentId?: string) => void;
  restoreAssignment?: (assignmentId: string) => void;
}) {
  const taskTerm = taskView?.byTerm?.[shownTerm];
  const byAssignment = new Map(data.assignments.map((item) => [item.id, item]));
  const courseById = new Map(data.courses.map((course) => [course.id, course]));
  const matchesWork = (assignment: Assignment) => {
    return Boolean(search?.assignmentIds.includes(assignment.id));
  };
  const workRow = (assignment: Assignment) => {
    const course = courseById.get(assignment.courseId);
    const due = taskView?.deadlines?.[assignment.id];
    const state = taskView?.states?.[assignment.id];
    const { submitted, tone } = assignmentRowStatus(state, due);
    return (
      <button
        type="button"
        key={assignment.id}
        className={`lectureRow${tone}${entry?.assignmentId === assignment.id ? " selected" : ""}`}
        aria-current={
          entry?.assignmentId === assignment.id ? "true" : undefined
        }
        style={{ "--course": course?.color } as React.CSSProperties}
        onClick={() => choose(assignment.courseId, assignment.id)}
      >
        <b>{assignment.title}</b>
        <small>
          {course?.title ?? "LMS"} · {submissionLabel(assignment, t, state)}
        </small>
        {due && (
          <DueChip
            due={due}
            submitted={submitted}
            locale={data.settings.locale}
          />
        )}
      </button>
    );
  };
  const hiddenRow = (assignment: Assignment) => {
    const course = courseById.get(assignment.courseId);
    return (
      <div
        key={assignment.id}
        className="lectureRow hidden"
        style={{ "--course": course?.color } as React.CSSProperties}
      >
        <b>{assignment.title}</b>
        <small>{course?.title ?? "LMS"}</small>
        <button
          type="button"
          className="textButton"
          onClick={() => restoreAssignment?.(assignment.id)}
        >
          {t("common.restore")}
        </button>
      </div>
    );
  };
  return (
    <AssignmentGroupList
      key={shownTerm}
      groups={taskTerm?.record}
      assignments={byAssignment}
      row={workRow}
      hiddenRow={restoreAssignment ? hiddenRow : undefined}
      matches={needle ? matchesWork : undefined}
      pastLabel={t("lectures.pastDeadline")}
      empty={t(
        needle
          ? "lectures.noMatchingAssignments"
          : "lectures.noAssignmentsThisQuarter",
      )}
    />
  );
}
