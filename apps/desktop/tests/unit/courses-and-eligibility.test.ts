import { describe, expect, it } from "vitest";

import { english } from "@app/i18n";
import { submissionLabel } from "@app/lib/academic";
import { COURSE_FILE_POLICY } from "@app/lib/course-file-policy";
import { SYLLABUS_DEPARTMENTS, departmentById } from "@app/lib/departments";
import type { Assignment } from "@app/lib/model";
import { readRepoFile } from "@tests/harness";

/**
 * The parts of the course pipeline that decide what the finder may offer and
 * which department the study profile refers to.
 *
 * These modules had no suite of their own: the finder constants, the department
 * table, the generated course-file limits and the assignment state label were
 * only exercised through the panels that render them. The expectations below
 * come from the modules' contracts and from the bundled 2026 subscription
 * fixture.
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
const CS_RECOMMENDED_PATH =
  "crates/app-core/tests/fixtures/subscriptions/data/2026/cs-recommended.json";

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

describe("the bundled undergraduate departments", () => {
  it("lists the same departments as the bundled 2026 subscription data", () => {
    const shipped = SYLLABUS_DEPARTMENTS.map((department) => department.id);
    const bundled = bundled2026.departments.map((department) => department.id);
    expect(shipped.length).toBeGreaterThan(0);
    expect(new Set(shipped).size).toBe(shipped.length);
    expect([...shipped].sort()).toEqual([...bundled].sort());
  });

  it("gives every department an official syllabus scope and unit code", () => {
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
  });

  it("agrees with the bundled 2026 data on every name, unit code and scope", () => {
    const bundled = new Map(
      bundled2026.departments.map((department) => [department.id, department]),
    );
    for (const department of SYLLABUS_DEPARTMENTS) {
      const source = bundled.get(department.id);
      expect(source, department.id).toBeDefined();
      expect(department.name, department.id).toEqual(source!.name);
      expect(department.unitCode, department.id).toBe(source!.unitCode);
      expect(department.scope, department.id).toBe(source!.syllabusScope);
    }
  });
});

describe("resolving the department stored in settings", () => {
  const cs = SYLLABUS_DEPARTMENTS.find((department) => department.id === "cs")!;

  it("resolves a subscription-scoped id by the part after the colon", () => {
    expect(departmentById("2026:cs")).toBe(cs);
    expect(departmentById("undergraduate:cs")).toBe(cs);
  });

  it("resolves every id the settings picker offers", () => {
    for (const department of SYLLABUS_DEPARTMENTS)
      expect(departmentById(department.id), department.id).toBe(department);
  });

  it("returns nothing when no department is selected", () => {
    expect(departmentById(undefined)).toBeUndefined();
    expect(departmentById("")).toBeUndefined();
  });

  it("returns nothing for an id that names no department", () => {
    for (const value of ["astrophysics", "2026:csx", "xcs", "2026:"])
      expect(departmentById(value), value).toBeUndefined();
  });

  it("resolves the department a hand-transcribed guide table names", () => {
    const transcript = repositoryJson<{
      departmentId: string;
      source: { title: string; url: string }[];
    }>(CS_RECOMMENDED_PATH);
    const resolved = departmentById(transcript.departmentId);
    expect(resolved).toBe(cs);
    expect(resolved!.scope).toBe(cs.scope);
    const bundledCs = bundled2026.departments.find(
      (department) => department.id === "cs",
    )!;
    expect(bundledCs.syllabusScope).toBe(resolved!.scope);
    expect(transcript.source.map((source) => source.url)).toContain(
      bundledCs.guidePdf,
    );
  });
});

describe("the course file limits the settings page edits", () => {
  it("keeps every limit a usable control with the default inside its range", () => {
    for (const [name, policy] of Object.entries(COURSE_FILE_POLICY)) {
      expect(Number.isInteger(policy.min), name).toBe(true);
      expect(Number.isInteger(policy.default), name).toBe(true);
      expect(Number.isInteger(policy.max), name).toBe(true);
      expect(policy.min, name).toBeGreaterThanOrEqual(1);
      expect(policy.default, name).toBeGreaterThanOrEqual(policy.min);
      expect(policy.default, name).toBeLessThanOrEqual(policy.max);
    }
  });
});

describe("the state label shown beside an assignment", () => {
  it("shows the grade as the LMS displays it next to the graded label", () => {
    expect(
      submissionLabel(
        assignment({ submission: "graded", grade: "A" }),
        label,
        false,
      ),
    ).toBe("graded A");
    expect(
      submissionLabel(
        assignment({ submission: "graded", grade: "秀" }),
        label,
        true,
      ),
    ).toBe("graded 秀");
  });

  it("shows only the graded label when the record carries no grade", () => {
    expect(
      submissionLabel(assignment({ submission: "graded" }), label, false),
    ).toBe("graded");
    expect(
      submissionLabel(
        assignment({ submission: "graded", grade: "" }),
        label,
        false,
      ),
    ).toBe("graded");
  });

  it("keeps a recorded grade hidden until Core reports the assignment graded", () => {
    expect(
      submissionLabel(
        assignment({ submission: "submitted", grade: "A" }),
        label,
        true,
      ),
    ).toBe("submitted");
  });

  it("shows the submitted label once the caller marks it submitted", () => {
    expect(
      submissionLabel(assignment({ submission: "open" }), label, true),
    ).toBe("submitted");
  });

  it("shows the state Core recorded while the caller has not submitted it", () => {
    expect(
      submissionLabel(assignment({ submission: "open" }), label, false),
    ).toBe("open");
    expect(
      submissionLabel(assignment({ submission: "submitted" }), label, false),
    ).toBe("submitted");
  });

  it("shows the unknown label when Core recorded no state at all", () => {
    expect(submissionLabel(assignment(), label, false)).toBe("unknown");
  });
});
