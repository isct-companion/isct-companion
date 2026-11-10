import type { Assignment } from "./model";
import type { MessageKey } from "../i18n";
export { JST } from "./civil-date";

const SUBMISSION = {
  unknown: "status.unknown",
  open: "status.open",
  submitted: "status.submitted",
  graded: "status.graded",
} satisfies Record<NonNullable<Assignment["submission"]>, MessageKey>;

/** Render Core's submitted state and the grade displayed by the LMS. */
export function submissionLabel(
  assignment: Assignment,
  translate: (key: MessageKey) => string,
  submitted: boolean,
): string {
  if (assignment.submission === "graded")
    return assignment.grade
      ? `${translate("status.graded")} ${assignment.grade}`
      : translate("status.graded");
  return translate(
    submitted
      ? "status.submitted"
      : SUBMISSION[assignment.submission ?? "unknown"],
  );
}
