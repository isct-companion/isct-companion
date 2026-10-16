// @vitest-environment jsdom
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  buttonByText,
  click,
  courseFor,
  mountApp,
  offering,
  overviewFor,
  planItemFor,
  planningWeekFor,
  readRepoFile,
  scheduleViewFor,
  textOf,
  typeInto,
  workspaceFixture,
  type FakeCore,
  type MountedApp,
} from "@tests/harness";
import { paths } from "@app/app/routes";
import type {
  Condition,
  Factor,
  FinderAction,
  FinderCourse,
  FinderView,
} from "@app/generated/persistence";
import type { Course, TermId, Workspace } from "@app/lib/model";
import type { WorkspaceIntent } from "@app/lib/workspace-intents";

/**
 * The course planner over the real shell and a scripted Core.
 *
 * The legacy suites mounted `Courses`, `CourseFinder`, `CourseEditor` and
 * `YearQuarterPicker` directly and replaced `core-query` / `finder-query` with
 * `vi.mock`. Here every page is reached through the shipped routes, and the
 * harness answers the real query path with `queries` / `finderHandler`, so a
 * passing test says something about the app rather than about a mock.
 *
 * Local helpers below exist because the harness has no finder-view builder:
 * `finderView`, `factorFor`, `chooseOption`, `timetableTitles`, `planningWeekByDay`.
 */

const FALLBACK = "2026-3Q" as TermId;

/* ------------------------------------------------------------------ *
 * A scripted finder_view, the way Core would build one from conditions.
 * ------------------------------------------------------------------ */

type FinderOptions = {
  courses: FinderCourse[];
  /** Rows the finder only returns once unavailable courses are asked for. */
  widened?: FinderCourse[];
  clashes?: FinderCourse[];
  eligibility?: Record<string, { status: string; reason: string }>;
  ineligibleIds?: string[];
  replacements?: Record<string, string[]>;
  recommended?: Record<string, { group: string; mark?: string | null }>;
  suggestions?: FinderView["suggestions"];
};

function factorFor(key: string): Factor {
  const condition: Condition = {
    type: "time",
    key,
    label: key,
    values: [],
  };
  return { type: "time", alts: [condition] };
}

/** Mirrors the one thing Core decides for `toggleTime`: append or replace. */
function keysAfterAction(
  keys: string[],
  action: FinderAction | null | undefined,
) {
  if (action?.kind !== "toggleTime") return keys;
  return action.append ? [...new Set([...keys, action.key])] : [action.key];
}

function finderView(
  options: FinderOptions,
  request: Record<string, unknown>,
  core: FakeCore,
): FinderView {
  const keys = (request.keys as string[] | undefined) ?? [];
  const includeIneligible = Boolean(request.includeIneligible);
  const action = request.action as FinderAction | null | undefined;
  const nextKeys = keysAfterAction(keys, action);
  const courses = [
    ...options.courses,
    ...(includeIneligible ? (options.widened ?? []) : []),
  ];
  const eligibility = options.eligibility ?? {};
  const ineligibleIds =
    options.ineligibleIds ??
    Object.entries(eligibility)
      .filter(([, value]) => value.status === "ineligible")
      .map(([id]) => id);
  const credits: Record<string, number> = {};
  for (const course of courses)
    credits[course.id] =
      course.credits.lecture + course.credits.exercise + course.credits.lab;
  return {
    context: {
      currentQuarter: 3,
      recommended: options.recommended ?? {},
      departments: [],
      busy: [],
    },
    courses,
    factors: nextKeys.map(factorFor),
    duplicates: 0,
    keys: nextKeys,
    plan: { unsupported: "empty" },
    rows: courses.map((course) => ({ course, matched: [] })),
    suggestions: options.suggestions ?? [],
    live: String(request.input ?? ""),
    clashes: (options.clashes ?? []).map((course) => course.id),
    ineligibleIds,
    eligibility: eligibility as FinderView["eligibility"],
    replacements: options.replacements ?? {},
    credits,
    onTimetable: core.workspace.courses
      .filter((row) => !row.deletedAt)
      .map((row) => row.code),
    activeSlots: [],
    commitKey: null,
    pasteHandled: false,
    pastedIds: [],
    missingCodes: [],
    ineligibleCodes: [],
    detailIds: [],
  };
}

type Handle = (
  command: string,
  args: Record<string, unknown>,
  core: FakeCore,
) => unknown;

/**
 * Answers `finder_query` from `handle`, not `finder`, because only `handle`
 * receives the live fake store — needed so `onTimetable` follows a commit.
 */
