import departments from "../../crates/isct-core/src/course_query/departments.json";
import type { Localized } from "./model";

/**
 * The undergraduate departments as the official syllabus lists them. This is
 * syllabus structure, not subscription data, so search works without any.
 */
export type SyllabusDepartment = {
  id: string;
  name: Localized;
  unitCode: string;
  scope: string;
};

// One checked-in syllabus structure is consumed by Rust and the shell.
export const SYLLABUS_DEPARTMENTS: readonly SyllabusDepartment[] = departments;

/** Settings may hold a plain id ("cs") or a subscription-scoped one ("2026:cs"). */
export const departmentById = (id: string | undefined) =>
  id
    ? SYLLABUS_DEPARTMENTS.find((d) => d.id === id || id.endsWith(`:${d.id}`))
    : undefined;
