// @vitest-environment jsdom
import { act } from "react";
import { MantineProvider } from "@mantine/core";
import { afterEach, beforeAll, describe, expect, it } from "vitest";

import {
  DEVICE,
  TERM,
  click,
  chooseOption,
  emptySchoolRecords,
  installFakeCore,
  mount,
  mountApp,
  overviewFor,
  planningWeekFor,
  scheduleViewFor,
  typeInto,
  until,
  workspaceFixture,
  type FakeCore,
  type Mounted,
  type MountedApp,
} from "@tests/harness";
import { appTheme, cssVariablesResolver } from "@app/app/theme";
import { paths } from "@app/app/routes";
import type {
  Assignment,
  Course,
  GpaSummary,
  TaskView,
  TermId,
  Workspace,
} from "@app/lib/model";
import type { SyllabusCourse } from "@app/lib/syllabus";

/**
 * Today, the lecture desk, the grades panel and syllabus details, driven
 * through the real shell over a scripted Core.
 *
 * The legacy suites mounted `Today`, `CourseFinder`, `LectureDesk`,
 * `AcademicSummary` and `SyllabusDetails` directly and replaced `core-query`,
 * `finder-query` and `@mantine/core` with `vi.mock`. Here the pages are reached
 * through the shipped routes and the harness answers the real query path, so a
 * passing test says something about the app instead of about a mock.
 *
 * Three local helpers stand in where the harness has no equivalent:
 *
 * - `mountCompactToday`: the shipped `Today` is never rendered with `compact`
 *   (only the dev-only `?preview=lms` workbench passes it), so the Today course
 *   list is mounted as the real page over the same scripted host, inside the
 *   shell's `MantineProvider`.
 * - `mountSyllabusDocument`: `variant="document"` is only ever rendered by the
 *   LMS reader's syllabus tab, whose `currentLesson` comes from LMS lesson
 *   state; the real component is mounted directly with a chosen lesson.
 * - `taskViewFor`: the harness has no `TaskView` builder, so the scripted Core
 *   derives one from the workspace the same way Core decides those sets.
 *
 * `freshRevision` exists because the real query caches key a read by the
 * snapshot revision and the module graph stays warm across a file: without a
 * new revision per mount, one test's scripted answer would render in the next.
 */

let app: MountedApp | undefined;
let standalone: Mounted | undefined;
let standaloneCore: FakeCore | undefined;

/**
 * The shell's first import builds the whole App module graph, which takes far
 * longer than any single test's work. Paying it here keeps each `it` timeout
 * about the behaviour it protects; it also captures the bridge before any
 * suite code runs, exactly as the app's own entry point would.
 */
beforeAll(async () => {
  const warm = await mountApp(
    {
      workspace: workspaceFixture(),
      revision: freshRevision(),
      queries: homeQueries,
    },
    { route: paths.home() },
  );
  await warm.unmount();
}, 180_000);

afterEach(async () => {
  await standalone?.unmount();
  standalone = undefined;
  standaloneCore?.restore();
  standaloneCore = undefined;
  await app?.unmount();
  app = undefined;
});

/* ------------------------------------------------------------------ *
 * Local helpers the harness does not provide.
 * ------------------------------------------------------------------ */

function required<T>(
  find: () => T | null | undefined,
  what: string,
): Promise<T> {
  return until(() => {
    const found = find();
    if (!found) throw new Error(`${what} is not ready`);
    return found;
  });
}

/** The native hide is not what these suites are about; answer it quietly. */
function nativeNoise(command: string): unknown {
  return command === "hide_school_web" ? true : undefined;
}

/**
 * The real query caches key a read by the snapshot revision, and the module
 * graph stays warm across a file. A distinct revision per mount keeps one
 * test's scripted answer out of the next test's render.
 */
let revisionCounter = 0;
function freshRevision(): string {
  revisionCounter += 1;
  return `revision:test-${revisionCounter}`;
}

