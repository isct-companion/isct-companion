// @vitest-environment jsdom
import { act } from "react";
import { afterEach, describe, expect, it } from "vitest";
import {
  buttonByText,
  click,
  mountApp,
  overviewFor,
  planningWeekFor,
  scheduleViewFor,
  typeInto,
  until,
  workspaceFixture,
  type FakeCore,
  type MountedApp,
} from "@tests/harness";
import { paths } from "@app/app/routes";
import type { Plan, ScheduledTask, Workspace } from "@app/lib/model";
import type { WorkspaceIntent } from "@app/lib/workspace-intents";

/**
 * The plan editor and the scheduled-task automation page, over the real shell
 * and a scripted Core.
 *
 * The legacy suites mounted `PlanEditor` and `Settings` directly and replaced
 * `platform/core-query` with `vi.mock`, so the "next occurrences" preview was
 * really an assertion about the mock. Here both surfaces are reached through
 * the shipped routes: the plan editor opens from the timetable block a person
 * clicks, the automation page from its settings route, and Core answers the
 * real query path through the harness `queries` table.
 *
 * Local helpers below exist because the harness has no plan-block builder
 * (`planningWeekWithPlan`), no modal reader (`modal`, `modalText`) and no
 * labelled-field lookup (`fieldInput`).
 *
 * Durable run records: the shipped frontend never emits
 * `recordScheduledTaskRun` itself — Core records a run inside
 * `run_scheduled_task` — so this suite covers the durable-record contract at
 * the level the UI can reach: Run now claims the run for the task, and the
 * shell then shows Core's durable run time and result and keeps using Core's
 * advanced revision. The store-level intent payload and adoption assertions
 * belong to `tests/unit/workspace-store.test.ts`.
 */

/* ------------------------------------------------------------------ *
 * Fixtures.
 * ------------------------------------------------------------------ */

const WEEKLY_RULE = "FREQ=WEEKLY;BYDAY=TU,FR";
/** What Core answers for the plan's weekly recurrence. */
const WEEKLY_OCCURRENCES = [
  "2026-09-29",
  "2026-10-02",
  "2026-10-06",
  "2026-10-09",
  "2026-10-13",
];
/** A daily recurrence overlaps two of those dates and adds 09-30, 10-01, 10-03. */
const DAILY_OCCURRENCES = [
  "2026-09-29",
  "2026-09-30",
  "2026-10-01",
  "2026-10-02",
  "2026-10-03",
];

const plan: Plan = {
  id: "plan-weekly-study",
  rev: 1,
  updatedAt: "2026-09-28T00:00:00.000Z",
  updatedBy: "device:test",
  title: "Weekly study plan",
  start: "2026-09-29T19:00:00+09:00",
  duration: 60,
  rrule: WEEKLY_RULE,
  exdates: [],
};

const DAILY_EXPORT: ScheduledTask = {
  id: "local-daily-export",
  rev: 3,
  updatedAt: "2026-09-29T00:00:00.000Z",
  updatedBy: "local-device",
  name: "Local daily export",
  action: "export",
  schedule: "FREQ=DAILY;BYHOUR=21;BYMINUTE=0",
  enabled: false,
};

/* ------------------------------------------------------------------ *
 * Local helpers the harness does not provide.
 * ------------------------------------------------------------------ */

/** Core's planning week with one plan block, so the timetable can show it. */
function planningWeekWithPlan(workspace: Workspace): Record<string, unknown> {
  const week = planningWeekFor(workspace) as {
    days: { day: number; blocks: Record<string, unknown>[] }[];
  };
  const tuesday = week.days.find((day) => day.day === 2);
  if (!tuesday) throw new Error("the planning week has no Tuesday");
  tuesday.blocks.push({
    key: `plan-${plan.id}`,
    id: plan.id,
    kind: "plan",
    date: "",
    title: plan.title,
    room: "",
    startTime: "19:00",
    endTime: "20:00",
    color: "#556b8d",
  });
  return week;
}

