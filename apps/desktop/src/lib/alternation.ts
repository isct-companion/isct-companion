import { t } from "../i18n";
import type { Alternation, ClassGroup, Turn } from "./model";

const GROUP_LABELS: Record<ClassGroup, string> = {
  red: "Red group",
  white: "White group",
};

/** "Red group" / "White group", localized. */
export function classGroupLabel(group: ClassGroup): string {
  return t(GROUP_LABELS[group]);
}

function turnLabel(turn: Turn, session?: Alternation["session"]): string {
  if (turn !== "all") return classGroupLabel(turn);
  if (session === "orientation") return t("Orientation (both groups)");
  if (session === "exam") return t("Exam (both groups)");
  return t("Both groups");
}

/** The short tag a 物理学演習・実験 block carries on the timetable. */
export function alternationLabel(alternation: Alternation): string {
  if (!alternation.scheduled) return t("Alternate weeks · schedule not listed");
  if (alternation.session === "no-class") return t("No class");
  if (!alternation.turn) {
    return alternation.group
      ? `${t("Alternate weeks")} · ${classGroupLabel(alternation.group)}`
      : t("Alternate weeks");
  }
  return [turnLabel(alternation.turn, alternation.session), alternation.number]
    .filter(Boolean)
    .join(" · ");
}

/** A semester's quarter ids from Core, as "2026 3Q–4Q". */
export function semesterLabel(terms: readonly string[]): string {
  return terms
    .map((term, index) => (index ? term.slice(5) : term.replace("-", " ")))
    .join("–");
}