function courseRow(
  id: string,
  title: string,
  overrides: Partial<Course> = {},
): Course {
  return {
    kind: "course",
    id,
    termId: TERM,
    code: id.toUpperCase(),
    title,
    instructor: "",
    credits: 2,
    creditSplit: { lecture: 2, exercise: 0, lab: 0 },
    instructionLanguage: "ja",
    color: "#556b8d",
    meetings: [],
    source: { provider: "lms", externalId: id },
    rev: 1,
    updatedAt: "2026-10-05T00:00:00Z",
    updatedBy: DEVICE,
    ...overrides,
  };
}

function assignmentRow(
  id: string,
  courseId: string,
  title: string,
  dueAt: string,
  overrides: Partial<Assignment> = {},
): Assignment {
  return {
    kind: "assignment",
    id,
    courseId,
    title,
    details: "",
    dueAt,
    submission: "open",
    source: { provider: "lms", externalId: id },
    rev: 1,
    updatedAt: "2026-10-05T00:00:00Z",
    updatedBy: DEVICE,
    ...overrides,
  };
}

/**
 * Core's task view for the fixtures here: which work is pinned, which is open
 * now, what is already past, and which rows count as submitted. The shell only
 * renders these decisions, so the test states them the way Core would.
 */
function taskViewFor(workspace: Workspace, term: TermId = TERM): TaskView {
  const ids = new Set(
    workspace.assignments.filter((row) => !row.deletedAt).map((row) => row.id),
  );
  const pick = (...candidates: string[]) =>
    candidates.filter((id) => ids.has(id));
  const overdue = pick("closed", "missed");
  return {
    deadlines: Object.fromEntries(
      workspace.assignments
        .filter((row) => !row.deletedAt)
        .map((row) => [
          row.id,
          {
            at: row.dueAt ?? "",
            label: "Due soon",
            tone: "normal",
            remainingMilliseconds: 86400000,
            countdownLabel: "1d",
            countdownTone: "soon",
            overdue: overdue.includes(row.id),
          },
        ]),
    ),
    submittedIds: pick("closed", "handed-in"),
    nearestByCourse: {},
    byTerm: {
      [term]: {
        pendingIds: pick("upcoming", "pinned"),
        pinnedIds: pick("pinned"),
        overdueIds: [...overdue].reverse(),
        dueThisWeekIds: pick("upcoming"),
        laterIds: [],
        earlierDueThisWeekIds: pick("upcoming", "handed-in"),
        earlierLaterIds: [],
        lectureOpenIds: pick("a1"),
        lectureHiddenIds: pick("a2"),
      },
    },
  };
}

const homeQueries = {
  workspaceOverview: (_query: unknown, core: FakeCore) =>
    overviewFor(core.workspace),
  planningWeek: (_query: unknown, core: FakeCore) =>
    planningWeekFor(core.workspace),
  scheduleView: (_query: unknown, core: FakeCore) =>
    scheduleViewFor(core.workspace),
};

/* ------------------------------------------------------------------ *
 * Today's assignment scopes.
 * ------------------------------------------------------------------ */

function todayWorkspace(): Workspace {
  const workspace = workspaceFixture({
    courses: [
      courseRow("now", "Computer architecture"),
      courseRow("old", "Linear algebra", { termId: "2026-2Q" }),
    ],
  });
  workspace.assignments = [
    assignmentRow("upcoming", "now", "Report upcoming", "2026-10-09T00:00:00Z"),
    assignmentRow(
      "handed-in",
      "now",
      "Report handed-in",
      "2026-10-12T00:00:00Z",
      {
        submission: "submitted",
      },
    ),
    assignmentRow("missed", "now", "Report missed", "2026-10-03T00:00:00Z"),
    assignmentRow("closed", "now", "Report closed", "2026-10-02T00:00:00Z", {
      submission: "graded",
    }),
    assignmentRow("stale", "old", "Report stale", "2026-08-01T00:00:00Z"),
  ];
  return workspace;
}

