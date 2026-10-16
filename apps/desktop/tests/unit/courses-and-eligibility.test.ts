import { describe, expect, it } from "vitest";

import { COURSE_FILE_POLICY as CORE_POLICY } from "@app/generated/persistence";
import { submissionLabel } from "@app/lib/academic";
import { COURSE_FILE_POLICY } from "@app/lib/course-file-policy";
import { DAYS, LOCAL_TYPES } from "@app/lib/course-query";
import { SYLLABUS_DEPARTMENTS, departmentById } from "@app/lib/departments";
import type { Assignment, Course } from "@app/lib/model";
import { isSyllabusCourse } from "@app/lib/syllabus";
import {
  syllabusIdentity,
  validSyllabusContent,
} from "@app/lib/syllabus-content";
import { courseFor, offering, readRepoFile } from "@tests/harness";

/**
 * The parts of the course pipeline that decide what the finder may offer, which
 * syllabus pages the app is willing to fetch and show, and which department the
 * study profile refers to.
 *
 * These modules had no suite of their own: the finder constants, the department
 * table, the syllabus page identity and content guard, the generated course-file
 * limits and the assignment state label were only exercised through the panels
 * that render them. The expectations below come from the modules' contracts and
 * from the two independent descriptions of the same real data that the
 * repository commits — the syllabus structure Core and the shell share, and the
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

/** Keys the UI catalogue publishes; anything else must never reach the user. */
const SUBMISSION_LABELS: Record<string, string> = {
  graded: "Graded",
  submitted: "Submitted",
  open: "Open",
  unknown: "Unknown",
};
const label = (key: string) => SUBMISSION_LABELS[key] ?? `⟨${key}⟩`;

const DEPARTMENTS_PATH =
  "crates/isct-core/tests/fixtures/subscriptions/data/2026/departments.json";
const SYLLABUS_EXAMPLES_PATH =
  "crates/isct-core/tests/fixtures/syllabus/course-examples-parsed.json";
const CS_RECOMMENDED_PATH = "docs/tasks/data/cs-2026-recommended.json";

const SYLLABUS_PAGE =
  "https://syllabus.s.isct.ac.jp/courses/2026/4/0-904-342300-0-0";
const CANONICAL_SYLLABUS_URL = `${SYLLABUS_PAGE}/202602440`;
const ID_ONLY_SYLLABUS_URL =
  "https://syllabus.s.isct.ac.jp/courses/-/-/-/202602440";

type SyllabusExample = {
  externalId: string;
  url: string;
  content: unknown;
};

const syllabusExamples = repositoryJson<SyllabusExample[]>(
  SYLLABUS_EXAMPLES_PATH,
);

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

