// @vitest-environment jsdom
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  buttonByText,
  click,
  chooseOption,
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
  until,
  watchConsoleErrors,
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
  FinderPage,
  FinderRowView,
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
 * Local helpers below exist because the harness has no finder-page builder:
 * `finderPage`, `factorFor`, `chooseOption`, `timetableTitles`, `planningWeekByDay`.
 */

const FALLBACK = "2026-3Q" as TermId;

/* ------------------------------------------------------------------ *
 * A scripted finder_query page, the way Core would build one.
 * ------------------------------------------------------------------ */

type FinderOptions = {
  courses: FinderCourse[];
  /** Rows the finder only returns once unavailable courses are asked for. */
  widened?: FinderCourse[];
  clashes?: FinderCourse[];
  eligibility?: Record<string, { status: string; reason: string }>;
  replacements?: Record<string, string[]>;
  recommended?: Record<string, { group: string; mark?: string }>;
  suggestions?: FinderPage["suggestions"];
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

/** Mirrors the one thing Core decides for an action: the keys that follow. */
function keysAfterAction(
  keys: string[],
  action: FinderAction | null | undefined,
) {
  if (action?.kind === "commit") return [...new Set([...keys, action.key])];
  if (action?.kind !== "toggleTime") return keys;
  return action.append ? [...new Set([...keys, action.key])] : [action.key];
}

function finderPage(
  options: FinderOptions,
  request: Record<string, unknown>,
  core: FakeCore,
): FinderPage {
  const keys = (request.keys as string[] | undefined) ?? [];
  const includeIneligible = Boolean(request.includeIneligible);
  const action = request.action as FinderAction | null | undefined;
  const nextKeys = keysAfterAction(keys, action);
  const courses = [
    ...options.courses,
    ...(includeIneligible ? (options.widened ?? []) : []),
  ];
  const eligibility = options.eligibility ?? {};
  const placedCodes = core.workspace.courses
    .filter((row) => !row.deletedAt)
    .map((row) => row.code);
  const row = (course: FinderCourse): FinderRowView => {
    const verdict = eligibility[course.id];
    return {
      course,
      matched: [],
      placed: placedCodes.includes(course.code),
      clash: (options.clashes ?? []).some((clash) => clash.id === course.id),
      replaces: options.replacements?.[course.id] ?? [],
      credits:
        course.credits.lecture + course.credits.exercise + course.credits.lab,
      ...(verdict && verdict.status !== "allowed"
        ? { eligibility: verdict as FinderRowView["eligibility"] }
        : {}),
      ...(options.recommended?.[course.code]
        ? { recommended: options.recommended[course.code] }
        : {}),
    };
  };
  const offset = Number(request.offset ?? 0);
  const limit = Number(request.limit ?? 100);
  return {
    context: {
      currentQuarter: 3,
      recommended: options.recommended ?? {},
      departments: [],
      busy: [],
      officialOptions: [],
    },
    factors: nextKeys.map(factorFor),
    duplicates: 0,
    keys: nextKeys,
    live: String(request.input ?? ""),
    suggestions: options.suggestions ?? [],
    total: courses.length,
    offset,
    rows: courses.slice(offset, offset + limit).map(row),
    stored: options.courses.length + (options.widened?.length ?? 0),
    ineligible: Object.values(eligibility).filter(
      (verdict) => verdict.status === "ineligible",
    ).length,
    // Core highlights the slots of the time conditions.
    activeSlots: nextKeys.flatMap((key) => {
      const time = /^([月火水木金土])(\d+)-(\d+)$/.exec(key);
      return time
        ? [`${"月火水木金土".indexOf(time[1]) + 1}:${time[2]}-${time[3]}`]
        : [];
    }),
    placedCodes,
    commitKey: null,
    pasteHandled: false,
    pasted: [],
    missingCodes: [],
    ineligibleCodes: [],
    catalogVersion: 1,
    contentRevision: core.revision,
  };
}

type Handle = (
  command: string,
  args: Record<string, unknown>,
  core: FakeCore,
) => unknown;

/**
 * Answers `finder_query` from `handle`, not `finder`, because only `handle`
 * receives the live fake store — needed so `placed` follows a commit.
 */
const finderHandler =
  (options: FinderOptions, extra?: Handle): Handle =>
  (command, args, core) => {
    if (command === "finder_query")
      return finderPage(options, args.request as Record<string, unknown>, core);
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

/** The workstation timetable opens as the planner; waits for its search panel. */
async function enablePlanning(
  app: MountedApp,
  focus = true,
): Promise<HTMLInputElement> {
  const input = await required(
    () => app.host.querySelector<HTMLInputElement>("#course-finder-input"),
    "the course search input",
  );
  if (focus) await act(async () => input.focus());
  return input;
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
    workspaceOverview: (query: unknown, core: FakeCore) =>
      overviewFor(core.workspace, {
        term: (query as { term: string }).term || core.workspace.selectedTerm,
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
  it("navigates filter categories and conditions while keeping IME confirmation out of search", async () => {
    const options: FinderOptions = {
      courses: [eligible],
      suggestions: [
        { kind: "text", key: "teacher:Tanaka", count: 1 },
        { kind: "text", key: "room:W9", count: 1 },
      ],
    };
    app = await mountApp(
      {
        revision: freshRevision(),
        workspace: workspaceFixture(),
        queries: plannerQueries(),
        handle: finderHandler(options),
      },
      { route: paths.timetable() },
    );
    const input = await enablePlanning(app);
    await typeInto(input, "teacher:");
    await settled(() =>
      expect(app!.host.querySelectorAll('[role="option"]')).toHaveLength(2),
    );
    const key = async (value: string, composing = false) =>
      act(async () => {
        input.dispatchEvent(
          new KeyboardEvent("keydown", {
            key: value,
            bubbles: true,
            isComposing: composing,
          }),
        );
      });
    await key("Enter", true);
    expect(
      app.core.calls.filter(
        (c) =>
          c.command === "finder_query" &&
          (c.args.request as { action?: unknown }).action,
      ),
    ).toHaveLength(0);
    await key("Escape");
    expect(app.host.querySelectorAll('[role="option"]')).toHaveLength(0);
    await key("ArrowDown");
    await key("ArrowDown");
    await key("Tab");
    await settled(() =>
      expect(
        app!.core.calls.some(
          (c) =>
            c.command === "finder_query" &&
            (c.args.request as { action?: FinderAction }).action?.kind ===
              "commit",
        ),
      ).toBe(true),
    );
    const commits = app.core.calls.filter(
      (c) =>
        c.command === "finder_query" &&
        (c.args.request as { action?: FinderAction }).action?.kind === "commit",
    );
    expect(
      (commits[0].args.request as { action: FinderAction }).action,
    ).toEqual({ kind: "commit", key: "room:W9" });
    options.suggestions = [
      { kind: "category", key: "level:", label: "Course level" },
    ];
    await typeInto(input, "level");
    await settled(() =>
      expect(app!.host.querySelector('[role="option"]')?.textContent).toContain(
        "Course level",
      ),
    );
    await key("Enter");
    expect(input.value).toBe("level:");
    expect(
      app.core.calls.filter(
        (call) =>
          call.command === "finder_query" &&
          (call.args.request as { action?: FinderAction }).action?.kind ===
            "commit",
      ),
    ).toHaveLength(1);
    options.suggestions = [
      {
        kind: "condition",
        key: "kyomu:graduate_level:live-600",
        condition: {
          type: "official-level",
          key: "kyomu:graduate_level:live-600",
          label: "600",
          values: [{ id: "graduate_level", value: "live-600" }],
        },
        count: -1,
      },
    ];
    await typeInto(input, "level:6");
    await settled(() =>
      expect(app!.host.querySelector('[role="option"]')?.textContent).toContain(
        "600",
      ),
    );
    await key("Tab");
    await settled(() =>
      expect(
        app!.core.calls
          .filter(
            (call) =>
              call.command === "finder_query" &&
              (call.args.request as { action?: FinderAction }).action?.kind ===
                "commit",
          )
          .at(-1)?.args.request,
      ).toMatchObject({
        action: { kind: "commit", key: "kyomu:graduate_level:live-600" },
      }),
    );
  });

  it("commits at once, shows the condition until Core answers, and searches a composition only when it ends", async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const options: FinderOptions = {
      courses: [eligible],
      suggestions: [{ kind: "text", key: "graph", count: -1 }],
    };
    const answer = finderHandler(options);
    app = await mountApp(
      {
        revision: freshRevision(),
        workspace: workspaceFixture(),
        queries: plannerQueries(),
        handle: (command, args, core) =>
          command === "finder_query" &&
          (args.request as { action?: FinderAction }).action?.kind === "commit"
            ? gate.then(() => answer(command, args, core))
            : answer(command, args, core),
      },
      { route: paths.timetable() },
    );
    const { host, core } = app;
    const input = await enablePlanning(app);
    await typeInto(input, "graph");
    await settled(() =>
      expect(host.querySelectorAll('[role="option"]')).toHaveLength(1),
    );
    await act(async () => {
      input.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      );
    });
    // Before Core answers: the field is empty, the list is closed and the
    // condition waits as a pill.
    expect(input.value).toBe("");
    expect(host.querySelectorAll('[role="option"]')).toHaveLength(0);
    expect(host.querySelector(".finderPill.pending")?.textContent).toBe(
      "graph",
    );
    await act(async () => {
      release();
      await gate;
    });
    await settled(() => expect(pills(host)).toEqual(["Timegraph"]));
    expect(host.querySelector(".finderPill.pending")).toBeNull();

    // An IME composition is shown but not searched until it ends; the input
    // event a webview sends after the end does not search it again.
    const searches = () => core.commands("finder_query").length;
    const before = searches();
    await act(async () => {
      input.dispatchEvent(
        new CompositionEvent("compositionstart", { bubbles: true }),
      );
    });
    await typeInto(input, "ぶつ");
    await typeInto(input, "物理");
    expect(searches()).toBe(before);
    await act(async () => {
      input.dispatchEvent(
        new CompositionEvent("compositionend", { bubbles: true, data: "物理" }),
      );
    });
    await settled(() =>
      expect(core.commands("finder_query").at(-1)?.args.request).toMatchObject(
        { input: "物理", keys: ["graph"] },
      ),
    );
    expect(searches()).toBe(before + 1);
    await typeInto(input, "物理");
    expect(searches()).toBe(before + 1);
  }, 60_000);

  it("searches only the stored catalog and renders a large result in batches of mounted rows", async () => {
    const courses = Array.from({ length: 1000 }, (_, index) =>
      offering(
        String(202600000 + index),
        `CSC.T${index}`,
        `Official course ${index}`,
      ),
    );
    app = await mountApp(
      {
        revision: freshRevision(),
        workspace: workspaceFixture(),
        queries: plannerQueries(),
        handle: finderHandler({ courses }),
      },
      { route: paths.timetable() },
    );
    const searchInput = await enablePlanning(app, false);
    expect(app.host.querySelectorAll(".finderRow")).toHaveLength(0);
    expect(
      app.core.calls.filter((call) => call.command === "finder_query"),
    ).toHaveLength(0);
    await act(async () => searchInput.focus());
    // The whole stored result is counted at once; rows mount in batches.
    await settled(() => expect(app!.host.textContent).toContain("1000 courses"));
    while (buttonByText(app.host, "Load more courses")) {
      await click(buttonByText(app.host, "Load more courses")!);
      expect(app.host.querySelectorAll(".finderRow").length).toBeLessThan(25);
    }
    expect(app.host.textContent).toContain("Official course 0");
    expect(app.host.textContent).not.toContain("Official course 999");
    const scroll = app.host.querySelector<HTMLElement>(".finderResults")!;
    await act(async () => {
      scroll.scrollTo({ top: 999 * 96 });
    });
    await settled(() =>
      expect(app!.host.textContent).toContain("Official course 999"),
    );
    expect(app.host.querySelectorAll(".finderRow").length).toBeLessThan(25);
    expect(app.host.textContent).not.toContain("Load more courses");
    // Every search is the local finder query; nothing asks 教務Web.
    expect(
      app.core.calls.some((call) =>
        ["catalog_query", "cancel_pull"].includes(call.command),
      ),
    ).toBe(false);
    const queriesBeforeNavigation = app.core.calls.filter(
      (call) => call.command === "finder_query",
    ).length;
    await click(buttonByText(app.host, "Today")!);
    await click(buttonByText(app.host, "Timetable")!);
    await required(
      () => app!.host.querySelector("#course-finder-input"),
      "fresh search panel",
    );
    expect(app.host.querySelectorAll(".finderRow")).toHaveLength(0);
    expect(
      app.core.calls.filter((call) => call.command === "finder_query"),
    ).toHaveLength(queriesBeforeNavigation);
  });

  it("says the year's catalog is downloading while nothing is stored and shows each department as Core commits it", async () => {
    let stored: FinderCourse[] = [];
    app = await mountApp(
      {
        revision: freshRevision(),
        workspace: workspaceFixture(),
        queries: plannerQueries(),
        handle: (command, args, core) =>
          command === "finder_query"
            ? finderPage(
                { courses: stored },
                args.request as Record<string, unknown>,
                core,
              )
            : undefined,
      },
      { route: paths.timetable() },
    );
    const searchInput = await enablePlanning(app, false);
    await act(async () => searchInput.focus());
    const year = await settled(() => {
      const request = app!.core.calls.find(
        (call) => call.command === "finder_query",
      )?.args.request as { year: number } | undefined;
      if (!request) throw new Error("the finder's first query is not ready");
      return request.year;
    });
    await settled(() =>
      expect(app!.core.calls.map((call) => call.args.hint)).toContainEqual({
        kind: "open",
        view: { kind: "finder", year },
      }),
    );
    const snapshot = (runs: unknown[]) =>
      act(async () =>
        app!.core.emit("sync-snapshot", {
          generation: runs.length + 1,
          resetting: false,
          runs,
          sources: [],
        }),
      );
    await snapshot([
      {
        run: "catalog",
        dataset: "catalog.official",
        label: { key: "sync.dataset.catalog.official", args: {} },
        priority: "visible",
        done: 3,
        total: 40,
        current: null,
        scope: String(year),
        held: null,
        failure: null,
      },
    ]);
    await settled(() =>
      expect(app!.host.querySelector(".courseFinder")!.textContent).toContain(
        `Downloading the ${year} course catalog`,
      ),
    );
    // Core commits one department while the run goes on. Another year's
    // commit is not this finder's; the shown year's is queried at once.
    stored = [eligible];
    const queries = () =>
      app!.core.calls.filter((call) => call.command === "finder_query").length;
    const before = queries();
    await act(async () =>
      app!.core.emit("catalog-changed", { year: year + 1, version: 1 }),
    );
    expect(queries()).toBe(before);
    await act(async () =>
      app!.core.emit("catalog-changed", { year, version: 2 }),
    );
    await settled(() => {
      const finder = app!.host.querySelector(".courseFinder")!;
      expect(finder.textContent).not.toContain("Downloading");
      expect(finder.querySelectorAll(".finderRow")).toHaveLength(1);
    });
  });

  it("shows course search results on the right and places an eligible course when its row Add is clicked", async () => {
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
      { route: paths.timetable() },
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

  it("shows an added course as saving until Core commits it", async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    app = await mountApp(
      {
        revision: freshRevision(),
        workspace: workspaceFixture(),
        queries: plannerQueries(),
        handle: finderHandler({ courses: [eligible] }, (command, _args, core) =>
          command === "mutate_workspace"
            ? gate.then(() => {
                const rows = [eligibleCourse];
                core.publish(
                  workspaceFixture({
                    courses: rows,
                    planItems: rows.map((course) => planItemFor(course)),
                  }),
                  `${core.revision}+placed`,
                );
                return {
                  revision: core.revision,
                  contentRevision: core.revision,
                  workspace: structuredClone(core.workspace),
                  schoolRecords: structuredClone(core.schoolRecords),
                };
              })
            : undefined,
        ),
      },
      { route: paths.timetable() },
    );
    const { host } = app;
    const input = await enablePlanning(app);
    await typeInto(input, "graph");
    await click(
      await required(
        () => host.querySelector<HTMLButtonElement>(".finderAdd"),
        "the row Add action",
      ),
    );
    expect(host.querySelector(".finderRow .finderPlaced")?.textContent).toBe(
      "Added · saving",
    );
    await act(async () => {
      release();
      await gate;
    });
    await settled(() =>
      expect(host.querySelector(".finderRow .finderPlaced")?.textContent).toBe(
        "Added",
      ),
    );
    await settled(() =>
      expect(timetableTitles(host)).toEqual(["Graph Algorithms"]),
    );
  }, 60_000);

  it("replaces the time condition on a plain slot click and appends with the meta key", async () => {
    let release!: () => void;
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    const answer = finderHandler({ courses: [eligible] });
    app = await mountApp(
      {
        revision: freshRevision(),
        workspace: workspaceFixture(),
        queries: plannerQueries(),
        handle: (command, args, core) =>
          command === "finder_query" &&
          (args.request as { action?: FinderAction }).action?.kind ===
            "toggleTime"
            ? held.then(() => answer(command, args, core))
            : answer(command, args, core),
      },
      { route: paths.timetable() },
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
    // The slot lights up before Core answers.
    const slot = (column: number) =>
      host.querySelectorAll(".ttCol")[column].querySelector(".ttSlot");
    await clickSlot(app, 1, false);
    expect(slot(1)?.getAttribute("aria-pressed")).toBe("true");
    expect(pills(host)).toEqual([]);
    await act(async () => {
      release();
      await held;
    });
    await settled(() => expect(pills(host)).toEqual(["Time火1-2"]));
    expect(slot(1)?.getAttribute("aria-pressed")).toBe("true");

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
      { route: paths.timetable() },
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
    // The school's syllabus page is shown inside the panel.
    expect(
      panel.querySelector('iframe[title="Official syllabus"]'),
    ).not.toBeNull();
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
      { route: paths.timetable() },
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
    // Planning only: materials and assignments are read in Lectures.
    expect(panel.querySelector('[role="tab"]')).toBeNull();
    // The search stays behind the panel, and a click on a block never removes it.
    expect(host.querySelector(".coursesSide .courseFinder")).not.toBeNull();
    expect(host.querySelector(".ttBlock.course")).not.toBeNull();

    const toLectures = buttonByText(panel, "Open in Lectures");
    if (!toLectures) throw new Error("the Open in Lectures action is missing");
    await click(toLectures);
    await until(() =>
      expect(window.location.hash).toBe(
        `#${paths.lectures(eligibleCourse.id)}`,
      ),
    );
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
      { route: paths.timetable() },
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
    // The row is offered again and the choice survives, so it can be retried.
    expect(host.querySelector(".finderRow .finderAdd")?.textContent).toBe(
      "Add",
    );
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
      { route: paths.timetable() },
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
      { route: paths.timetable() },
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

  it("explains a placed clash where the course is clicked and offers both ways out", async () => {
    const sectioned = { ...clashingCourse, code: "CSC.T999-02" };
    const workspace = workspaceFixture({
      courses: [eligibleCourse, sectioned],
      planItems: [planItemFor(eligibleCourse), planItemFor(sectioned)],
    });
    // Core names each side of the clash; the shell only renders it.
    const conflicts = [
      [eligibleCourse.id, sectioned.id, sectioned.title],
      [sectioned.id, eligibleCourse.id, eligibleCourse.title],
    ].map(([courseId, withCourseId, withTitle]) => ({
      courseId,
      day: 2,
      startTime: "10:45",
      endTime: "12:25",
      withCourseId,
      withTitle,
    }));
    app = await mountApp(
      {
        revision: freshRevision(),
        workspace,
        queries: {
          ...plannerQueries(),
          planningWeek: (_query: unknown, core: FakeCore) =>
            planningWeekFor(core.workspace, { conflictCount: 1, conflicts }),
        },
        handle: finderHandler({ courses: [eligible] }),
      },
      { route: paths.timetable() },
    );
    const { host, core } = app;
    const panel = () => host.querySelector(".coursesSide .coursePanel");
    const notice = () => panel()?.querySelector(".coursePanelConflict");
    const button = (label: string) =>
      [...(notice()?.querySelectorAll("button") ?? [])].find(
        (node) => node.textContent?.trim() === label,
      );

    // The header count is a way in, not just a number.
    await click(
      await required(
        () => host.querySelector<HTMLButtonElement>(".conflictBadge"),
        "the conflict count",
      ),
    );
    await settled(() =>
      expect(panel()?.querySelector(".finderTitle b")?.textContent).toBe(
        "Graph Algorithms",
      ),
    );
    expect(notice()?.textContent).toContain("Clashing Lecture");
    expect(notice()?.textContent).toContain("10:45–12:25");

    await click(button("Open")!);
    await settled(() =>
      expect(panel()?.querySelector(".finderTitle b")?.textContent).toBe(
        "Clashing Lecture",
      ),
    );
    expect(notice()?.textContent).toContain("Graph Algorithms");

    await click(button("Find another class time")!);
    await settled(() => expect(panel()).toBeNull());
    // Every class group shares the code without its "-02" class suffix.
    await settled(() =>
      expect(core.commands("finder_query").at(-1)?.args.request).toMatchObject({
        keys: ["CSC.T999"],
      }),
    );
  }, 60_000);

  it("offers Replace for a clashing course, commits it, and Undo restores the previous one", async () => {
    const consoleWatch = watchConsoleErrors();
    app = await mountApp(clashApp(), { route: paths.timetable() });
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
    expect(host.querySelector(".pageError")).toBeNull();
    consoleWatch.expectClean();
    consoleWatch.restore();
  }, 60_000);
});

/* ------------------------------------------------------------------ *
 * The course editor, reached from the timetable's course panel.
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
    { route: paths.timetable() },
  );
  await click(
    await required(
      () => mounted.host.querySelector<HTMLButtonElement>(".ttBlock.course"),
      "the placed course block",
    ),
  );
  const panelEdit = await required(
    () =>
      [
        ...mounted.host.querySelectorAll<HTMLButtonElement>(
          ".coursePanelFoot button",
        ),
      ].find((button) => (button.textContent ?? "").trim() === "Edit"),
    "the panel Edit action",
  );
  await click(panelEdit);
  await required(
    () => document.querySelector('[role="combobox"][aria-label="Period"]'),
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
    const period = document.querySelector<HTMLInputElement>(
      '[role="combobox"][aria-label="Period"]',
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

    const period = document.querySelector<HTMLInputElement>(
      '[role="combobox"][aria-label="Period"]',
    )!;
    expect(period.value).toContain("1-4 · 08:50–12:25");

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

    const period = document.querySelector<HTMLInputElement>(
      '[role="combobox"][aria-label="Period"]',
    )!;
    expect(period.value).toBe("Custom time");

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

    const period = document.querySelector<HTMLInputElement>(
      '[role="combobox"][aria-label="Period"]',
    )!;
    expect(period.value).toContain("1-2");

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
  it("switches cached quarters before saving, coalesces bursts and restores a failed selection", async () => {
    const workspace = workspaceFixture();
    workspace.terms = TERMS;
    workspace.selectedTerm = "2026-3Q";
    workspace.currentTerm = "2026-3Q";
    let failNextSelection = false;
    let rejectSelection: ((error: Error) => void) | undefined;
    const quarterCourse = (term: TermId) => ({
      ...meetingCourse({
        days: [1],
        periods: [3, 4],
        startTime: "10:45",
        endTime: "12:25",
        room: "W9",
      }),
      id: term,
      termId: term,
      title: `Cached ${term}`,
    });
    app = await mountApp(
      {
        revision: freshRevision(),
        workspace,
        queries: {
          ...plannerQueries(TERMS),
          planningWeek: (query: unknown) =>
            planningWeekFor({
              ...workspace,
              courses: [quarterCourse((query as { term: TermId }).term)],
            }),
        },
        handle: finderHandler({ courses: [eligible] }, (command) => {
          if (command === "mutate_workspace" && failNextSelection)
            return new Promise((_resolve, reject) => {
              rejectSelection = reject;
            });
          return undefined;
        }),
        apply: (intent, core) => {
          if (intent.kind === "selectTerm")
            core.publish(
              { ...core.workspace, selectedTerm: intent.termId as TermId },
              `${core.revision}+term`,
            );
        },
      },
      { route: paths.timetable() },
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
    // Pin only this gesture's clock so a busy native build cannot turn the
    // assertion about the first click into an assertion after the quiet window.
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "performance"] });
    try {
      await act(async () => {
        host
          .querySelector<HTMLButtonElement>('[aria-label="2025 1Q"]')!
          .click();
        host
          .querySelector<HTMLButtonElement>('[aria-label="2025 3Q"]')!
          .click();
        quarter.click();
      });
      expect(
        core.intents.filter((candidate) => candidate.kind === "selectTerm"),
      ).toHaveLength(1);
      // The last quarter is already selected while only the first has saved.
      expect(core.workspace.selectedTerm).toBe("2025-1Q");
      expect(
        host
          .querySelector('[aria-label="2025 2Q"]')
          ?.getAttribute("aria-selected"),
      ).toBe("true");
      await act(async () => vi.advanceTimersByTimeAsync(100));
    } finally {
      vi.useRealTimers();
    }

    await settled(() =>
      expect(
        host
          .querySelector('[aria-label="2025 2Q"]')
          ?.getAttribute("aria-selected"),
      ).toBe("true"),
    );
    expect(
      core.intents
        .filter((candidate) => candidate.kind === "selectTerm")
        .map((intent) => intent.termId),
    ).toEqual(["2025-1Q", "2025-2Q"]);
    expect(
      host.querySelector(".yearQuarterGroup.expanded .yearQuarterYear")
        ?.textContent,
    ).toBe("2025");
    failNextSelection = true;
    await click(
      host.querySelector<HTMLButtonElement>('[aria-label="2025 4Q"]')!,
    );
    await settled(() => expect(rejectSelection).toBeTypeOf("function"));
    expect(core.workspace.selectedTerm).toBe("2025-2Q");
    expect(
      host
        .querySelector('[aria-label="2025 4Q"]')
        ?.getAttribute("aria-selected"),
    ).toBe("true");
    expect(timetableTitles(host)).toContain("Cached 2025-4Q");
    await act(async () => rejectSelection!(new Error("Selection save failed")));
    await settled(() =>
      expect(
        host
          .querySelector('[aria-label="2025 2Q"]')
          ?.getAttribute("aria-selected"),
      ).toBe("true"),
    );
    expect(timetableTitles(host)).toContain("Cached 2025-2Q");
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
  fetchedAt: string;
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
      "crates/isct-core/tests/fixtures/catalog/course-examples.json",
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
      id: `kyomu:${source.externalId}`,
      rev: 1,
      updatedAt: source.fetchedAt,
      updatedBy: "catalog-fixture",
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
        provider: "kyomu",
        externalId: source.externalId,
        fetchedAt: source.fetchedAt,
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
      { route: paths.timetable() },
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