async function mountToday(workspace: Workspace): Promise<MountedApp> {
  const mounted = await mountApp(
    {
      workspace,
      revision: freshRevision(),
      queries: {
        ...homeQueries,
        taskView: (_query, core) => taskViewFor(core.workspace),
      },
      handle: nativeNoise,
    },
    { route: paths.home() },
  );
  app = mounted;
  return mounted;
}

function dueTitles(section: Element): (string | null)[] {
  return [...section.querySelectorAll(".dueRow b")].map(
    (node) => node.textContent,
  );
}

function dueRow(section: Element, title: string): Element | undefined {
  return [...section.querySelectorAll(".dueRow")].find((row) =>
    row.textContent?.includes(title),
  );
}

async function assignmentsSection(mounted: MountedApp): Promise<HTMLElement> {
  return required(
    () =>
      mounted.host.querySelector<HTMLElement>(
        '[data-today-section="assignments"]',
      ),
    "the Today assignments section",
  );
}

describe("Today's assignment scopes", () => {
  it("keeps pinned work at the top of Today without listing it twice", async () => {
    const workspace = todayWorkspace();
    workspace.assignments.push(
      assignmentRow("pinned", "now", "Report pinned", "2026-11-12T00:00:00Z", {
        pinned: true,
      }),
    );
    const mounted = await mountToday(workspace);
    const section = await assignmentsSection(mounted);

    await until(() => expect(dueTitles(section)).toContain("Report pinned"));
    const titles = dueTitles(section);
    expect(titles[0]).toBe("Report pinned");
    expect(titles.filter((title) => title === "Report pinned")).toHaveLength(1);
    // Pinned work is still open work, so it stays counted as active.
    expect(section.textContent).toContain("2 active");
  }, 20_000);

  it("shows only open work from now on under Today", async () => {
    const mounted = await mountToday(todayWorkspace());
    const section = await assignmentsSection(mounted);

    await until(() => expect(dueTitles(section)).toContain("Report upcoming"));
    expect(dueTitles(section)).toEqual(["Report upcoming"]);
    expect(section.textContent).toContain("1 active");
  }, 20_000);

  it("moves submitted and past work of this quarter under Earlier", async () => {
    const mounted = await mountToday(todayWorkspace());
    const section = await assignmentsSection(mounted);
    await until(() => expect(dueTitles(section)).toContain("Report upcoming"));

    const earlier = await required(
      () =>
        [
          ...section.querySelectorAll<HTMLButtonElement>(".dueScope button"),
        ].find((button) => button.textContent?.trim() === "Earlier"),
      "the Earlier scope",
    );
    await click(earlier);
    await until(() =>
      expect(dueRow(section, "Report handed-in")).toBeDefined(),
    );

    expect(dueRow(section, "Report upcoming")?.className).toBe("dueRow");
    expect(dueRow(section, "Report handed-in")?.className).toContain("done");
    expect(dueRow(section, "Report missed")?.className).toContain("missed");
    expect(dueRow(section, "Report closed")?.className).toContain("closed");
    // Another quarter's work is not this quarter's Earlier.
    expect(dueRow(section, "Report stale")).toBeUndefined();
    expect(section.textContent).toContain("Past deadline · 2");
  }, 20_000);
});

/* ------------------------------------------------------------------ *
 * The Today course list.
 * ------------------------------------------------------------------ */

async function mountCompactToday(opened: string[]): Promise<Mounted> {
  const data = workspaceFixture({
    courses: [
      courseRow("a", "Old Algebra", { termId: "2025-4Q", code: "MTH.A201" }),
      courseRow("b", "Signals", { termId: "2026-3Q", code: "EEE.S301" }),
      courseRow("c", "Linear Algebra", { termId: "2026-1Q", code: "MTH.A101" }),
    ],
  });
  const core = installFakeCore({
    workspace: data,
    revision: freshRevision(),
    queries: {
      scheduleView: (_query, fake) => scheduleViewFor(fake.workspace),
      taskView: (_query, fake) => taskViewFor(fake.workspace),
    },
    handle: nativeNoise,
  });
  standaloneCore = core;
  const { Today } = await import("@app/pages/today/TodayPage");
  standalone = await mount(
    <MantineProvider
      theme={appTheme}
      cssVariablesResolver={cssVariablesResolver}
    >
      <Today
        compact
        data={data}
        term={TERM}
        open={(courseId) => opened.push(courseId)}
        openEvent={() => undefined}
        openPlan={() => undefined}
        openTask={() => undefined}
        hideAssignment={() => undefined}
        openSchedule={() => undefined}
      />
    </MantineProvider>,
  );
  return standalone;
}