describe("the identity of a syllabus page", () => {
  it("returns the canonical dated page unchanged", () => {
    expect(syllabusIdentity(CANONICAL_SYLLABUS_URL)).toBe(
      CANONICAL_SYLLABUS_URL,
    );
  });

  it("keeps the English marker so the two language pages stay apart", () => {
    expect(syllabusIdentity(`${CANONICAL_SYLLABUS_URL}?hl=en`)).toBe(
      `${CANONICAL_SYLLABUS_URL}?hl=en`,
    );
  });

  it("treats a Japanese page, the default, as the bare canonical URL", () => {
    expect(syllabusIdentity(`${CANONICAL_SYLLABUS_URL}?hl=ja`)).toBe(
      CANONICAL_SYLLABUS_URL,
    );
  });

  it("differs between the languages only by the English marker", () => {
    const japanese = syllabusIdentity(CANONICAL_SYLLABUS_URL);
    const english = syllabusIdentity(`${CANONICAL_SYLLABUS_URL}?hl=en`);
    expect(english).toBe(`${japanese}?hl=en`);
  });

  it("drops every other query parameter and the fragment", () => {
    expect(
      syllabusIdentity(`${CANONICAL_SYLLABUS_URL}?hl=en&from=finder#plan`),
    ).toBe(`${CANONICAL_SYLLABUS_URL}?hl=en`);
  });

  it("accepts the id-only locator the app builds from an offering id", () => {
    expect(syllabusIdentity(ID_ONLY_SYLLABUS_URL)).toBe(ID_ONLY_SYLLABUS_URL);
  });

  it("refuses a page on another host or an insecure scheme", () => {
    expect(
      syllabusIdentity(CANONICAL_SYLLABUS_URL.replace("https://", "http://")),
    ).toBeUndefined();
    expect(
      syllabusIdentity(
        CANONICAL_SYLLABUS_URL.replace(
          "syllabus.s.isct.ac.jp",
          "syllabus.example.com",
        ),
      ),
    ).toBeUndefined();
  });

  it("refuses a syllabus host reached on a port or with embedded credentials", () => {
    expect(
      syllabusIdentity(
        CANONICAL_SYLLABUS_URL.replace(
          "syllabus.s.isct.ac.jp",
          "syllabus.s.isct.ac.jp:8443",
        ),
      ),
    ).toBeUndefined();
    expect(
      syllabusIdentity(
        CANONICAL_SYLLABUS_URL.replace("https://", "https://user:secret@"),
      ),
    ).toBeUndefined();
  });

  it("refuses a path that is not a course page at all", () => {
    for (const value of [
      "https://syllabus.s.isct.ac.jp/",
      "https://syllabus.s.isct.ac.jp/courses/2026",
      "https://syllabus.s.isct.ac.jp/courses/-/-/-/abc",
      "https://syllabus.s.isct.ac.jp/courses/-/-/-/12345678",
    ])
      expect(syllabusIdentity(value), value).toBeUndefined();
  });

  it("refuses a language it cannot serve", () => {
    expect(syllabusIdentity(`${CANONICAL_SYLLABUS_URL}?hl=fr`)).toBeUndefined();
    expect(syllabusIdentity(`${CANONICAL_SYLLABUS_URL}?hl=EN`)).toBeUndefined();
  });

  it("refuses text that is not a URL", () => {
    expect(syllabusIdentity("not a syllabus url")).toBeUndefined();
    expect(syllabusIdentity("")).toBeUndefined();
  });
});

/** The smallest page the reader is willing to display, in one shape. */
function syllabusContentFixture(
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    schemaVersion: 1,
    sourceUrl: `${CANONICAL_SYLLABUS_URL}?hl=en`,
    sourceLanguage: "en",
    fetchedAt: "2026-09-10T03:14:11Z",
    sourceUpdated: null,
    metadata: [{ label: "Course Code", value: "CSC.T243" }],
    sections: [],
    lecturePlan: [],
    competencies: [],
    ...overrides,
  };
}

