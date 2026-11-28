// @vitest-environment jsdom
import { act } from "react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";

import {
  DEVICE,
  TERM,
  click,
  chooseOption,
  mountApp,
  overviewFor,
  planningWeekFor,
  lectureListFor,
  lectureSearchFor,
  until,
  workspaceFixture,
  type FakeCore,
  type MountedApp,
} from "@tests/harness";
import { paths } from "@app/app/routes";
import type {
  Assignment,
  Course,
  TaskView,
  TermId,
  Workspace,
} from "@app/lib/model";
import type { AssignmentGroups } from "@app/generated/persistence";

/**
 * Today and the lecture desk, driven through the real shell over a scripted
 * Core. Each test walks a path and checks where it ends: the route, or the
 * intents the shell committed. Which assignments Core groups where is Core's
 * task view (`crates/app-core/src/tests/schedule.rs`); how the rows look is
 * the manual check DESKTOP-TODAY-LECTURE-ASSIGNMENTS.
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
  const overdue = pick("closed", "missed");
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
        pendingIds: pick("upcoming"),
        today: groups([], pick("upcoming", "a1"), pick("missed")),
        record: groups(
          [],
          pick("upcoming", "handed-in", "a1"),
          pick("missed", "closed"),
          { hiddenIds: pick("a2") },
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
 * Today's assignments.
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

async function assignmentsSection(mounted: MountedApp): Promise<HTMLElement> {
  return required(
    () =>
      mounted.host.querySelector<HTMLElement>(
        '[data-today-section="assignments"]',
      ),
    "the Today assignments section",
  );
}

describe("Today's assignments", () => {
  it("opens the quarter's full record on Lecture Desk", async () => {
    const mounted = await mountToday(todayWorkspace());
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

async function mountLectures(workspace: Workspace): Promise<MountedApp> {
  const mounted = await mountApp(
    {
      workspace,
      queries: {
        ...homeQueries,
        taskView: (_query, core) => taskViewFor(core.workspace),
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
          taskView: (_query, core) => taskViewFor(core.workspace),
        },
        handle: nativeNoise,
      },
      { route: paths.home() },
    );
    const notice = await required(
      () => document.querySelector<HTMLElement>('[role="dialog"]'),
      "the data notice",
    );
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
    const cancel = await required(
      () =>
        [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find(
          (item) => item.textContent?.includes("Mark as canceled"),
        ),
      "the session menu's cancel item",
    );
    await click(cancel);
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
    expect(mounted.core.intents).toEqual([]);
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
          taskView: (_query, core) => taskViewFor(core.workspace),
        },
        handle: nativeNoise,
      },
      { route: paths.lectures() },
    );
    app = mounted;
    await readyLectureRows(mounted.host);
    await click(await assignmentModeButton(mounted.host));
    const work = () =>
      mounted.host.querySelectorAll(".lectureList button.lectureRow").length;
    await until(() => expect(work()).toBeGreaterThan(0));
    const quarter = await required(
      () =>
        mounted.host.querySelector<HTMLInputElement>(
          '.lectureTerm [role="combobox"]',
        ),
      "the quarter picker",
    );

    await chooseOption(quarter, "2026-4Q");
    await until(() => expect(work()).toBe(0));
    expect(mounted.host.querySelector(".pageError")).toBeNull();
    await chooseOption(quarter, TERM);
    await until(() => expect(work()).toBeGreaterThan(0));
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
    await until(() =>
      expect(
        mounted.host.querySelectorAll(".lectureList button.lectureRow").length,
      ).toBeGreaterThan(0),
    );
    expect(mounted.host.querySelector(".lectureRow.hidden")).toBeNull();

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
