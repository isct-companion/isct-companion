import type { Course } from "./model";

/** Muted course colours; blocks tint them lightly, so text stays dark. */
export const COURSE_PALETTE = [
  "#355ba7",
  "#4f8a8b",
  "#668879",
  "#7b8f3a",
  "#b0713f",
  "#a34e5d",
  "#806a9d",
  "#5c7d93",
] as const;

/** Colours the importers give every course; they mean "not chosen". */
const IMPORT_DEFAULTS = new Set(["#526e91", "#556b8d"]);

const hash = (text: string) => {
  let value = 2166136261;
  for (const character of text) value = Math.imul(value ^ character.charCodeAt(0), 16777619);
  return value >>> 0;
};

/**
 * The colour a course is drawn with: the person's choice, else a stable
 * palette colour from its code (the same course keeps its colour across
 * quarters and devices).
 */
export function displayColor(course: Pick<Course, "id" | "code" | "color" | "overrides">): string {
  if (course.overrides?.color) return course.overrides.color;
  if (course.color && !IMPORT_DEFAULTS.has(course.color.toLowerCase())) return course.color;
  const key = course.code.replace(/[-[].*$/, "") || course.id;
  return COURSE_PALETTE[hash(key) % COURSE_PALETTE.length];
}