describe("the syllabus content a reader will trust", () => {
  it("accepts a page that only carries its metadata table", () => {
    expect(validSyllabusContent(syllabusContentFixture())).toBe(true);
  });

  it("accepts a page whose only prose is a section block", () => {
    expect(
      validSyllabusContent(
        syllabusContentFixture({
          metadata: [],
          sections: [
            {
              key: "overview",
              heading: "Course overview and goals",
              blocks: [{ kind: "paragraph", text: "Graph Model." }],
              links: [],
            },
          ],
        }),
      ),
    ).toBe(true);
  });

  it("accepts every block kind the reader renders", () => {
    expect(
      validSyllabusContent(
        syllabusContentFixture({
          metadata: [],
          sections: [
            {
              key: "overview",
              heading: "Course overview and goals",
              blocks: [
                { kind: "subheading", text: "Aim" },
                { kind: "listItem", text: "First" },
                { kind: "tableRow", text: "1 · Mon", cells: ["1", "Mon"] },
              ],
              links: [],
            },
          ],
        }),
      ),
    ).toBe(true);
  });

  it("accepts a page whose only content is a cited link", () => {
    expect(
      validSyllabusContent(
        syllabusContentFixture({
          metadata: [],
          sections: [
            {
              key: "overview",
              heading: "Course overview and goals",
              blocks: [],
              links: [
                { label: "Study guide", url: "https://example.com/g.pdf" },
              ],
            },
          ],
        }),
      ),
    ).toBe(true);
  });

  it("accepts a lesson plan without any metadata or prose", () => {
    expect(
      validSyllabusContent(
        syllabusContentFixture({
          metadata: [],
          lecturePlan: [
            { label: "1", topic: "Introduction", objective: "Understand" },
          ],
        }),
      ),
    ).toBe(true);
  });

  it("keeps http links because the school pages cite both schemes", () => {
    expect(
      validSyllabusContent(
        syllabusContentFixture({
          metadata: [],
          sections: [
            {
              key: "overview",
              heading: "Course overview and goals",
              blocks: [],
              links: [
                { label: "Guide", url: "http://www.titech.ac.jp/guide/" },
              ],
            },
          ],
        }),
      ),
    ).toBe(true);
  });

  it("accepts a source-updated date as text and as absent", () => {
    expect(
      validSyllabusContent(
        syllabusContentFixture({ sourceUpdated: "Mar 5, 2026" }),
      ),
    ).toBe(true);
    expect(
      validSyllabusContent(syllabusContentFixture({ sourceUpdated: null })),
    ).toBe(true);
  });

  it("refuses anything that is not a content object", () => {
    for (const value of [null, undefined, "text", 42])
      expect(validSyllabusContent(value)).toBe(false);
  });

  it("refuses a schema version it does not understand", () => {
    expect(
      validSyllabusContent(syllabusContentFixture({ schemaVersion: 2 })),
    ).toBe(false);
  });

  it("refuses content that is not tied to a syllabus page", () => {
    expect(
      validSyllabusContent(
        syllabusContentFixture({
          sourceUrl: "https://example.com/courses/2026/x/202602440",
        }),
      ),
    ).toBe(false);
  });

  it("refuses a page whose declared language disagrees with its source URL", () => {
    expect(
      validSyllabusContent(
        syllabusContentFixture({
          sourceUrl: `${CANONICAL_SYLLABUS_URL}?hl=ja`,
        }),
      ),
    ).toBe(false);
  });

  it("refuses a language the reader cannot display", () => {
    expect(
      validSyllabusContent(syllabusContentFixture({ sourceLanguage: "zh" })),
    ).toBe(false);
  });

  it("refuses a fetch time that is not a machine timestamp", () => {
    for (const fetchedAt of ["not-a-date", ""])
      expect(
        validSyllabusContent(syllabusContentFixture({ fetchedAt })),
        fetchedAt,
      ).toBe(false);
  });

  it("refuses a source-updated value that is neither text nor absent", () => {
    expect(
      validSyllabusContent(syllabusContentFixture({ sourceUpdated: 42 })),
    ).toBe(false);
  });

  it("refuses a page with nothing to read", () => {
    expect(
      validSyllabusContent(
        syllabusContentFixture({
          metadata: [],
          sections: [
            {
              key: "overview",
              heading: "Course overview and goals",
              blocks: [{ kind: "paragraph", text: "   " }],
              links: [],
            },
          ],
          lecturePlan: [],
        }),
      ),
    ).toBe(false);
  });

  it("refuses a metadata row that is not a label and a value", () => {
    expect(
      validSyllabusContent(
        syllabusContentFixture({
          metadata: [{ label: "Course Code", value: 3 }],
        }),
      ),
    ).toBe(false);
  });

  it("refuses a block kind the reader cannot render", () => {
    expect(
      validSyllabusContent(
        syllabusContentFixture({
          sections: [
            {
              key: "overview",
              heading: "Course overview and goals",
              blocks: [{ kind: "video", text: "Watch this" }],
              links: [],
            },
          ],
        }),
      ),
    ).toBe(false);
  });

  it("refuses a table row whose cells are not all text", () => {
    expect(
      validSyllabusContent(
        syllabusContentFixture({
          sections: [
            {
              key: "overview",
              heading: "Course overview and goals",
              blocks: [{ kind: "tableRow", text: "1 · Mon", cells: ["1", 2] }],
              links: [],
            },
          ],
        }),
      ),
    ).toBe(false);
  });

  it("refuses a link the reader will not follow", () => {
    expect(
      validSyllabusContent(
        syllabusContentFixture({
          metadata: [],
          sections: [
            {
              key: "overview",
              heading: "Course overview and goals",
              blocks: [],
              links: [{ label: "Click", url: "javascript:alert(1)" }],
            },
          ],
        }),
      ),
    ).toBe(false);
  });

  it("refuses a link with embedded credentials", () => {
    expect(
      validSyllabusContent(
        syllabusContentFixture({
          metadata: [],
          sections: [
            {
              key: "overview",
              heading: "Course overview and goals",
              blocks: [],
              links: [
                { label: "Click", url: "https://user:secret@example.com/x" },
              ],
            },
          ],
        }),
      ),
    ).toBe(false);
  });

  it("refuses a link whose target cannot be parsed at all", () => {
    expect(
      validSyllabusContent(
        syllabusContentFixture({
          metadata: [],
          sections: [
            {
              key: "overview",
              heading: "Course overview and goals",
              blocks: [],
              links: [{ label: "Click", url: "/relative" }],
            },
          ],
        }),
      ),
    ).toBe(false);
  });

  it("refuses more metadata rows than the reader accepts", () => {
    const rows = (count: number) =>
      Array.from({ length: count }, (_, index) => ({
        label: `Row ${index}`,
        value: "x",
      }));
    expect(
      validSyllabusContent(syllabusContentFixture({ metadata: rows(80) })),
    ).toBe(true);
    expect(
      validSyllabusContent(syllabusContentFixture({ metadata: rows(81) })),
    ).toBe(false);
  });

  it("refuses a payload over the transport cap even when each row is fine", () => {
    const rows = (count: number) =>
      Array.from({ length: count }, (_, index) => ({
        label: `Row ${index}`,
        value: "x".repeat(20_000),
      }));
    // Every row is inside the row cap and the per-field cap, so a page just
    // under the whole-payload limit is accepted and one over it is not.
    const nearCap = syllabusContentFixture({ metadata: rows(40) });
    const oversized = syllabusContentFixture({ metadata: rows(80) });
    expect(JSON.stringify(nearCap).length).toBeLessThanOrEqual(1_000_000);
    expect(validSyllabusContent(nearCap)).toBe(true);
    expect(JSON.stringify(oversized).length).toBeGreaterThan(1_000_000);
    expect(validSyllabusContent(oversized)).toBe(false);
  });
});