const finderHandler =
  (options: FinderOptions, extra?: Handle): Handle =>
  (command, args, core) => {
    if (command === "finder_query")
      return finderView(options, args.request as Record<string, unknown>, core);
    return extra?.(command, args, core);
  };

/* ------------------------------------------------------------------ *
 * Shell helpers.
 * ------------------------------------------------------------------ */

/**
 * Waits for the shell to settle. The default `until` timeout is 5 s, which is
 * too tight for a full-shell mount on a loaded machine.
 */
const settled = <T,>(check: () => T): Promise<T> =>
  vi.waitFor(check, { timeout: 15_000 });

function required<T extends Element>(
  find: () => T | null | undefined,
  what: string,
): Promise<T> {
  return settled(() => {
    const found = find();
    if (!found) throw new Error(`${what} is not ready`);
    return found;
  });
}

function timetableTitles(host: HTMLElement): string[] {
  return [
    ...new Set(
      [...host.querySelectorAll(".ttBlock b")].map(
        (node) => node.textContent ?? "",
      ),
    ),
  ];
}

/** Turns the planner on the way a person does, then waits for the search panel. */
async function enablePlanning(app: MountedApp): Promise<HTMLInputElement> {
  const edit = await required(
    () => app.host.querySelector<HTMLInputElement>('input[role="switch"]'),
    "the Edit switch",
  );
  await click(edit);
  return required(
    () => app.host.querySelector<HTMLInputElement>("#course-finder-input"),
    "the syllabus search input",
  );
}