function withTask(workspace: Workspace, task: ScheduledTask): Workspace {
  const next = structuredClone(workspace);
  next.scheduledTasks = next.scheduledTasks.map((row) =>
    row.id === task.id ? task : row,
  );
  return next;
}

/** The plan editor renders in a portal, outside the mounted shell. */
function modal(): HTMLElement | null {
  return document.querySelector<HTMLElement>(".mantine-Modal-content");
}

function modalText(): string {
  return modal()?.textContent ?? "";
}

function modalButton(label: string): HTMLButtonElement | undefined {
  const root = modal();
  return root ? buttonByText(root, label) : undefined;
}

/** The input behind a Mantine `label`, so a rewrite does not depend on order. */
function fieldInput(label: string): HTMLInputElement {
  const root = modal();
  if (!root) throw new Error("The plan editor is not open.");
  const wrapper = [
    ...root.querySelectorAll<HTMLElement>(".mantine-InputWrapper-root"),
  ].find((node) => node.querySelector("label")?.textContent?.trim() === label);
  const input = wrapper?.querySelector<HTMLInputElement>("input");
  if (!input) throw new Error(`The "${label}" field is not on screen.`);
  return input;
}

function taskSwitch(host: HTMLElement): HTMLInputElement | null {
  return host.querySelector<HTMLInputElement>('input[type="checkbox"]');
}

type TaskIntent = Extract<WorkspaceIntent, { kind: "saveScheduledTask" }>;

function intentsOfKind<T extends WorkspaceIntent["kind"]>(
  core: FakeCore,
  kind: T,
): Extract<WorkspaceIntent, { kind: T }>[] {
  return core.intents.filter((intent) => intent.kind === kind) as Extract<
    WorkspaceIntent,
    { kind: T }
  >[];
}

type OccurrenceQuery = {
  kind: string;
  plan?: { rrule?: string };
  limit?: number;
};

/** Every `nextPlanOccurrences` request Core received, in order. */
function occurrenceQueries(core: FakeCore): OccurrenceQuery[] {
  return core
    .commands("schedule_query")
    .flatMap((call) => call.args.queries as OccurrenceQuery[])
    .filter((query) => query.kind === "nextPlanOccurrences");
}

/** Opens the existing plan the way a person does: the plan block in the planner. */
async function openPlanEditor(app: MountedApp): Promise<void> {
  const block = await until(() => {
    const node = app.host.querySelector<HTMLButtonElement>(".ttBlock.plan");
    if (!node) throw new Error("The plan is not on the timetable.");
    return node;
  });
  await click(block);
  await until(() => {
    if (!modal()) throw new Error("The plan editor did not open.");
    return true;
  });
}

async function openAutomation(app: MountedApp): Promise<void> {
  await until(() => {
    const toggle = taskSwitch(app.host);
    if (!toggle) throw new Error("The automation page has no task switch.");
    return toggle;
  });
}

/* ------------------------------------------------------------------ *
 * Suite state.
 * ------------------------------------------------------------------ */

let app: MountedApp | undefined;

afterEach(async () => {
  await app?.unmount();
  app = undefined;
});

function planQueries() {
  return {
    workspaceOverview: (_query: unknown, core: FakeCore) =>
      overviewFor(core.workspace),
    planningWeek: (_query: unknown, core: FakeCore) =>
      planningWeekWithPlan(core.workspace),
    scheduleView: (_query: unknown, core: FakeCore) =>
      scheduleViewFor(core.workspace),
    planCivilStart: () => ({ date: "2026-09-29", time: "19:00" }),
    // Core derives occurrences from the recurrence in the request.
    nextPlanOccurrences: (query: unknown) =>
      (query as { plan?: { rrule?: string } }).plan?.rrule?.includes("DAILY")
        ? DAILY_OCCURRENCES
        : WEEKLY_OCCURRENCES,
  };
}