function finderGroups(): (string | null | undefined)[][] {
  return [...document.querySelectorAll(".courseFinderList section")].map(
    (section) => [
      section.querySelector("small")?.textContent,
      ...[...section.querySelectorAll("button span")].map(
        (span) => span.textContent,
      ),
    ],
  );
}

describe("the Today course list", () => {
  it("lists this term first and older quarters newest first", async () => {
    const mounted = await mountCompactToday([]);
    const input = await required(
      () =>
        mounted.host.querySelector<HTMLInputElement>('input[type="search"]'),
      "the course finder input",
    );
    await act(async () => input.focus());
    await required(
      () => document.querySelector(".courseFinderList"),
      "the course list",
    );

    expect(finderGroups()).toEqual([
      ["This term · 2026 · 3Q", "Signals"],
      ["2026 · 1Q", "Linear Algebra"],
      ["2025 · 4Q", "Old Algebra"],
    ]);
  }, 20_000);

  it("filters by title or code and opens the chosen course with Enter", async () => {
    const opened: string[] = [];
    const mounted = await mountCompactToday(opened);
    const input = await required(
      () =>
        mounted.host.querySelector<HTMLInputElement>('input[type="search"]'),
      "the course finder input",
    );
    await act(async () => input.focus());
    await typeInto(input, "algebra");

    await until(() =>
      expect(finderGroups()).toEqual([
        ["2026 · 1Q", "Linear Algebra"],
        ["2025 · 4Q", "Old Algebra"],
      ]),
    );

    await act(async () => {
      input.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      );
    });
    expect(opened).toEqual(["c"]);
    // The list closes behind the choice and the query clears.
    await until(() => expect(input.value).toBe(""));

    // A course code narrows the list just like part of a title.
    await typeInto(input, "MTH.A201");
    await until(() =>
      expect(finderGroups()).toEqual([["2025 · 4Q", "Old Algebra"]]),
    );
  }, 20_000);
});

/* ------------------------------------------------------------------ *
 * The lecture desk.
 * ------------------------------------------------------------------ */

