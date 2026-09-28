import { useMemo, useState } from "react";

import { deadlineState } from "../../core/academic";
import type { Assignment, Workspace } from "../../core/model";

type TaskGroup = "overdue" | "today" | "upcoming" | "completed";

const dayKey = (value: Date) =>
  new Intl.DateTimeFormat("en-CA", { timeZone: "Asia/Tokyo" }).format(value);

function groupFor(assignment: Assignment, today: string): TaskGroup {
  if (assignment.submissionStatus === "submitted" || assignment.completedAt)
    return "completed";
  const dueDay = dayKey(new Date(assignment.dueAt));
  if (dueDay < today) return "overdue";
  if (dueDay === today) return "today";
  return "upcoming";
}

const groupLabels: Record<TaskGroup, string> = {
  overdue: "Overdue",
  today: "Today",
  upcoming: "Upcoming",
  completed: "Completed",
};
const statusLabels: Record<
  NonNullable<Assignment["submissionStatus"]>,
  string
> = {
  unknown: "LMS status unavailable",
  draft: "Draft",
  submitted: "Submitted",
  late: "Late",
  "not-submitted": "Not submitted",
};

export function Tasks({
  data,
  openCourse,
  openOfficial,
}: {
  data: Workspace;
  openCourse: (courseId: string) => void;
  openOfficial: (assignmentId: string, url: string) => void;
}) {
  const [showCompleted, setShowCompleted] = useState(false);
  const today = dayKey(new Date());
  const assignments = useMemo(
    () =>
      data.assignments
        .filter((assignment) => !assignment.deletedAt && !assignment.hidden)
        .sort((left, right) => left.dueAt.localeCompare(right.dueAt)),
    [data.assignments],
  );
  const groups = (["overdue", "today", "upcoming", "completed"] as const)
    .map((id) => ({
      id,
      items: assignments.filter(
        (assignment) => groupFor(assignment, today) === id,
      ),
    }))
    .filter((group) => group.id !== "completed" || showCompleted);
  const openCount = assignments.filter(
    (assignment) =>
      assignment.submissionStatus !== "submitted" && !assignment.completedAt,
  ).length;

  return (
    <div className="page tasksPage">
      <header className="pageBar tasksHeader">
        <div>
          <h1>Tasks</h1>
          <span>
            {openCount ? `${openCount} open` : "Nothing due"}
            {data.importStatus.lastImportedAt
              ? ` · Last checked ${new Intl.DateTimeFormat("en-GB", { dateStyle: "medium", timeStyle: "short" }).format(new Date(data.importStatus.lastImportedAt))}`
              : ""}
          </span>
        </div>
        <UiButton
          type="button"
          className="textButton"
          onClick={() => setShowCompleted((value) => !value)}
        >
          {showCompleted ? "Hide completed" : "Show completed"}
        </UiButton>
      </header>
      <div className="taskTimeline">
        {groups.map((group) => (
          <section className="taskGroup" key={group.id}>
            <header>
              <h2>{groupLabels[group.id]}</h2>
              <span>{group.items.length}</span>
            </header>
            {group.items.map((assignment) => {
              const course = data.courses.find(
                (item) => item.id === assignment.courseId && !item.deletedAt,
              );
              const due = deadlineState(assignment);
              const submissionStatus =
                assignment.submissionStatus ??
                (assignment.completedAt ? "submitted" : "unknown");
              return (
                <article className="taskRow" key={assignment.id}>
                  <span
                    className={`taskState ${submissionStatus}`}
                    title={statusLabels[submissionStatus]}
                  >
                    {submissionStatus === "submitted" ? "✓" : ""}
                  </span>
                  <UiButton
                    type="button"
                    className="taskMain"
                    onClick={() => course && openCourse(course.id)}
                  >
                    <b>{assignment.title}</b>
                    <span>
                      {course?.title ?? "Unlinked task"}
                      {assignment.details ? ` · ${assignment.details}` : ""}
                    </span>
                    <small className={`submissionStatus ${submissionStatus}`}>
                      {statusLabels[submissionStatus]}
                    </small>
                  </UiButton>
                  <time className={due.tone} dateTime={assignment.dueAt}>
                    {due.label}
                  </time>
                  <div className="taskActions">
                    {assignment.sourceUrl && (
                      <UiButton
                        type="button"
                        aria-label={`Open ${assignment.title} in the official site`}
                        onClick={() =>
                          openOfficial(assignment.id, assignment.sourceUrl!)
                        }
                      >
                        Official
                      </UiButton>
                    )}
                  </div>
                </article>
              );
            })}
            {!group.items.length && (
              <div className="taskEmpty">
                {group.id === "today" ? "Nothing due today." : "No tasks."}
              </div>
            )}
          </section>
        ))}
      </div>
    </div>
  );
}
import { UiButton } from "../../components/mantine";
