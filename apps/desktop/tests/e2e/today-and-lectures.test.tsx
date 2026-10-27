// @vitest-environment jsdom
import { act } from "react";
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
  lectureSearchFor,
  typeInto,
  until,
  workspaceFixture,
  type FakeCore,
  type Mounted,
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
import type { ExamView, AssignmentGroups } from "@app/generated/persistence";

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
 * Two local helpers stand in where the harness has no equivalent:
 *
 * - `mountCompactToday`: the shipped `Today` is never rendered with `compact`
 *   (the dev-only `?preview=lms` workbench that passed it is gone), so the Today course
 *   list is mounted as the real page over the same scripted host, inside the
 *   same shell tokens.
 * - `taskViewFor`: the harness has no `TaskView` builder, so the scripted Core
 *   derives one from the workspace the same way Core decides those sets.
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
    // Core closes submitted work and past-due work that takes no late hand-in.
    closedIds: pick("closed", "handed-in", "missed", "early"),
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
    queries: {
      scheduleView: (_query, fake) => scheduleViewFor(fake.workspace),
      taskView: (_query, fake) => taskViewFor(fake.workspace),
      lectureSearch: (query, fake) => lectureSearchFor(fake.workspace, query),
    },
    handle: nativeNoise,
  });
  standaloneCore = core;
  const { TodayBody: Today } = await import("@app/pages/today/TodayPage");
  standalone = await mount(
    <>
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
    </>,
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
      queries: {
        ...homeQueries,
        scheduleView: (_query, core) => lectureSchedule(core.workspace),
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
  it("shows upcoming exams, links a historical published exam at once and saves or resets its corrections", async () => {
    const workspace = lectureWorkspace();
    const exam: ExamView = {
      id: "published-exam",
      key: "reference:exam:2q",
      kind: "exam",
      origin: "published",
      term: "2026-2Q",
      courseId: null,
      code: "CSC.T241-02",
      title: "Published exam",
      date: "2026-08-04",
      startTime: "10:45",
      endTime: "12:25",
      periods: [3, 4],
      room: "Published room",
      color: "#c98245",
      status: "scheduled",
      autoLinked: false,
      source: {
        date: "2026-08-04",
        startTime: "10:45",
        endTime: "12:25",
        room: "Published room",
      },
      edited: false,
    };
    const historical = courseRow("history", "Historical course", {
      termId: "2026-2Q",
      code: "CSC.T241",
    });
    workspace.courses.push(historical);
    const bound = {
      ...exam,
      id: "existing-exam",
      key: "reference:existing",
      courseId: historical.id,
      title: "Existing linked exam",
      room: "My room",
      edited: true,
    };
    const weekClass: ExamView = {
      ...bound,
      id: "week-class",
      key: "reference:week-class",
      kind: "class",
      title: "Exam-week session",
      edited: false,
    };
    const upcoming = {
      ...exam,
      id: "upcoming",
      key: "reference:upcoming",
      term: TERM,
      courseId: "now",
      title: "Upcoming linked exam",
      date: jstDay(1),
    };
    app = await mountApp(
      {
        workspace,
        queries: {
          ...homeQueries,
          scheduleView: (query, core) => ({
            ...lectureSchedule(core.workspace),
            allTerms: [TERM, "2026-2Q"],
            courses:
              (query as { term: string }).term === "2026-2Q"
                ? [historical]
                : lectureSchedule(core.workspace).courses,
            exams:
              (query as { term: string }).term === "2026-2Q"
                ? [exam, bound, weekClass]
                : [],
            upcomingExams: [upcoming],
          }),
          examAssociation: () => ({ conflicts: [exam] }),
          // Core owns search; this script matches the exam titles above.
          lectureSearch: (query) => ({
            courseIds: [],
            assignmentIds: [],
            examKeys: [exam, bound]
              .filter((row) =>
                row.title
                  .toLowerCase()
                  .includes((query as { query: string }).query.toLowerCase()),
              )
              .map((row) => row.key),
          }),
          taskView: (_query, core) => lectureTasks(core.workspace),
        },
        handle: nativeNoise,
      },
      { route: paths.home() },
    );
    await required(
      () => app!.host.querySelector('[aria-label="Upcoming exams"] .examCard'),
      "the upcoming exam card",
    );
    expect(
      app.host.querySelector('[aria-label="Upcoming exams"]')?.textContent,
    ).toContain("Upcoming linked exam");
    await app.go(paths.lectures());
    await readyLectureRows(app.host);
    await click(
      [
        ...app.host.querySelectorAll<HTMLButtonElement>(".lectureMode button"),
      ].find((row) => row.textContent === "Exams")!,
    );
    await chooseOption(
      app.host.querySelector<HTMLInputElement>(
        '.lectureTerm [role="combobox"]',
      )!,
      "2026-2Q",
    );
    await required(
      () =>
        [...app!.host.querySelectorAll<HTMLElement>(".lectureRow.exam")].find(
          (row) => row.textContent?.includes("Existing linked exam"),
        ),
      "the automatically linked exam",
    );
    // A class the timetable lists in exam week has its own group after the exams.
    const listed = [
      ...app.host.querySelectorAll<HTMLElement>(
        ".lectureList .lectureExamScope small, .lectureRow.exam b",
      ),
    ].map((node) => node.textContent);
    expect(listed).toEqual([
      "My exams",
      "Existing linked exam",
      "Classes in exam week",
      "Exam-week session",
    ]);
    expect(
      app.host.querySelector(".lectureRow.examWeekClass")?.textContent,
    ).toContain("Exam-week session");
    expect(app.host.querySelector(".lectureList")?.textContent).not.toContain(
      "Published exam",
    );
    await typeInto(
      app.host.querySelector<HTMLInputElement>(
        ".lectureFilter input, input.lectureFilter",
      )!,
      "Published exam",
    );
    const card = await required(
      () => app!.host.querySelector<HTMLButtonElement>(".lectureRow.exam"),
      "the searched published exam",
    );
    expect(card.textContent).toContain("Not linked");
    await click(card);
    const editor = await required(
      () =>
        app!.host.querySelector<HTMLElement>(".lectureExamDetails .examEditor"),
      "the right-hand exam editor",
    );
    await chooseOption(
      editor.querySelector<HTMLInputElement>('[role="combobox"]')!,
      "history",
    );
    // The link applies on its own; it carries no schedule fields.
    await until(() =>
      expect(app!.core.intents).toContainEqual({
        kind: "linkExam",
        examKey: "reference:exam:2q",
        courseId: "history",
      }),
    );
    await typeInto(
      editor.querySelector<HTMLInputElement>('input[name="room"]')!,
      "Personal room",
    );
    await click(
      [...editor.querySelectorAll<HTMLButtonElement>("button")].find(
        (row) => row.textContent === "Save",
      )!,
    );
    await until(() =>
      expect(app!.core.intents).toContainEqual(
        expect.objectContaining({
          kind: "editExam",
          examKey: "reference:exam:2q",
          date: "2026-08-04",
          room: "Personal room",
        }),
      ),
    );
    expect(document.querySelector('[role="dialog"]')).toBeNull();

    await typeInto(
      app.host.querySelector<HTMLInputElement>(
        ".lectureFilter input, input.lectureFilter",
      )!,
      "Existing linked exam",
    );
    await click(
      await required(
        () =>
          [
            ...app!.host.querySelectorAll<HTMLButtonElement>(
              ".lectureRow.exam",
            ),
          ].find((row) => row.textContent?.includes("Existing linked exam")),
        "the edited exam",
      ),
    );
    const edited = await required(
      () =>
        app!.host.querySelector<HTMLElement>(".lectureExamDetails .examEditor"),
      "the edited exam's editor",
    );
    await until(() => {
      expect(edited.textContent).toContain("Source: Published room");
      expect(edited.textContent).toContain(
        "This course also has: Published exam · 2026-08-04",
      );
    });
    await click(
      [...edited.querySelectorAll<HTMLButtonElement>("button")].find(
        (row) => row.textContent === "Reset to source",
      )!,
    );
    await until(() =>
      expect(app!.core.intents).toContainEqual({
        kind: "editExam",
        examKey: "reference:existing",
      }),
    );
    // Delete unlinks the exam; the notice's Undo links its course again.
    const linked = app.core.intents.length;
    await click(
      [...edited.querySelectorAll<HTMLButtonElement>("button")].find(
        (row) => row.textContent === "Delete",
      )!,
    );
    const undo = await required(
      () =>
        [...document.querySelectorAll<HTMLButtonElement>("button")].find(
          (row) => row.textContent === "Undo",
        ),
      "the notice's Undo",
    );
    await click(undo);
    await until(() =>
      expect(app!.core.intents.slice(linked)).toEqual([
        { kind: "removeExam", examKey: "reference:existing" },
        expect.objectContaining({
          kind: "linkExam",
          examKey: "reference:existing",
        }),
      ]),
    );
  }, 30_000);

  it("asks once about data accuracy and marks a calendar session from its right-click menu", async () => {
    const workspace = lectureWorkspace();
    delete workspace.settings.dataNoticeAcknowledgedAt;
    app = await mountApp(
      {
        workspace,
        queries: {
          ...homeQueries,
          scheduleView: (_query, core) => lectureSchedule(core.workspace),
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
    expect(
      items.map((item) => item.textContent?.trim()),
    ).toEqual(["Mark as exam", "Mark as quiz", "Mark as cancelled"]);
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

  it("lists lectures in class order with the Now chip and each lecture's next deadline", async () => {
    const mounted = await mountLectures(lectureWorkspace());
    const rows = await readyLectureRows(mounted.host);

    expect(rows.map((row) => row.querySelector("b")?.textContent)).toEqual([
      "Course now",
      "Course later",
      "Course friday",
    ]);
    // The course code leads the meeting details.
    expect(rows[0].querySelector("small")?.textContent).toBe("NOW · Now · R3");
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
          scheduleView: (_query, core) => ({
            ...lectureSchedule(core.workspace),
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