/** A YYYY-MM-DD date `offset` days from the real JST today. */
function jstDay(offset: number): string {
  const at = new Date(Date.now() + offset * 86_400_000);
  return new Intl.DateTimeFormat("en-CA", {
    timeZone: "Asia/Tokyo",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).format(at);
}

function lectureBlock(
  id: string,
  date: string,
  startTime: string,
  endTime: string,
  room: string,
) {
  return {
    key: `${id}-${date}-${startTime}`,
    id,
    kind: "course" as const,
    courseId: id,
    date,
    title: `Course ${id}`,
    subtitle: id.toUpperCase(),
    room,
    startTime,
    endTime,
    color: "#556b8d",
  };
}

function lectureWorkspace(): Workspace {
  const workspace = workspaceFixture({
    courses: [
      courseRow("now", "Course now"),
      courseRow("later", "Course later"),
      courseRow("friday", "Course friday"),
    ],
  });
  workspace.assignments = [
    assignmentRow("a1", "later", "Report", "2026-10-08T15:00:00Z"),
  ];
  return workspace;
}

/**
 * The block at index 0 is the desk's "today", so it is the one that can read
 * "Now". Its window spans the whole day, which keeps the chip independent of
 * the minute the suite happens to run at.
 */
function lectureSchedule(workspace: Workspace) {
  return scheduleViewFor(workspace, {
    allTerms: [TERM],
    agendas: [
      {
        date: jstDay(0),
        occurrences: [lectureBlock("now", jstDay(0), "00:00", "24:00", "R3")],
        importantDates: [],
        noClasses: false,
      },
      {
        date: jstDay(1),
        occurrences: [lectureBlock("later", jstDay(1), "08:50", "10:30", "R3")],
        importantDates: [],
        noClasses: false,
      },
      {
        date: jstDay(2),
        occurrences: [
          lectureBlock("friday", jstDay(2), "08:50", "10:30", "R5"),
        ],
        importantDates: [],
        noClasses: false,
      },
    ],
  });
}

function lectureTasks(workspace: Workspace): TaskView {
  const view = taskViewFor(workspace);
  return {
    ...view,
    nearestByCourse: { later: "a1" },
    deadlines: {
      ...view.deadlines,
      a1: {
        at: "2026-10-08T15:00:00Z",
        label: "Due soon",
        tone: "normal",
        remainingMilliseconds: 129600000,
        countdownLabel: "1d 12h",
        countdownTone: "soon",
        overdue: false,
      },
    },
  };
}

async function mountLectures(workspace: Workspace): Promise<MountedApp> {
  const mounted = await mountApp(
    {
      workspace,
      revision: freshRevision(),
      queries: {
        ...homeQueries,
        scheduleView: (_query, core) => lectureSchedule(core.workspace),
        taskView: (_query, core) => lectureTasks(core.workspace),
        courseTimeline: () => ({
          lastStarted: null,
          next: null,
          classAt: null,
        }),
      },
      handle: nativeNoise,
    },
    { route: paths.lectures() },
  );
  app = mounted;
  return mounted;
}

function lectureRows(host: HTMLElement): HTMLElement[] {
  return [...host.querySelectorAll<HTMLElement>(".lectureList .lectureRow")];
}

function lectureTitles(host: HTMLElement): (string | null | undefined)[] {
  return lectureRows(host).map((row) => row.querySelector("b")?.textContent);
}

async function readyLectureRows(host: HTMLElement): Promise<HTMLElement[]> {
  return required(() => {
    const rows = lectureRows(host);
    return rows.length >= 3 ? rows : undefined;
  }, "the lecture rows");
}

async function assignmentModeButton(
  host: HTMLElement,
): Promise<HTMLButtonElement> {
  return required(
    () =>
      [...host.querySelectorAll<HTMLButtonElement>(".lectureMode button")][1],
    "the Assignments mode",
  );
}

describe("the lecture desk", () => {
  it("lists lectures in class order with the Now chip and each lecture's next deadline", async () => {
    const mounted = await mountLectures(lectureWorkspace());
    const rows = await readyLectureRows(mounted.host);

    expect(rows.map((row) => row.querySelector("b")?.textContent)).toEqual([
      "Course now",
      "Course later",
      "Course friday",
    ]);
    expect(rows[0].querySelector("small")?.textContent).toBe("Now · R3");
    expect(rows[0].className).toContain("now");
    expect(rows[1].querySelector(".dueChip")?.textContent).toBe("1d 12h");
  }, 20_000);

  it("waits for an explicit selection instead of opening a lecture on its own", async () => {
    const mounted = await mountLectures(lectureWorkspace());
    await readyLectureRows(mounted.host);

    expect(window.location.hash.replace(/^#/, "")).toBe(paths.lectures());
    expect(mounted.host.querySelector(".lectureRow.selected")).toBeNull();
    expect(
      mounted.host.querySelector('.lectureRow[aria-current="true"]'),
    ).toBeNull();
    expect(mounted.core.intents).toEqual([]);
  }, 20_000);

  it("filters the lecture list by title or code", async () => {
    const mounted = await mountLectures(lectureWorkspace());
    await readyLectureRows(mounted.host);
    const filter = await required(
      () =>
        mounted.host.querySelector<HTMLInputElement>(
          "input.lectureFilter, .lectureFilter input",
        ),
      "the lecture filter",
    );

    await typeInto(filter, "FRI");
    await until(() =>
      expect(lectureTitles(mounted.host)).toEqual(["Course friday"]),
    );

    await typeInto(filter, "later");
    await until(() =>
      expect(lectureTitles(mounted.host)).toEqual(["Course later"]),
    );
  }, 20_000);

  it("switches to the quarter's open assignments and opens the chosen one", async () => {
    const mounted = await mountLectures(lectureWorkspace());
    await readyLectureRows(mounted.host);

    await click(await assignmentModeButton(mounted.host));
    const work = await required(() => {
      const rows = [
        ...mounted.host.querySelectorAll<HTMLButtonElement>(
          ".lectureList button.lectureRow",
        ),
      ];
      return rows.length ? rows : undefined;
    }, "the open assignment rows");

    expect(work.map((row) => row.querySelector("b")?.textContent)).toEqual([
      "Report",
    ]);
    expect(work[0].querySelector("small")?.textContent).toBe("Course later");
    expect(work[0].querySelector(".dueChip")?.textContent).toBe("1d 12h");

    await click(work[0]);
    await until(() =>
      expect(window.location.hash).toContain(paths.lectures("later", "a1")),
    );
  }, 20_000);

  it("keeps the open assignments when the quarter changes in Assignments mode", async () => {
    // Regression: the task view answers for the new quarter at once while that
    // quarter's courses are still loading, and the list read a missing course.
    const mounted = await mountApp(
      {
        workspace: lectureWorkspace(),
        revision: freshRevision(),
        queries: {
          ...homeQueries,
          scheduleView: (_query, core) => ({
            ...lectureSchedule(core.workspace),
            allTerms: [TERM, "2026-4Q"],
          }),
          taskView: (_query, core) => lectureTasks(core.workspace),
          courseTimeline: () => ({
            lastStarted: null,
            next: null,
            classAt: null,
          }),
        },
        handle: nativeNoise,
      },
      { route: paths.lectures() },
    );
    app = mounted;
    await readyLectureRows(mounted.host);
    await click(await assignmentModeButton(mounted.host));
    await until(() => expect(mounted.host.textContent).toContain("Report"));
    const quarter = await required(
      () =>
        mounted.host.querySelector<HTMLInputElement>(
          '.lectureTerm [role="combobox"]',
        ),
      "the quarter picker",
    );
    const choose = (term: TermId) => chooseOption(quarter, term);

    expect(mounted.host.querySelector("select.lectureTerm")).toBeNull();
    await click(quarter);
    await act(async () => {
      quarter.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "ArrowUp",
          code: "ArrowUp",
          bubbles: true,
        }),
      );
    });
    await act(async () => {
      quarter.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "Enter",
          code: "Enter",
          bubbles: true,
        }),
      );
    });
    expect(quarter.getAttribute("aria-expanded")).toBe("false");
    await until(() =>
      expect(mounted.host.textContent).toContain("No open assignments."),
    );
    await choose(TERM);
    await until(() => expect(mounted.host.textContent).toContain("Report"));
    expect(mounted.host.querySelector(".pageError")).toBeNull();
  }, 20_000);

  it("folds hidden assignments away until asked, then restores one", async () => {
    const workspace = lectureWorkspace();
    workspace.assignments.push(
      assignmentRow("a2", "now", "Quiz", "2026-10-09T15:00:00Z", {
        hidden: true,
      }),
    );
    const mounted = await mountLectures(workspace);
    await readyLectureRows(mounted.host);

    await click(await assignmentModeButton(mounted.host));
    await until(() => expect(mounted.host.textContent).toContain("Report"));
    expect(mounted.host.textContent).not.toContain("Quiz");

    const toggle = await required(
      () =>
        mounted.host.querySelector<HTMLButtonElement>(".lectureHiddenToggle"),
      "the hidden-assignment toggle",
    );
    await click(toggle);
    const hidden = await required(
      () => mounted.host.querySelector<HTMLElement>(".lectureRow.hidden"),
      "the folded hidden assignment",
    );
    expect(hidden.querySelector("b")?.textContent).toBe("Quiz");
    expect(hidden.querySelector("small")?.textContent).toBe("Course now");

    await click(hidden.querySelector("button"));
    await until(() =>
      expect(mounted.core.intents).toContainEqual({
        kind: "setAssignmentMark",
        assignmentId: "a2",
        hidden: false,
      }),
    );
  }, 20_000);
});

