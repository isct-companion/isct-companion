// @vitest-environment jsdom
import { act } from "react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";

import {
  DEVICE,
  TERM,
  click,
  chooseOption,
  emptySchoolRecords,
  mountApp,
  overviewFor,
  planningWeekFor,
  lectureListFor,
  lectureSearchFor,
  typeInto,
  until,
  workspaceFixture,
  type FakeCore,
  type MountedApp,
} from "@tests/harness";
import { paths } from "@app/app/routes";
import type {
  Assignment,
  Course,
  GpaSummary,
  TaskView,
  TermId,
  Workspace,
} from "@app/lib/model";
import type { AssignmentGroups } from "@app/generated/persistence";

/**
 * Today, the lecture desk and the grades panel, driven through the real shell
 * over a scripted Core.
 *
 * The legacy suites mounted `Today`, `CourseFinder`, `LectureDesk` and
 * `AcademicSummary` directly and replaced `core-query`,
 * `finder-query` and UI components with `vi.mock`. Here the pages are reached
 * through the shipped routes and the harness answers the real query path, so a
 * passing test says something about the app instead of about a mock.
 *
 * `taskViewFor` supplies the assignment views needed by these journeys.
 */

let app: MountedApp | undefined;

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
      queries: homeQueries,
    },
    { route: paths.home() },
  );
  await warm.unmount();
}, 180_000);

