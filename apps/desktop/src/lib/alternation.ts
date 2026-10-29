import { t, type MessageKey } from "../i18n";
import type { Alternation, ClassGroup, Turn } from "./model";

const GROUP_LABELS: Record<ClassGroup, MessageKey> = {
  red: "alternation.redGroup",
  white: "alternation.whiteGroup",
};

/** "Red group" / "White group", localized. */
export function classGroupLabel(group: ClassGroup): string {
  return t(GROUP_LABELS[group]);
}

function turnLabel(turn: Turn, session?: Alternation["session"]): string {
  if (turn !== "all") return classGroupLabel(turn);
  if (session === "orientation") return t("alternation.orientationBothGroups");
  if (session === "exam") return t("alternation.examBothGroups");
  return t("alternation.bothGroups");
}

/** The short tag a 物理学演習・実験 block carries on the timetable. */
export function alternationLabel(alternation: Alternation): string {
  if (!alternation.scheduled)
    return t("alternation.alternateWeeksScheduleNotListed");
  if (alternation.session === "no-class") return t("common.noClass");
  if (!alternation.turn) {
    return alternation.group
      ? `${t("alternation.alternateWeeks")} · ${classGroupLabel(alternation.group)}`
      : t("alternation.alternateWeeks");
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