/* ------------------------------------------------------------------ *
 * The plan editor.
 * ------------------------------------------------------------------ */

describe("The plan editor", () => {
  it("shows Core's next five occurrences for the plan's recurrence and follows a rule change", async () => {
    const workspace = workspaceFixture();
    workspace.plans = [plan];

    app = await mountApp(
      { workspace, queries: planQueries() },
      { route: paths.timetable() },
    );
    const { core } = app;

    await openPlanEditor(app);
    await until(() => expect(modalText()).toContain("Next 5 occurrences"));
    for (const date of WEEKLY_OCCURRENCES) {
      expect(modalText()).toContain(date);
    }

    // Core was asked about the plan's chosen recurrence, five at a time.
    const first = occurrenceQueries(core)[0];
    expect(first.plan?.rrule).toBe(WEEKLY_RULE);
    expect(first.limit).toBe(5);

    // Choosing another recurrence asks Core again and shows its new answer.
    await typeInto(fieldInput("RRULE"), "FREQ=DAILY");
    await until(() => expect(modalText()).toContain("2026-09-30"));
    for (const date of DAILY_OCCURRENCES) {
      expect(modalText()).toContain(date);
    }
    expect(modalText()).not.toContain("2026-10-13");
    expect(occurrenceQueries(core).at(-1)?.plan?.rrule).toBe("FREQ=DAILY");
  }, 40_000);

  it("saves once when Save is pressed twice before the first save settles", async () => {
    const workspace = workspaceFixture();
    workspace.plans = [plan];

    app = await mountApp(
      { workspace, queries: planQueries() },
      { route: paths.timetable() },
    );
    const { core } = app;

    await openPlanEditor(app);
    const save = await until(() => {
      const button = modalButton("Save");
      if (!button) throw new Error("The Save button is not on screen.");
      return button;
    });

    // Two clicks in one frame, before React can render the busy state.
    act(() => {
      save.click();
      save.click();
    });
    expect(save.disabled).toBe(true);

    await until(() => expect(intentsOfKind(core, "savePlan")).toHaveLength(1));
    const saved = intentsOfKind(core, "savePlan")[0];
    expect(saved.existingId).toBe(plan.id);
    expect(saved.draft).toMatchObject({ title: plan.title, duration: 60 });
    await until(() => expect(modal()).toBeNull());
  }, 40_000);

  it("reports a save Core rejects and accepts a later retry", async () => {
    const workspace = workspaceFixture();
    workspace.plans = [plan];
    let rejectFirst = true;

    app = await mountApp(
      {
        workspace,
        queries: planQueries(),
        handle: (command) => {
          if (command === "mutate_workspace" && rejectFirst) {
            rejectFirst = false;
            throw new Error("Core rejected the save");
          }
          return undefined;
        },
      },
      { route: paths.timetable() },
    );
    const { core } = app;

    await openPlanEditor(app);
    await click(modalButton("Save"));
    await until(() =>
      expect(
        modal()?.querySelector('[role="alert"]')?.textContent,
      ).toBeTruthy(),
    );
    expect(modal()).not.toBeNull();

    await click(modalButton("Save"));
    await until(() => expect(intentsOfKind(core, "savePlan")).toHaveLength(1));
    await until(() => expect(modal()).toBeNull());
  }, 40_000);
});

/* ------------------------------------------------------------------ *
 * Scheduled tasks.
 * ------------------------------------------------------------------ */