describe("the committed syllabus examples", () => {
  it("holds at least one parsed example", () => {
    expect(syllabusExamples.length).toBeGreaterThan(0);
  });

  it("accepts every committed example as content the reader may display", () => {
    for (const example of syllabusExamples)
      expect(validSyllabusContent(example.content), example.externalId).toBe(
        true,
      );
  });

  it("gives every committed example a page identity matching its own URL", () => {
    for (const example of syllabusExamples) {
      const { sourceUrl } = example.content as { sourceUrl: string };
      expect(syllabusIdentity(example.url), example.externalId).toBeDefined();
      expect(syllabusIdentity(sourceUrl), example.externalId).toBe(
        syllabusIdentity(example.url),
      );
    }
  });
});

describe("deciding which placed courses have a syllabus", () => {
  const placed = (overrides: Partial<Course> = {}) =>
    courseFor(
      offering("202602440", "CSC.T243", "Procedural Programming"),
      overrides,
    );

  it("accepts a course Core sourced from the syllabus, even without a URL", () => {
    expect(isSyllabusCourse(placed({ url: undefined }))).toBe(true);
    expect(
      isSyllabusCourse(
        placed({
          source: { provider: "syllabus" },
          url: "https://example.com/not-a-syllabus",
        }),
      ),
    ).toBe(true);
  });

  it("accepts a school course whose timetable entry links to the syllabus", () => {
    expect(
      isSyllabusCourse(
        placed({ source: { provider: "school" }, url: CANONICAL_SYLLABUS_URL }),
      ),
    ).toBe(true);
    expect(
      isSyllabusCourse(
        placed({ source: { provider: "school" }, url: ID_ONLY_SYLLABUS_URL }),
      ),
    ).toBe(true);
  });

  it("refuses a school course with no URL to follow", () => {
    const school = placed({ source: { provider: "school" }, url: undefined });
    expect(isSyllabusCourse(school)).toBe(false);
  });

  it("refuses a school course whose URL is not a syllabus page", () => {
    expect(
      isSyllabusCourse(
        placed({
          source: { provider: "school" },
          url: "https://syllabus.s.isct.ac.jp/",
        }),
      ),
    ).toBe(false);
    expect(
      isSyllabusCourse(
        placed({
          source: { provider: "school" },
          url: "https://lms.example.com/course/view.php?id=1",
        }),
      ),
    ).toBe(false);
  });

  it("refuses a course from another source even when it cites a syllabus URL", () => {
    for (const provider of ["lms", "manual", "subscription"] as const)
      expect(
        isSyllabusCourse(
          placed({ source: { provider }, url: CANONICAL_SYLLABUS_URL }),
        ),
        provider,
      ).toBe(false);
  });
});

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

  it("resolves a plain department id", () => {
    expect(departmentById("cs")).toBe(cs);
  });

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