/* ------------------------------------------------------------------ *
 * The grades panel.
 * ------------------------------------------------------------------ */

const gpaSummary: GpaSummary = {
  id: "kyomu-gpa:all",
  scope: "all",
  gpa: 3.2,
  gpt: 100,
  earnedCredits: 10,
  fetchedAt: "2026-09-01T00:00:00Z",
  rev: 1,
  updatedAt: "2026-09-01T00:00:00Z",
  updatedBy: "kyomu",
};

async function mountGrades(
  summaries: GpaSummary[],
  availability: {
    state: "available" | "unavailable";
    checkedAt: string;
    message: string;
  },
): Promise<MountedApp> {
  const workspace = workspaceFixture();
  workspace.gpaSummary = summaries;
  const schoolRecords = emptySchoolRecords();
  schoolRecords.pageAvailability = { "kyomu:gpa": availability };
  const mounted = await mountApp(
    {
      workspace,
      schoolRecords,
      revision: freshRevision(),
      queries: homeQueries,
      handle: nativeNoise,
    },
    { route: paths.timetable() },
  );
  app = mounted;

  const details = await required(
    () =>
      mounted.host.querySelector<HTMLButtonElement>(".academicDetailsTrigger"),
    "the academic summary Details button",
  );
  await click(details);
  await required(
    () => document.querySelector(".academicPanel"),
    "the academic panel",
  );
  return mounted;
}

