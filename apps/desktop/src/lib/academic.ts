import type { Assignment, Quarter, TermId } from "./model";
import { JST, tokyoDate } from "./civil-date";
export { JST } from "./civil-date";
export function termForDate(date: Date): TermId {
  const [year, month] = tokyoDate(date).split("-").map(Number);
  const academic = month < 4 ? year - 1 : year;
  const q: Quarter =
    month >= 4 && month < 6
      ? "1Q"
      : month >= 6 && month < 10
        ? "2Q"
        : month >= 10 && month < 12
          ? "3Q"
          : "4Q";
  return `${academic}-${q}`;
}

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