afterEach(async () => {
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
 * Core's task view for the fixtures here: Today's open work and recent
 * misses, the quarter's full record, and which rows count as submitted. The
 * shell only renders these decisions, so the test states them the way Core
 * would.
 */
function taskViewFor(workspace: Workspace, term: TermId = TERM): TaskView {
  const ids = new Set(
    workspace.assignments.filter((row) => !row.deletedAt).map((row) => row.id),
  );
  const pick = (...candidates: string[]) =>
    candidates.filter((id) => ids.has(id));
  const overdue = pick("closed", "missed", "early");
  const groups = (
    pinned: string[],
    thisWeek: string[],
    past: string[],
    rest: Partial<AssignmentGroups> = {},
  ): AssignmentGroups => ({
    pinnedIds: pinned,
    dueThisWeekIds: thisWeek,
    laterIds: [],
    undatedIds: [],
    pastRecentIds: past,
    pastEarlierIds: [],
    hiddenIds: [],
    ...rest,
  });
  return {
    deadlines: Object.fromEntries(
      workspace.assignments
        .filter((row) => !row.deletedAt && row.dueAt)
        .map((row) => [
          row.id,
          {
            at: row.dueAt ?? "",
            units: [{ unit: "days" as const, count: 1 }],
            state: overdue.includes(row.id)
              ? ("overdue" as const)
              : ("remaining" as const),
            countdownTone: "soon",
            nextChangeAt: null,
          },
        ]),
    ),
    states: Object.fromEntries(
      workspace.assignments.map((row) => [
        row.id,
        pick("closed", "handed-in").includes(row.id)
          ? "submitted"
          : overdue.includes(row.id)
            ? "closed"
            : "open",
      ]),
    ),
    handInAvailability: {},
    nearestByCourse: {},
    byTerm: {
      [term]: {
        pendingIds: pick("upcoming", "pinned"),
        today: groups(pick("pinned"), pick("upcoming", "a1"), pick("missed")),
        record: groups(
          pick("pinned"),
          pick("upcoming", "handed-in", "a1"),
          pick("missed", "closed"),
          { pastEarlierIds: pick("early"), hiddenIds: pick("a2") },
        ),
      },
    },
  };
}

const homeQueries = {
  calendarRange: () => [],
  upcomingExams: () => [],
  workspaceOverview: (_query: unknown, core: FakeCore) =>
    overviewFor(core.workspace),
  planningWeek: (_query: unknown, core: FakeCore) =>
    planningWeekFor(core.workspace),
  lectureList: (_query: unknown, core: FakeCore) =>
    lectureListFor(core.workspace),
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

  it("shows open work and the last week's misses, never completed work", async () => {
    const mounted = await mountToday(todayWorkspace());
    const section = await assignmentsSection(mounted);

    await until(() => expect(dueTitles(section)).toContain("Report upcoming"));
    expect(dueTitles(section)).toEqual(["Report upcoming", "Report missed"]);
    expect(dueRow(section, "Report missed")?.className).toContain("missed");
    expect(section.textContent).toContain("Missed · 1");
    expect(dueRow(section, "Report handed-in")).toBeUndefined();
    expect(dueRow(section, "Report closed")).toBeUndefined();
    expect(dueRow(section, "Report stale")).toBeUndefined();
    expect(section.textContent).toContain("1 active");
  }, 20_000);

  it("opens the quarter's full record on Lecture Desk, older work folded", async () => {
    const workspace = todayWorkspace();
    workspace.assignments.push(
      assignmentRow("early", "now", "Report early", "2026-09-20T00:00:00Z", {
        submission: "submitted",
      }),
    );
    const mounted = await mountToday(workspace);
    const section = await assignmentsSection(mounted);
    const all = await required(
      () =>
        [...section.querySelectorAll<HTMLButtonElement>("header button")].find(
          (button) => button.textContent?.includes("All assignments"),
        ),
      "the All assignments link",
    );
    await click(all);
    await until(() => expect(window.location.hash).toContain(paths.lectures()));
    const rows = await required(() => {
      const found = lectureRows(mounted.host);
      return found.length >= 4 ? found : undefined;
    }, "the quarter's assignment record");

    expect(rows.map((row) => row.querySelector("b")?.textContent)).toEqual([
      "Report upcoming",
      "Report handed-in",
      "Report missed",
      "Report closed",
    ]);
    const list = mounted.host.querySelector(".lectureList")!;
    expect(list.textContent).toContain("This week · 2");
    expect(list.textContent).toContain("Past deadline · 2");
    expect(rows[1].className).toContain("done");
    expect(rows[2].className).toContain("missed");
    expect(rows[3].className).toContain("closed");
    expect(rows[1].querySelector("small")?.textContent).toContain("submitted");
    // Submitted work says so on the right instead of repeating its deadline;
    // only open work counts down and only missed work shows its deadline.
    expect(
      rows.map((row) => row.querySelector(".dueChip")?.textContent),
    ).toEqual(["1d", "Submitted", "1d ago", "Submitted"]);
    expect(rows[2].querySelector(".dueChip")?.className).toContain("overdue");

    const earlier = await required(
      () =>
        [...list.querySelectorAll<HTMLButtonElement>("button")].find((button) =>
          button.textContent?.includes("1 earlier, back to the quarter start"),
        ),
      "the folded earlier work",
    );
    expect(lectureTitles(mounted.host)).not.toContain("Report early");
    await click(earlier);
    await until(() =>
      expect(lectureTitles(mounted.host)).toContain("Report early"),
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
    // Core marks every running class "class" (`Occurrence.mark`).
    mark: "class" as const,
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

function lectureTasks(workspace: Workspace): TaskView {
  const view = taskViewFor(workspace);
  return {
    ...view,
    deadlines: {
      ...view.deadlines,
      a1: {
        at: "2026-10-08T15:00:00Z",
        units: [
          { unit: "days", count: 1 },
          { unit: "hours", count: 12 },
        ],
        state: "remaining",
        countdownTone: "soon",
        nextChangeAt: null,
      },
    },
  };
}

async function mountLectures(workspace: Workspace): Promise<MountedApp> {
  const mounted = await mountApp(
    {
      workspace,
      queries: {
        ...homeQueries,
        taskView: (_query, core) => lectureTasks(core.workspace),
        lectureSearch: (query, core) => lectureSearchFor(core.workspace, query),
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
  it("asks once about data accuracy and marks a calendar session from its right-click menu", async () => {
    const workspace = lectureWorkspace();
    delete workspace.settings.dataNoticeAcknowledgedAt;
    app = await mountApp(
      {
        workspace,
        queries: {
          ...homeQueries,
          calendarRange: () => [
            {
              date: jstDay(0),
              occurrences: [
                lectureBlock("now", jstDay(0), "00:00", "24:00", "R3"),
              ],
              importantDates: [],
              noClasses: false,
            },
          ],
          taskView: (_query, core) => lectureTasks(core.workspace),
        },
        handle: nativeNoise,
      },
      { route: paths.home() },
    );
    const notice = await required(
      () => document.querySelector<HTMLElement>('[role="dialog"]'),
      "the data notice",
    );
    expect(notice.textContent).toContain("Before you rely on this data");
    await click(
      [...notice.querySelectorAll<HTMLButtonElement>("button")].find(
        (button) => button.textContent === "I understand",
      )!,
    );
    await until(() =>
      expect(app!.core.intents).toContainEqual(
        expect.objectContaining({
          kind: "replaceSettings",
          settings: expect.objectContaining({
            dataNoticeAcknowledgedAt: expect.any(String),
          }),
        }),
      ),
    );

    const block = await required(
      () =>
        app!.host.querySelector<HTMLElement>(
          ".calendarWorkspace .ttBlock.course",
        ),
      "a class on the calendar",
    );
    await act(async () => {
      block.dispatchEvent(
        new MouseEvent("contextmenu", {
          bubbles: true,
          cancelable: true,
          clientX: 40,
          clientY: 40,
        }),
      );
    });
    const items = await required(() => {
      const found = [
        ...document.querySelectorAll<HTMLElement>('[role="menuitem"]'),
      ];
      return found.length ? found : null;
    }, "the session menu");
    expect(items.map((item) => item.textContent?.trim())).toEqual([
      "Mark as exam",
      "Mark as quiz",
      "Mark as cancelled",
    ]);
    // A running class carries no mark.
    expect(items.some((item) => item.textContent?.includes("✓"))).toBe(false);
    await click(items[2]);
    await until(() =>
      expect(app!.core.intents).toContainEqual({
        kind: "markSession",
        key: `now-${jstDay(0)}-00:00`,
        mark: "cancelled",
      }),
    );
  }, 30_000);

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

    await click(
      await required(
        () =>
          mounted.host.querySelector<HTMLButtonElement>(
            '.lectureFilter button[aria-label="Clear"]',
          ),
        "the filter's clear button",
      ),
    );
    expect(filter.value).toBe("");
    expect(lectureTitles(mounted.host)).toHaveLength(3);
  }, 20_000);

  it("switches to the quarter's assignments and opens the chosen one", async () => {
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
    expect(work[0].querySelector("small")?.textContent).toBe(
      "Course later · open",
    );
    expect(work[0].querySelector(".dueChip")?.textContent).toBe("1d 12h");

    await click(work[0]);
    await until(() =>
      expect(window.location.hash).toContain(paths.lectures("later", "a1")),
    );
  }, 20_000);

  it("keeps the assignments when the quarter changes in Assignments mode", async () => {
    // Regression: the task view answers for the new quarter at once while that
    // quarter's courses are still loading, and the list read a missing course.
    const mounted = await mountApp(
      {
        workspace: lectureWorkspace(),
        queries: {
          ...homeQueries,
          lectureList: (_query, core) => ({
            ...lectureListFor(core.workspace),
            allTerms: [TERM, "2026-4Q"],
          }),
          taskView: (_query, core) => lectureTasks(core.workspace),
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
    await until(() =>
      expect(document.activeElement?.getAttribute("data-option-value")).toBe(
        TERM,
      ),
    );
    await act(async () => {
      document.activeElement!.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "ArrowDown",
          code: "ArrowDown",
          bubbles: true,
        }),
      );
    });
    // Radix moves option focus asynchronously; Enter belongs to the next option.
    await until(() =>
      expect(document.activeElement?.getAttribute("data-option-value")).toBe(
        "2026-4Q",
      ),
    );
    await act(async () => {
      document.activeElement!.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "Enter",
          code: "Enter",
          bubbles: true,
        }),
      );
    });
    expect(quarter.getAttribute("aria-expanded")).toBe("false");
    await until(() =>
      expect(mounted.host.textContent).toContain(
        "No assignments this quarter.",
      ),
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
        [
          ...mounted.host.querySelectorAll<HTMLButtonElement>(
            ".lectureList button",
          ),
        ].find((button) => button.textContent?.includes("Hidden assignments")),
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
