import type {
  AssignmentState,
  TaskDeadlineView,
} from "../generated/persistence";
import type { Assignment } from "./model";
import type { MessageKey } from "../i18n";
export { JST } from "./civil-date";

/** Render the state Core selected and the grade displayed by the LMS. */
export function submissionLabel(
  assignment: Assignment,
  translate: (key: MessageKey) => string,
  state: AssignmentState | undefined,
): string {
  if (state === "graded")
    return assignment.grade
      ? `${translate("status.graded")} ${assignment.grade}`
      : translate("status.graded");
  if (state === "submitted") return translate("status.submitted");
  return translate(
    assignment.submission === "open" ? "status.open" : "status.unknown",
  );
}

/** Visual status of a list row and its deadline chip. */
export function assignmentRowStatus(
  state: AssignmentState | undefined,
  deadline: TaskDeadlineView | undefined,
) {
  const submitted = state === "submitted" || state === "graded";
  const tone =
    deadline?.state === "overdue"
      ? submitted
        ? " closed"
        : " missed"
      : submitted
        ? " done"
        : "";
  return { submitted, tone };
}
