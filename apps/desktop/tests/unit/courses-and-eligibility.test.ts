import { describe, expect, it } from "vitest";

import { english } from "@app/i18n";
import { submissionLabel } from "@app/lib/academic";
import type { Assignment } from "@app/lib/model";
import { readRepoFile } from "@tests/harness";

/**
 * The parts of the course pipeline that decide what the finder may offer and
 * which department the study profile refers to.
 *
 * These modules had no suite of their own: the department table and the
 * assignment state label were only exercised through the panels that render
 * them. The expectations below come from the modules' contracts and from the
 * bundled 2026 subscription fixture.
 *
 * `unit/` never mounts a component; everything here is plain node.
 */

/** The harness reads files but does not parse them, so this suite owns the cast. */
function repositoryJson<T>(path: string): T {
  return JSON.parse(readRepoFile(path)) as T;
}

/** The harness has no assignment builder; this suite owns one. */
function assignment(overrides: Partial<Assignment> = {}): Assignment {
  return {
    kind: "assignment",
    courseId: "2026-202602440",
    title: "Report 1",
    details: "",
    dueAt: null,
    source: { provider: "lms" },
    id: "assignment:1",
    rev: 1,
    updatedAt: "2026-10-01T00:00:00.000Z",
    updatedBy: "device:test",
    ...overrides,
  };
}

/** The real English catalogue: a key it lacks is a build error, not a label. */
const label = english;

const DEPARTMENTS_PATH =
  "crates/app-core/tests/fixtures/subscriptions/data/2026/departments.json";

type BundledDepartment = {
  id: string;
  name: { ja?: string; en?: string };
  syllabusScope: string;
  unitCode: string;
  guidePdf: string;
  recommended: unknown[];
};

const bundled2026 = repositoryJson<{ departments: BundledDepartment[] }>(
  DEPARTMENTS_PATH,
);

/** The syllabus departments Core lists in the finder and study profile. */
const SYLLABUS_DEPARTMENTS = repositoryJson<
  {
    id: string;
    name: { ja?: string; en?: string };
    unitCode: string;
    scope: string;
  }[]
>("crates/app-core/src/course_query/departments.json");

describe("the bundled undergraduate departments", () => {
  it("matches the bundled 2026 departments and their official fields", () => {
    const shipped = SYLLABUS_DEPARTMENTS.map((department) => department.id);
    const bundled = bundled2026.departments.map((department) => department.id);
    expect(shipped.length).toBeGreaterThan(0);
    expect(new Set(shipped).size).toBe(shipped.length);
    expect([...shipped].sort()).toEqual([...bundled].sort());
    const scopes = SYLLABUS_DEPARTMENTS.map((department) => department.scope);
    const unitCodes = SYLLABUS_DEPARTMENTS.map(
      (department) => department.unitCode,
    );
    expect(new Set(scopes).size).toBe(scopes.length);
    expect(new Set(unitCodes).size).toBe(unitCodes.length);
    for (const department of SYLLABUS_DEPARTMENTS) {
      expect(department.scope, department.id).toMatch(/^\d+\/[^/]+$/);
      expect(department.unitCode, department.id).toMatch(/^\d+$/);
      expect(department.name.ja, department.id).toBeTruthy();
      expect(department.name.en, department.id).toBeTruthy();
    }
    const bundledById = new Map(
      bundled2026.departments.map((department) => [department.id, department]),
    );
    for (const department of SYLLABUS_DEPARTMENTS) {
      const source = bundledById.get(department.id);
      expect(source, department.id).toBeDefined();
      expect(department.name, department.id).toEqual(source!.name);
      expect(department.unitCode, department.id).toBe(source!.unitCode);
      expect(department.scope, department.id).toBe(source!.syllabusScope);
    }
  });
});

describe("the state label shown beside an assignment", () => {
  it("only appends a grade after Core marks the work as graded", () => {
    const graded = assignment({ grade: "秀" });
    const ungraded = submissionLabel(assignment(), label, "graded");
    expect(submissionLabel(graded, label, "graded")).toContain("秀");
    expect(submissionLabel(graded, label, "graded")).toContain(ungraded);
    expect(submissionLabel(graded, label, "submitted")).not.toContain("秀");
  });
});
