import { DueChip } from "../../components/DueChip";
import { submissionLabel } from "../../lib/academic";
import { t } from "../../i18n";
import { UiButton, Skeleton } from "../../components/ui";
import { type Assignment, type TermId, type Workspace } from "../../lib/model";
import { AssignmentGroupList } from "../../components/assignments/AssignmentGroupList";
import { type TaskView } from "../../platform/core-query";

const DUE_ROWS = 8;

export function TodayAssignments({
  data,
  term,
  taskView,
  tasksLoading,
  tasksReveal,
  selection,
  openTask,
  hideAssignment,
  openAssignments,
}: {
  data: Workspace;
  term: TermId;
  taskView?: TaskView;
  tasksLoading: boolean;
  tasksReveal?: string;
  selection?: { assignmentId?: string };
  openTask: (assignmentId: string) => void;
  hideAssignment: (assignmentId: string) => void;
  openAssignments?: () => void;
}) {
  const taskTerm = taskView?.byTerm[term];
  const submittedIds = new Set(taskView?.submittedIds);
  const byAssignment = new Map(data.assignments.map((item) => [item.id, item]));
  const pendingCount = taskTerm?.pendingIds.length ?? 0;
  const row = (assignment: Assignment) => {
    const course = data.courses.find(
      (item) => item.id === assignment.courseId && !item.deletedAt,
    );
    const due = taskView?.deadlines?.[assignment.id];
    const submitted = submittedIds.has(assignment.id);
    // Past and submitted: gray; past and never submitted: red; submitted
    // ahead of the deadline: green.
    const state =
      due?.state === "overdue"
        ? submitted
          ? " closed"
          : " missed"
        : submitted
          ? " done"
          : "";
    return (
      <div className={`dueRow${state}`} key={assignment.id}>
        <i style={{ background: course?.color ?? "var(--muted)" }} />
        <UiButton
          className={`dueAssignment${selection?.assignmentId === assignment.id ? " selected" : ""}`}
          aria-current={
            selection?.assignmentId === assignment.id ? "true" : undefined
          }
          onClick={() => openTask(assignment.id)}
        >
          <span>
            <b>{assignment.title}</b>
            <small>
              {course?.title ?? "LMS"} ·{" "}
              {submissionLabel(
                assignment,
                t,
                taskView?.states?.[assignment.id],
              )}
            </small>
          </span>
          {due && (
            <DueChip
              due={due}
              submitted={submitted}
              locale={data.settings.locale}
            />
          )}
        </UiButton>
        <UiButton
          className="textButton"
          onClick={() => hideAssignment(assignment.id)}
        >
          {t("common.hide")}
        </UiButton>
      </div>
    );
  };
  return (
    <section
      className="todaySection dueSection"
      data-today-section="assignments"
    >
      <header>
        <h2>{t("common.assignments")}</h2>
        <span>
          {pendingCount
            ? t("today.countActive", { count: pendingCount })
            : t("today.allClear")}
        </span>
        {openAssignments && (
          <UiButton className="textButton" onClick={openAssignments}>
            {t("common.allAssignments")} →
          </UiButton>
        )}
      </header>
      {tasksLoading ? (
        <Skeleton count={3} />
      ) : (
        <div className={`dueList${tasksReveal ? ` ${tasksReveal}` : ""}`}>
          <AssignmentGroupList
            groups={taskTerm?.today}
            assignments={byAssignment}
            row={row}
            thisWeekLimit={DUE_ROWS}
            pastLabel={t("today.missed")}
            empty={t("today.nothingDueThisWeek")}
          />
        </div>
      )}
    </section>
  );
}