async function chooseOption(select: HTMLSelectElement, value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(
      HTMLSelectElement.prototype,
      "value",
    )!.set!.call(select, value);
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

async function clickSlot(app: MountedApp, column: number, append: boolean) {
  const slots = app.host
    .querySelectorAll(".ttCol")
    [column].querySelectorAll<HTMLButtonElement>(".ttSlot");
  const slot = slots[0];
  if (!slot) throw new Error(`column ${column} has no pickable slot`);
  await act(async () => {
    slot.dispatchEvent(
      new MouseEvent("click", { bubbles: true, metaKey: append }),
    );
  });
}

function pills(host: HTMLElement): string[] {
  return textOf(host, ".finderPill").map((text) => text.replace(/×$/, ""));
}

/**
 * Harness gap: `core-query` and `finder-query` keep module-level response
 * caches keyed by `[revision, request]`, and `installFakeCore` resets neither.
 * Two mounts in one file both start at the fake's default revision, so the
 * second would render the first mount's answers. A fresh revision per mount
 * (and per commit) keeps every request key unique.
 */
let revisionSeed = 0;
function freshRevision(): string {
  revisionSeed += 1;
  return `revision:planner-${revisionSeed}`;
}

/** The intent the scripted host accepted, narrowed to the kind asked for. */
function recordedIntent<K extends WorkspaceIntent["kind"]>(
  core: FakeCore,
  kind: K,
): Extract<WorkspaceIntent, { kind: K }> {
  const intent = core.intents.find((candidate) => candidate.kind === kind);
  if (!intent) throw new Error(`no ${kind} intent was recorded`);
  return intent as Extract<WorkspaceIntent, { kind: K }>;
}

/* ------------------------------------------------------------------ *
 * Fixtures.
 * ------------------------------------------------------------------ */

const TERMS: TermId[] = [
  "2025-1Q",
  "2025-2Q",
  "2025-3Q",
  "2025-4Q",
  "2026-1Q",
  "2026-2Q",
  "2026-3Q",
  "2026-4Q",
];

const eligible = offering("202602421", "CSC.T252", "Graph Algorithms");
const eligibleCourse = courseFor(eligible);
const clashing = offering("202600009", "CSC.T999", "Clashing Lecture");
const clashingCourse = courseFor(clashing);
/** Only returned once unavailable courses are asked for. */
const hidden = offering("202609999", "CSC.X000", "Only When Shown");

let app: MountedApp | undefined;

afterEach(async () => {
  await app?.unmount();
  app = undefined;
});

function plannerQueries(allTerms: TermId[] = [FALLBACK]) {
  return {
    workspaceOverview: (_query: unknown, core: FakeCore) =>
      overviewFor(core.workspace, {
        allTerms,
        coursesTerm: core.workspace.selectedTerm,
        selectedTerm: core.workspace.selectedTerm,
        currentTerm: core.workspace.currentTerm,
      }),
    planningWeek: (_query: unknown, core: FakeCore) =>
      planningWeekFor(core.workspace),
    scheduleView: (_query: unknown, core: FakeCore) =>
      scheduleViewFor(core.workspace),
  };
}

/* ------------------------------------------------------------------ *
 * Searching and placing.
 * ------------------------------------------------------------------ */

describe("the course planner", () => {
  it("shows syllabus results on the right and places an eligible course when its row Add is clicked", async () => {
    app = await mountApp(
      {
        revision: freshRevision(),
        workspace: workspaceFixture(),
        queries: plannerQueries(),
        handle: finderHandler({ courses: [eligible] }),
        apply: (intent, core) => {
          if (intent.kind === "addCatalogCourses") {
            const rows = (intent.offerings as FinderCourse[]).map((row) =>
              courseFor(row),
            );
            core.publish(
              workspaceFixture({
                courses: rows,
                planItems: rows.map((course) => planItemFor(course)),
              }),
              `${core.revision}+placed`,
            );
          }
        },
      },
      { route: paths.home() },
    );
    const { host, core } = app;
    const input = await enablePlanning(app);
    await typeInto(input, "graph");

    // The search and its results sit on the right of the timetable.
    const side = await required(
      () => host.querySelector(".coursesSide .courseFinder"),
      "the search panel",
    );
    await settled(() => expect(side.textContent).toContain("Graph Algorithms"));
    expect(host.querySelector(".coursesLeft .ttCard")).not.toBeNull();
    // The panel is the whole search: no separate search field or add button.
    expect(side.querySelector('input[type="search"]')).toBeNull();
    expect(side.textContent).not.toMatch(/＋ Course|Filters/);

    const add = await required(
      () =>
        [...host.querySelectorAll<HTMLButtonElement>(".finderAdd")].find(
          (button) => button.textContent === "Add",
        ),
      "the row Add action",
    );
    await click(add);

    await settled(() =>
      expect(timetableTitles(host)).toEqual(["Graph Algorithms"]),
    );
    const intent = recordedIntent(core, "addCatalogCourses");
    expect(intent.termId).toBe("2026-3Q");
    expect((intent.offerings as FinderCourse[]).map((row) => row.id)).toEqual([
      eligible.id,
    ]);
    await settled(() =>
      expect(host.querySelector(".finderRow")?.textContent).toContain("Added"),
    );
  }, 60_000);

  it("replaces the time condition on a plain slot click and appends with the meta key", async () => {
    app = await mountApp(
      {
        revision: freshRevision(),
        workspace: workspaceFixture(),
        queries: plannerQueries(),
        handle: finderHandler({ courses: [eligible] }),
      },
      { route: paths.home() },
    );
    const { host } = app;
    await enablePlanning(app);

    // Weekdays only until the weekend is asked for, and Sunday never has a
    // pickable slot because classes run Monday to Saturday.
    expect(host.querySelectorAll(".ttCol")).toHaveLength(5);
    await click(
      await required(
        () => host.querySelector(".weekLength"),
        "the week-length toggle",
      ),
    );
    const columns = host.querySelectorAll(".ttCol");
    expect(columns).toHaveLength(7);
    expect(columns[6].querySelector(".ttSlot")).toBeNull();
    expect(columns[5].querySelector(".ttSlot")).not.toBeNull();

    // A plain slot click replaces the time condition; ⌘/Ctrl appends one.
    await clickSlot(app, 1, false);
    await settled(() => expect(pills(host)).toEqual(["Time火1-2"]));

    await clickSlot(app, 0, true);
    await settled(() =>
      expect(pills(host)).toEqual(["Time火1-2", "Time月1-2"]),
    );
  }, 60_000);

  it("opens a search result's details over the timetable", async () => {
    app = await mountApp(
      {
        revision: freshRevision(),
        workspace: workspaceFixture(),
        queries: plannerQueries(),
        handle: finderHandler({ courses: [eligible] }),
      },
      { route: paths.home() },
    );
    const { host } = app;
    const input = await enablePlanning(app);
    await typeInto(input, "graph");

    const row = await required(
      () => host.querySelector<HTMLButtonElement>(".finderMain"),
      "the search result row",
    );
    await click(row);

    const panel = await required(
      () => host.querySelector(".coursesLeft .coursesSidePanel .coursePanel"),
      "the result details panel",
    );
    expect(panel.textContent).toContain("Graph Algorithms");
    expect(panel.textContent).toContain("Syllabus");
    // It covers the timetable rather than replacing it.
    expect(host.querySelector(".coursesLeft .ttCard")).not.toBeNull();
  }, 60_000);

  it("opens a placed course's details over the search panel", async () => {
    app = await mountApp(
      {
        revision: freshRevision(),
        workspace: workspaceFixture({
          courses: [eligibleCourse],
          planItems: [planItemFor(eligibleCourse)],
        }),
        queries: plannerQueries(),
        handle: finderHandler({ courses: [eligible] }),
      },
      { route: paths.home() },
    );
    const { host } = app;
    await enablePlanning(app);

    const block = await required(
      () => host.querySelector<HTMLButtonElement>(".ttBlock.course"),
      "the placed course block",
    );
    await click(block);

    const panel = await required(
      () => host.querySelector(".coursesSide .coursesSidePanel .coursePanel"),
      "the course details panel",
    );
    expect(panel.textContent).toContain("Graph Algorithms");
    expect(panel.textContent).toContain("Remove");
    // The search stays behind the panel, and a click on a block never removes it.
    expect(host.querySelector(".coursesSide .courseFinder")).not.toBeNull();
    expect(host.querySelector(".ttBlock.course")).not.toBeNull();
  }, 60_000);
});

/* ------------------------------------------------------------------ *
 * Eligibility.
 * ------------------------------------------------------------------ */

describe("course eligibility", () => {
  it("keeps the chosen course selected and withholds Undo when the add fails", async () => {
    app = await mountApp(
      {
        revision: freshRevision(),
        workspace: workspaceFixture(),
        queries: plannerQueries(),
        handle: finderHandler({ courses: [eligible] }, (command, args) => {
          if (command === "mutate_workspace") {
            const mutation = args.mutation as { intent: { kind: string } };
            if (mutation.intent.kind === "addCatalogCourses")
              throw new Error("write failed");
          }
          return undefined;
        }),
      },
      { route: paths.home() },
    );
    const { host } = app;
    const input = await enablePlanning(app);
    await typeInto(input, "graph");

    const selected = await required(
      () => host.querySelector<HTMLInputElement>(`#finder-${eligible.id}`),
      "the row checkbox",
    );
    await click(selected);
    const addSelected = await required(
      () => host.querySelector<HTMLButtonElement>(".finderFoot .primary"),
      "the footer Add",
    );
    await click(addSelected);

    await settled(() =>
      expect(host.querySelector(".finderError")?.textContent).toContain(
        "write failed",
      ),
    );
    // The choice survives the failure, so it can be retried.
    expect(selected.checked).toBe(true);
    expect(host.querySelector(".finderFoot")?.textContent).toContain(
      "1 selected",
    );
    // Nothing was committed, so there is nothing to undo.
    expect(buttonByText(host, "Undo")).toBeUndefined();
    expect(
      host.querySelector<HTMLButtonElement>(".finderFoot .primary")?.disabled,
    ).toBe(false);
  }, 60_000);

  it("shows a first-year upper-level course but disables every way of adding it", async () => {
    const eligibility = {
      [eligible.id]: {
        status: "ineligible",
        reason: "undergraduateFirstYearUpperLevel",
      },
    };
    app = await mountApp(
      {
        revision: freshRevision(),
        workspace: workspaceFixture(),
        queries: plannerQueries(),
        handle: finderHandler({
          courses: [eligible],
          widened: [hidden],
          eligibility,
        }),
      },
      { route: paths.home() },
    );
    const { host } = app;
    const input = await enablePlanning(app);
    await typeInto(input, "graph");

    const checkbox = await required(
      () => host.querySelector<HTMLInputElement>(`#finder-${eligible.id}`),
      "the row checkbox",
    );
    const add = await required(
      () => host.querySelector<HTMLButtonElement>(".finderAdd"),
      "the row Add action",
    );
    expect(checkbox.disabled).toBe(true);
    expect(add.disabled).toBe(true);
    expect(host.textContent).toContain(
      "Not available to regular first-year undergraduate students",
    );
    expect(
      host.querySelector<HTMLButtonElement>(".finderFoot .primary")?.disabled,
    ).toBe(true);

    // Asking for unavailable courses widens the query — a course Core would
    // otherwise leave out appears — but adds nothing to the timetable.
    const show = await required(
      () => buttonByText(host, "Show unavailable courses"),
      "the Show unavailable button",
    );
    await click(show);
    await settled(() =>
      expect(buttonByText(host, "Hide unavailable courses")).toBeDefined(),
    );
    await settled(() => expect(host.textContent).toContain("Only When Shown"));
    expect(add.disabled).toBe(true);
  }, 60_000);

  it("keeps an unknown approval rule actionable and says what must be confirmed", async () => {
    const eligibility = {
      [eligible.id]: {
        status: "unknown",
        reason: "graduateCourseApprovalRequired",
      },
    };
    app = await mountApp(
      {
        revision: freshRevision(),
        workspace: workspaceFixture(),
        queries: plannerQueries(),
        handle: finderHandler({ courses: [eligible], eligibility }),
      },
      { route: paths.home() },
    );
    const { host } = app;
    const input = await enablePlanning(app);
    await typeInto(input, "graph");

    const add = await required(
      () => host.querySelector<HTMLButtonElement>(".finderAdd"),
      "the row Add action",
    );
    expect(add.disabled).toBe(false);
    await settled(() =>
      expect(host.textContent).toContain(
        "Course approval after the research proposal presentation must be confirmed",
      ),
    );
  }, 60_000);
});

/* ------------------------------------------------------------------ *
 * Clashes and replacement.
 * ------------------------------------------------------------------ */

describe("replacing a clashing course", () => {
  const before = workspaceFixture({
    courses: [eligibleCourse],
    planItems: [planItemFor(eligibleCourse)],
  });
  const after = workspaceFixture({
    courses: [clashingCourse],
    planItems: [planItemFor(clashingCourse)],
  });

  function clashApp() {
    return {
      revision: freshRevision(),
      workspace: before,
      queries: plannerQueries(),
      handle: finderHandler({
        courses: [eligible, clashing],
        clashes: [clashing],
        replacements: { [clashing.id]: [eligibleCourse.id] },
      }),
      apply: (intent: { kind: string }, core: FakeCore) => {
        if (intent.kind === "addCatalogCourses")
          core.publish(after, `${core.revision}+after`);
        else if (intent.kind === "undoCatalogCourses")
          core.publish(before, `${core.revision}+before`);
      },
    };
  }

  it("offers Replace for a clashing course, commits it, and Undo restores the previous one", async () => {
    app = await mountApp(clashApp(), { route: paths.home() });
    const { host, core } = app;
    await enablePlanning(app);

    await settled(() =>
      expect(timetableTitles(host)).toEqual(["Graph Algorithms"]),
    );
    const replace = await required(
      () =>
        [...host.querySelectorAll<HTMLButtonElement>(".finderAdd")].find(
          (button) => button.textContent === "Replace",
        ),
      "the Replace action",
    );
    expect(host.textContent).toContain("Conflict");
    await click(replace);

    await settled(() =>
      expect(timetableTitles(host)).toEqual(["Clashing Lecture"]),
    );
    const intent = recordedIntent(core, "addCatalogCourses");
    expect(intent.termId).toBe("2026-3Q");
    expect((intent.offerings as FinderCourse[]).map((row) => row.id)).toEqual([
      clashing.id,
    ]);

    const undo = await required(() => buttonByText(host, "Undo"), "Undo");
    await click(undo);

    await settled(() =>
      expect(timetableTitles(host)).toEqual(["Graph Algorithms"]),
    );
    const undoIntent = recordedIntent(core, "undoCatalogCourses");
    expect(undoIntent.termId).toBe("2026-3Q");
    expect(undoIntent.offeringIds).toEqual([clashing.id]);
  }, 60_000);
});

/* ------------------------------------------------------------------ *
 * The course editor, reached from the lecture drawer.
 * ------------------------------------------------------------------ */

function meetingCourse(meeting: Course["meetings"][number]): Course {
  return {
    kind: "course",
    termId: FALLBACK,
    code: "CSC.T354",
    title: "Parallel programming",
    instructor: "Instructor",
    credits: 2,
    creditSplit: { lecture: 2, exercise: 0, lab: 0 },
    instructionLanguage: "en",
    color: "#355ba7",
    meetings: [meeting],
    source: { provider: "manual" },
    id: "course-1",
    rev: 1,
    updatedAt: "2026-10-15T00:00:00Z",
    updatedBy: "test",
  } as Course;
}

async function openEditorFor(course: Course): Promise<MountedApp> {
  const mounted = await mountApp(
    {
      revision: freshRevision(),
      workspace: workspaceFixture({
        courses: [course],
        planItems: [planItemFor(course)],
      }),
      queries: plannerQueries(),
      handle: finderHandler({ courses: [eligible] }),
    },
    { route: paths.drawer(paths.home(), course.id) },
  );
  await required(
    () => mounted.host.querySelector<HTMLInputElement>('input[role="switch"]'),
    "the Edit switch",
  );
  await click(
    mounted.host.querySelector<HTMLInputElement>('input[role="switch"]')!,
  );
  const drawerEdit = await required(
    () =>
      [
        ...document.querySelectorAll<HTMLButtonElement>(
          ".drawerActions button",
        ),
      ].find((button) => (button.textContent ?? "").trim() === "Edit"),
    "the drawer Edit action",
  );
  await click(drawerEdit);
  await required(
    () => document.querySelector('select[aria-label="Period"]'),
    "the course editor",
  );
  return mounted;
}

/** Renames the course and saves, returning the intent the host recorded. */
async function saveEditor(core: FakeCore, selectValue?: string) {
  const title = document.querySelector<HTMLInputElement>(
    'input[placeholder="並列プログラミング"]',
  );
  if (!title) throw new Error("the title field is not ready");
  await typeInto(title, "Renamed course");
  if (selectValue) {
    const period = document.querySelector<HTMLSelectElement>(
      'select[aria-label="Period"]',
    )!;
    await chooseOption(period, selectValue);
  }
  const save = [
    ...document.querySelectorAll<HTMLButtonElement>(".sheetActions button"),
  ].find((button) => (button.textContent ?? "").trim() === "Save");
  if (!save) throw new Error("the Save action is not ready");
  await click(save);
  return settled(() => {
    const intent = recordedIntent(core, "saveCourse") as {
      draft: {
        title: string;
        meetings: { periods: number[]; startTime: string; endTime: string }[];
      };
    };
    return intent;
  });
}

describe("the course editor", () => {
  it("keeps a cross-block period range when only the course details change", async () => {
    const course = meetingCourse({
      days: [1],
      periods: [1, 2, 3, 4],
      startTime: "08:50",
      endTime: "12:25",
      room: "W9-201",
    });
    app = await openEditorFor(course);

    const period = document.querySelector<HTMLSelectElement>(
      'select[aria-label="Period"]',
    )!;
    expect(period.value).toBe("periods:1,2,3,4");
    expect(period.selectedOptions[0].textContent).toContain(
      "1-4 · 08:50–12:25",
    );

    const intent = await saveEditor(app.core);
    expect(intent.draft.title).toBe("Renamed course");
    expect(intent.draft.meetings[0].periods).toEqual([1, 2, 3, 4]);
  }, 60_000);

  it("keeps an explicit custom time as a custom meeting", async () => {
    const course = meetingCourse({
      days: [1],
      periods: [],
      startTime: "09:10",
      endTime: "10:20",
      room: "W9-201",
    });
    app = await openEditorFor(course);

    const period = document.querySelector<HTMLSelectElement>(
      'select[aria-label="Period"]',
    )!;
    expect(period.value).toBe("custom");

    const intent = await saveEditor(app.core);
    expect(intent.draft.meetings[0]).toMatchObject({
      periods: [],
      startTime: "09:10",
      endTime: "10:20",
    });
  }, 60_000);

  it("keeps a normal paired teaching slot period-backed", async () => {
    const course = meetingCourse({
      days: [1],
      periods: [1, 2],
      startTime: "08:50",
      endTime: "10:30",
      room: "W9-201",
    });
    app = await openEditorFor(course);

    const period = document.querySelector<HTMLSelectElement>(
      'select[aria-label="Period"]',
    )!;
    expect(period.value).toBe("1-2");

    const intent = await saveEditor(app.core);
    expect(intent.draft.meetings[0].periods).toEqual([1, 2]);
  }, 60_000);

  it("clears the periods only when Custom time is chosen", async () => {
    const course = meetingCourse({
      days: [1],
      periods: [1, 2, 3, 4],
      startTime: "08:50",
      endTime: "12:25",
      room: "W9-201",
    });
    app = await openEditorFor(course);

    const intent = await saveEditor(app.core, "custom");
    expect(intent.draft.meetings[0].periods).toEqual([]);
    expect(intent.draft.meetings[0].startTime).toBe("08:50");
  }, 60_000);
});

/* ------------------------------------------------------------------ *
 * The year and quarter picker.
 * ------------------------------------------------------------------ */

describe("the year and quarter picker", () => {
  it("keeps one year open and changes the term only when a quarter is clicked", async () => {
    const workspace = workspaceFixture();
    workspace.terms = TERMS;
    workspace.selectedTerm = "2026-3Q";
    workspace.currentTerm = "2026-3Q";
    app = await mountApp(
      {
        revision: freshRevision(),
        workspace,
        queries: plannerQueries(TERMS),
        handle: finderHandler({ courses: [eligible] }),
        apply: (intent, core) => {
          if (intent.kind === "selectTerm")
            core.publish(
              { ...core.workspace, selectedTerm: intent.termId as TermId },
              `${core.revision}+term`,
            );
        },
      },
      { route: paths.home() },
    );
    const { host, core } = app;

    const expanded = await required(
      () => host.querySelector(".yearQuarterGroup.expanded .yearQuarterYear"),
      "the expanded year",
    );
    expect(expanded.textContent).toBe("2026");

    // Hovering a year opens it but must not select a quarter.
    const past = host.querySelectorAll<HTMLElement>(".yearQuarterGroup")[0];
    const hover = new Event("pointerover", { bubbles: true });
    Object.defineProperty(hover, "pointerType", { value: "mouse" });
    await act(async () => past.dispatchEvent(hover));

    expect(host.querySelectorAll(".yearQuarterGroup.expanded")).toHaveLength(1);
    expect(
      host.querySelector(".yearQuarterGroup.expanded .yearQuarterYear")
        ?.textContent,
    ).toBe("2025");
    expect(
      host
        .querySelector('[aria-label="2026 3Q"]')
        ?.getAttribute("aria-selected"),
    ).toBe("true");
    expect(
      core.intents.filter((candidate) => candidate.kind === "selectTerm"),
    ).toHaveLength(0);

    const quarter = await required(
      () => host.querySelector<HTMLButtonElement>('[aria-label="2025 2Q"]'),
      "the 2025 2Q quarter",
    );
    await click(quarter);

    await settled(() =>
      expect(
        host
          .querySelector('[aria-label="2025 2Q"]')
          ?.getAttribute("aria-selected"),
      ).toBe("true"),
    );
    const intent = recordedIntent(core, "selectTerm");
    expect(intent.termId).toBe("2025-2Q");
    expect(
      host.querySelector(".yearQuarterGroup.expanded .yearQuarterYear")
        ?.textContent,
    ).toBe("2025");
  }, 60_000);
});

/* ------------------------------------------------------------------ *
 * The committed school fixtures the planner reads.
 * ------------------------------------------------------------------ */

type PublicCourse = {
  externalId: string;
  year: number;
  quarter: "1Q" | "2Q" | "3Q" | "4Q";
  code: string;
  title: string;
  instructor: string;
  credits: number;
  language: string;
  meetings: Course["meetings"];
  url: string;
  content: { fetchedAt: string; sourceUrl: string };
};
type OfficialDepartments = {
  admissionYear: number;
  source: { url: string }[];
  departments: Array<{
    id: string;
    name: { ja: string; en: string };
    syllabusScope: string;
    unitCode: string;
    guidePdf: string;
    recommended: Array<{
      category: { ja: string; en?: string };
      level: number;
      code: string;
      group: string;
      mark?: "required" | "recommended";
      title: { ja: string; en?: string };
      credits: { lecture: number; exercise: number; lab: number };
    }>;
  }>;
};

const parsedCourses = () =>
  JSON.parse(
    readRepoFile(
      "crates/isct-core/tests/fixtures/syllabus/course-examples-parsed.json",
    ),
  ) as PublicCourse[];
const departments = (year: 2025 | 2026) =>
  JSON.parse(
    readRepoFile(
      `crates/isct-core/tests/fixtures/subscriptions/data/${year}/departments.json`,
    ),
  ) as OfficialDepartments;

/** Core returns one block per meeting day; the harness fixture only uses day 2. */
function planningWeekByDay(workspace: Workspace): Record<string, unknown> {
  const active = workspace.courses.filter((row) => !row.deletedAt);
  return {
    courses: active,
    conflictCount: 0,
    days: Array.from({ length: 7 }, (_, index) => ({
      day: index + 1,
      blocks: active.flatMap((row) =>
        row.meetings
          .filter((meeting) => meeting.days.includes(index + 1))
          .map((meeting, slot) => ({
            key: `${row.id}-${index}-${slot}`,
            id: row.id,
            courseId: row.id,
            kind: "course",
            date: "",
            title: row.title,
            subtitle: row.code,
            room: meeting.room ?? "",
            startTime: meeting.startTime ?? "",
            endTime: meeting.endTime ?? "",
            color: row.color,
          })),
      ),
    })),
  };
}

describe("the committed school fixtures", () => {
  it("pin the 2026 and 2025 admission-year guide data behind the planner", () => {
    const source = parsedCourses().find(
      (course) => course.code === "CSC.T354",
    )!;
    expect(source.year).toBe(2026);
    expect(source.content.sourceUrl).toBe(source.url);

    const cs2026 = departments(2026).departments.find(
      (item) => item.id === "cs",
    )!;
    expect(departments(2026).admissionYear).toBe(2026);
    expect(departments(2026).source[0].url).toContain("/guide/guide_2026/");
    expect(cs2026.guidePdf).toContain("/guide/guide_2026/");
    expect(
      cs2026.recommended.find((item) => item.code === "CSC.T354")!.group,
    ).toBe("B");

    const cs2025 = departments(2025).departments.find(
      (item) => item.id === "cs",
    )!;
    expect(departments(2025).admissionYear).toBe(2025);
    expect(departments(2025).source).toEqual([
      expect.objectContaining({
        url: "https://www.titech.ac.jp/guide/guide_2025/gakubu1/",
      }),
    ]);
    expect(cs2025.guidePdf).toBe(
      "https://www.titech.ac.jp/guide/guide_2025/gakubu1/pdf/26-13.pdf",
    );
  });

  it("renders the committed CSC.T354 offering with its real meetings and guide mark", async () => {
    const source = parsedCourses().find(
      (course) => course.code === "CSC.T354",
    )!;
    const recommendation = departments(2026)
      .departments.find((item) => item.id === "cs")!
      .recommended.find((item) => item.code === "CSC.T354")!;

    const workspace = workspaceFixture();
    workspace.terms = ["2026-2Q"] as TermId[];
    workspace.selectedTerm = "2026-2Q";
    workspace.currentTerm = "2026-2Q";
    workspace.academicProfile.admissionYear = 2026;
    workspace.settings.departmentId = "cs";
    const course = {
      id: `syllabus:${source.externalId}`,
      rev: 1,
      updatedAt: source.content.fetchedAt,
      updatedBy: "public-syllabus-fixture",
      kind: "course",
      termId: "2026-2Q",
      code: source.code,
      title: source.title,
      instructor: source.instructor,
      credits: source.credits,
      creditSplit: { lecture: source.credits, exercise: 0, lab: 0 },
      instructionLanguage: source.language === "Japanese" ? "ja" : "en",
      color: "#52657a",
      meetings: source.meetings,
      url: source.url,
      source: {
        provider: "syllabus",
        externalId: source.externalId,
        fetchedAt: source.content.fetchedAt,
      },
    } as Course;
    workspace.courses = [course];
    workspace.planItems = [planItemFor(course)];

    const real: FinderCourse = {
      id: source.externalId,
      code: source.code,
      year: source.year,
      quarters: [source.quarter],
      title: { ja: source.title, en: source.title },
      instructors: [{ ja: source.instructor, en: source.instructor }],
      credits: { lecture: source.credits, exercise: 0, lab: 0 },
      instructionLanguage: source.language === "Japanese" ? "ja" : "en",
      format: { kinds: ["lecture"] },
      schedule: source.meetings.map((meeting) => ({
        kind: "weekly",
        day: meeting.days[0],
        periods: [meeting.periods[0], meeting.periods.at(-1)!] as [
          number,
          number,
        ],
      })),
      syllabusUrl: source.url,
    };

    app = await mountApp(
      {
        revision: freshRevision(),
        workspace,
        queries: {
          ...plannerQueries(["2026-2Q"]),
          planningWeek: (_query: unknown, core: FakeCore) =>
            planningWeekByDay(core.workspace),
        },
        handle: finderHandler({
          courses: [real],
          recommended: {
            [recommendation.code]: {
              group: recommendation.group,
              mark: recommendation.mark,
            },
          },
        }),
      },
      { route: paths.home() },
    );
    const { host } = app;
    const input = await enablePlanning(app);
    await typeInto(input, "parallel");

    await settled(() =>
      expect(timetableTitles(host)).toContain("Parallel Programming"),
    );
    const blocks = [...host.querySelectorAll<HTMLElement>(".ttBlock.course")];
    const titles = blocks.map((block) => block.getAttribute("title") ?? "");
    expect(titles.some((title) => title.includes("W9-323"))).toBe(true);
    expect(
      titles.filter((title) => title.includes("10:45–12:25")),
    ).toHaveLength(2);
    await settled(() =>
      expect(host.querySelector(".finderRow")?.textContent).toContain("B ○"),
    );
  }, 60_000);
});