describe("the grades panel", () => {
  it("reports the capability check instead of calling an unavailable GPA page unfetched", async () => {
    await mountGrades([], {
      state: "unavailable",
      checkedAt: "2026-10-15T00:00:00Z",
      message: "このページは利用できません",
    });

    expect(document.body.textContent).toContain(
      "This GPA page is not provided for this account.",
    );
    expect(document.body.textContent).toContain(
      "Credits for multi-quarter courses are split equally across quarters.",
    );
    expect(document.body.textContent).toContain("Checked:");
    expect(document.body.textContent).not.toContain("Not fetched yet");
  }, 20_000);

  it("labels retained GPA rows as coming from an earlier fetch", async () => {
    await mountGrades([gpaSummary], {
      state: "unavailable",
      checkedAt: "2026-10-15T00:00:00Z",
      message: "このページは利用できません",
    });

    expect(document.body.textContent).toContain(
      "The values below were retained from an earlier successful fetch.",
    );
    expect(document.body.textContent).toContain("Previous fetch:");
    expect(document.body.textContent).toContain("3.20");
  }, 20_000);
});

/* ------------------------------------------------------------------ *
 * Syllabus details.
 * ------------------------------------------------------------------ */

/** An official offering-ID locator, which redirects to the dated syllabus. */
const SYLLABUS_LOCATOR =
  "https://syllabus.s.isct.ac.jp/courses/-/-/-/202600001";

function englishSyllabus(): SyllabusCourse {
  const url = `${SYLLABUS_LOCATOR}?hl=en`;
  return {
    externalId: "202600001",
    year: 2026,
    quarter: "3Q",
    code: "CSC.T101",
    title: "Assembly",
    instructor: "Teacher",
    url,
    content: {
      schemaVersion: 1,
      sourceUrl: url,
      sourceLanguage: "en",
      fetchedAt: "2026-10-05",
      sourceUpdated: null,
      metadata: [{ label: "Credits", value: "2" }],
      sections: [
        {
          key: "overview",
          heading: "Overview",
          blocks: [{ kind: "paragraph", text: "English course objectives" }],
          links: [],
        },
      ],
      lecturePlan: [],
      competencies: [],
    },
  };
}