describe("the weekday labels the finder and timetable print", () => {
  it("names the six teaching days from Monday", () => {
    expect([...DAYS]).toEqual(["月", "火", "水", "木", "金", "土"]);
  });

  it("looks up a day number of one to six by subtracting one", () => {
    const names = [1, 2, 3, 4, 5, 6].map((day) => DAYS[day - 1]);
    expect(names).toEqual(["月", "火", "水", "木", "金", "土"]);
  });

  it("leaves a seventh day unnamed so no Sunday cell is mislabelled", () => {
    expect(DAYS.at(6)).toBeUndefined();
  });
});

describe("the finder conditions the loaded context can settle", () => {
  it("marks exactly the recommendation group, mark and free-slot conditions", () => {
    expect([...LOCAL_TYPES].sort()).toEqual(["free", "group", "mark"]);
  });

  it("does not mark conditions that need the school to answer", () => {
    for (const type of ["dept", "code", "level", "text"])
      expect(LOCAL_TYPES.has(type), type).toBe(false);
  });
});

describe("the course file limits the settings page edits", () => {
  it("re-exports the policy Core generates instead of keeping a second copy", () => {
    expect(COURSE_FILE_POLICY).toBe(CORE_POLICY);
  });

  it("exposes the two limits the storage settings offer", () => {
    expect(Object.keys(COURSE_FILE_POLICY).sort()).toEqual([
      "autoDownloadLimitMb",
      "retentionDays",
    ]);
  });

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
    ).toBe("Graded A");
    expect(
      submissionLabel(
        assignment({ submission: "graded", grade: "秀" }),
        label,
        true,
      ),
    ).toBe("Graded 秀");
  });

  it("shows only the graded label when the record carries no grade", () => {
    expect(
      submissionLabel(assignment({ submission: "graded" }), label, false),
    ).toBe("Graded");
    expect(
      submissionLabel(
        assignment({ submission: "graded", grade: "" }),
        label,
        false,
      ),
    ).toBe("Graded");
  });

  it("keeps a recorded grade hidden until Core reports the assignment graded", () => {
    expect(
      submissionLabel(
        assignment({ submission: "submitted", grade: "A" }),
        label,
        true,
      ),
    ).toBe("Submitted");
  });

  it("shows the submitted label once the caller marks it submitted", () => {
    expect(
      submissionLabel(assignment({ submission: "open" }), label, true),
    ).toBe("Submitted");
  });

  it("shows the state Core recorded while the caller has not submitted it", () => {
    expect(
      submissionLabel(assignment({ submission: "open" }), label, false),
    ).toBe("Open");
    expect(
      submissionLabel(assignment({ submission: "submitted" }), label, false),
    ).toBe("Submitted");
  });

  it("shows the unknown label when Core recorded no state at all", () => {
    expect(submissionLabel(assignment(), label, false)).toBe("Unknown");
  });
});
