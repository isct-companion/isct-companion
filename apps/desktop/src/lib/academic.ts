import type { Assignment } from "./model";
export { JST } from "./civil-date";

/** Render Core's submitted state and the grade displayed by the LMS. */
export function submissionLabel(
  assignment: Assignment,
  translate: (key: string) => string,
  submitted: boolean,
): string {
  if (assignment.submission === "graded")
    return assignment.grade
      ? `${translate("graded")} ${assignment.grade}`
      : translate("graded");
  return translate(
    submitted ? "submitted" : (assignment.submission ?? "unknown"),
  );
}