describe("Scheduled tasks", () => {
  it("keeps the existing schedule and revision when a task is switched on", async () => {
    const workspace = workspaceFixture();
    // `hook` has no plain-language hint, so the page shows the real schedule.
    workspace.scheduledTasks = [{ ...DAILY_EXPORT, action: "hook" }];

    app = await mountApp(
      {
        workspace,
        queries: {
          workspaceOverview: (_query, core) => overviewFor(core.workspace),
        },
        apply: (intent, core) => {
          if (intent.kind === "saveScheduledTask") {
            core.publish(
              withTask(core.workspace, intent.draft as ScheduledTask),
              "revision:2",
            );
          }
        },
      },
      { route: paths.settings("automation") },
    );
    const { host, core } = app;

    await openAutomation(app);
    const toggle = taskSwitch(host)!;
    expect(host.textContent).toContain("21:00");
    expect(toggle.checked).toBe(false);

    await click(toggle);

    await until(() => expect(taskSwitch(host)?.checked).toBe(true));
    // The page still describes the schedule the task already had.
    expect(host.textContent).toContain("21:00");

    const saved = core.intents.find(
      (intent): intent is TaskIntent => intent.kind === "saveScheduledTask",
    );
    expect(saved?.existingId).toBe(DAILY_EXPORT.id);
    expect(saved?.draft).toMatchObject({
      id: DAILY_EXPORT.id,
      enabled: true,
      rev: 3,
      schedule: DAILY_EXPORT.schedule,
    });
  }, 40_000);

  it("runs the task from Run now and adopts Core's durable record and revision", async () => {
    const workspace = workspaceFixture();
    workspace.scheduledTasks = [{ ...DAILY_EXPORT, enabled: true }];
    const durable: ScheduledTask = {
      ...DAILY_EXPORT,
      enabled: true,
      rev: 4,
      updatedAt: "2026-09-29T12:00:01.000Z",
      // The occurrence Core records the run against, together with its result.
      lastOccurrenceAt: "2026-09-29T12:00:00.000Z",
      lastRunAt: "2026-09-29T12:00:01.000Z",
      lastResult: "Export completed",
    };

    app = await mountApp(
      {
        workspace,
        queries: {
          workspaceOverview: (_query, core) => overviewFor(core.workspace),
        },
        handle: (command, _args, core) => {
          if (command === "run_scheduled_task") {
            // Core records the run durably, at its own revision, and
            // publishes the record as a scheduler change.
            core.publish(withTask(core.workspace, durable), "revision:42");
            core.emit("workspace-changed", {
              source: "scheduler",
              changes: ["scheduled-tasks"],
            });
            return "Export completed";
          }
          return undefined;
        },
        apply: (intent, core) => {
          if (intent.kind === "saveScheduledTask") {
            core.publish(
              withTask(core.workspace, intent.draft as ScheduledTask),
              "revision:43",
            );
          }
        },
      },
      { route: paths.settings("automation") },
    );
    const { host, core } = app;

    await openAutomation(app);
    expect(host.textContent).toContain("Not run yet");

    await click(buttonByText(host, "Run now"));

    // The UI claimed the run for this task...
    const run = await until(() => {
      const call = core.commands("run_scheduled_task").at(-1);
      if (!call) throw new Error("Run now did not reach Core.");
      return call;
    });
    expect(run.args).toEqual({ id: DAILY_EXPORT.id });

    // ...and shows the run Core durably recorded, not one invented locally.
    // Core's 12:00 UTC is 21:00 in Tokyo, where the app shows run times.
    await until(() =>
      expect(host.textContent).toMatch(/Last run 29 Sept?, 21:00/),
    );
    expect(host.textContent).toContain("Export completed");
    expect(host.textContent).not.toContain("Not run yet");

    // The store adopted Core's revision: the next edit expects it.
    await click(taskSwitch(host));
    await until(() =>
      expect(
        core.mutations.filter(
          (mutation) => mutation.intent.kind === "saveScheduledTask",
        ),
      ).toHaveLength(1),
    );
    const nextEdit = core.mutations.find(
      (mutation) => mutation.intent.kind === "saveScheduledTask",
    );
    expect(nextEdit?.expectedRevision).toBe("revision:42");
  }, 40_000);
});