function documentSyllabus(): SyllabusCourse {
  const url = `${SYLLABUS_LOCATOR}?hl=en`;
  return {
    externalId: "202600001",
    year: 2026,
    quarter: "3Q",
    code: "CSC.T101",
    title: "Assembly",
    instructor: "Teacher",
    url,
    content: {
      schemaVersion: 1,
      sourceUrl: url,
      sourceLanguage: "en",
      fetchedAt: "2026-10-05",
      sourceUpdated: null,
      metadata: [{ label: "Credits", value: "2" }],
      sections: [
        {
          key: "overview",
          heading: "Course overview",
          blocks: [{ kind: "paragraph", text: "Machine instructions" }],
          links: [],
        },
      ],
      lecturePlan: [
        { label: "Class 1", topic: "Introduction", objective: "" },
        { label: "Class 2", topic: "Registers", objective: "" },
        { label: "Class 3", topic: "Memory", objective: "" },
      ],
      competencies: [],
    },
  };
}

async function mountSyllabusDocument(): Promise<Mounted> {
  standaloneCore = installFakeCore({
    workspace: workspaceFixture(),
    revision: freshRevision(),
  });
  const { SyllabusDetails } =
    await import("@app/pages/courses/syllabus-details");
  standalone = await mount(
    <MantineProvider
      theme={appTheme}
      cssVariablesResolver={cssVariablesResolver}
    >
      <SyllabusDetails
        variant="document"
        currentLesson={2}
        url={`${SYLLABUS_LOCATOR}?hl=en`}
        fetchCourse={async () => documentSyllabus()}
      />
    </MantineProvider>,
  );
  return standalone;
}

describe("syllabus details", () => {
  it("resolves an official offering-ID locator into the canonical English syllabus", async () => {
    const course = courseRow("offering", "Assembly", {
      offeringId: "202600001",
      source: { provider: "syllabus", externalId: "202600001" },
    });
    app = await mountApp(
      {
        workspace: workspaceFixture({ courses: [course] }),
        revision: freshRevision(),
        // Core derives the locator from the offering id; the drawer shows it.
        queries: {
          ...homeQueries,
          courseDetail: { syllabusUrl: SYLLABUS_LOCATOR },
        },
        handle: (command) => {
          if (command === "syllabus_course") return englishSyllabus();
          return nativeNoise(command);
        },
      },
      { route: paths.drawer(paths.home(), course.id, "syllabus") },
    );

    await until(() =>
      expect(document.body.textContent).toContain("English course objectives"),
    );
    // The locator itself is what the host was asked for; the canonical page
    // comes back from it.
    expect(
      app.core
        .commands("syllabus_course")
        .some((call) => call.args.id === SYLLABUS_LOCATOR),
    ).toBe(true);
    expect(document.body.textContent).toContain("Credits");
  }, 20_000);

  it("reads as a document in the course reader: facts, parts and the current lesson", async () => {
    const mounted = await mountSyllabusDocument();
    const plan = await required(
      () => mounted.host.querySelector(".syllabusPlan"),
      "the lesson plan",
    );

    // Nothing folds: no disclosure triangles in the reader.
    expect(mounted.host.querySelector("details")).toBeNull();
    expect(mounted.host.querySelector(".syllabusFacts")?.textContent).toContain(
      "Credits",
    );
    expect(mounted.host.querySelector(".syllabusPart h3")?.textContent).toBe(
      "Course overview",
    );
    expect(mounted.host.querySelector(".syllabusPart")?.textContent).toContain(
      "Machine instructions",
    );
    expect(
      [...plan.querySelectorAll(":scope > li")].map((row) => row.className),
    ).toEqual(["held", "current", ""]);
  }, 20_000);
});
