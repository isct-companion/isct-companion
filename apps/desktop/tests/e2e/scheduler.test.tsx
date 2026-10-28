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
  until,
  workspaceFixture,
  type FakeCore,
  type MountedApp,
} from "@tests/harness";
import { paths } from "@app/app/routes";
import type { PersonalEvent, ScheduledTask, Workspace } from "@app/lib/model";
import type { WorkspaceIntent } from "@app/lib/workspace-intents";

/**
 * The weekly event editor and the scheduled-task automation page, over the
 * real shell and a scripted Core.
 *
 * Both surfaces are reached through the shipped routes: the event editor
 * opens from the timetable block a person clicks, the automation page from
 * its settings route, and Core answers the real query path through the
 * harness `queries` table.
 *
 * Local helpers below exist because the harness has no event-block builder
 * (`planningWeekWithEvent`) and no dialog reader (`modal`).
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

const event: PersonalEvent = {
  id: "event-lab-meeting",
  rev: 1,
  updatedAt: "2026-09-28T00:00:00.000Z",
  updatedBy: "device:test",
  kind: "event",
  termId: "2026-3Q",
  title: "Lab meeting",
  room: "W9",
  days: [2],
  startTime: "19:00",
  endTime: "20:00",
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

/** Core's planning week with the event's block, so the timetable can show it. */
function planningWeekWithEvent(workspace: Workspace): Record<string, unknown> {
  const week = planningWeekFor(workspace) as {
    days: { day: number; blocks: Record<string, unknown>[] }[];
  };
  const tuesday = week.days.find((day) => day.day === 2);
  if (!tuesday) throw new Error("the planning week has no Tuesday");
  tuesday.blocks.push({
    key: `${event.id}-2`,
    id: event.id,
    kind: "event",
    date: "",
    title: event.title,
    room: event.room,
    startTime: event.startTime,
    endTime: event.endTime,
    color: "#8a887f",
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

/** The event editor renders in a portal, outside the mounted shell. */
function modal(): HTMLElement | null {
  return document.querySelector<HTMLElement>('[role="dialog"]');
}

function modalButton(label: string): HTMLButtonElement | undefined {
  const root = modal();
  return root ? buttonByText(root, label) : undefined;
}

function taskSwitch(host: HTMLElement): HTMLInputElement | null {
  return host.querySelector<HTMLInputElement>('[role="switch"]');
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

/** Opens the existing event the way a person does: its block in the planner. */
async function openEventEditor(app: MountedApp): Promise<void> {
  const block = await until(() => {
    const node = app.host.querySelector<HTMLButtonElement>(".ttBlock.event");
    if (!node) throw new Error("The event is not on the timetable.");
    return node;
  });
  await click(block);
  await until(() => {
    if (!modal()) throw new Error("The event editor did not open.");
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

/** The editor's Save as a person sees it: enabled. */
const enabledSave = () =>
  until(() => {
    const button = modalButton("Save");
    if (!button || button.disabled)
      throw new Error("The Save button is not enabled yet.");
    return button;
  });

afterEach(async () => {
  await app?.unmount();
  app = undefined;
});

function eventQueries() {
  return {
    workspaceOverview: (_query: unknown, core: FakeCore) =>
      overviewFor(core.workspace),
    planningWeek: (_query: unknown, core: FakeCore) =>
      planningWeekWithEvent(core.workspace),
    scheduleView: (_query: unknown, core: FakeCore) =>
      scheduleViewFor(core.workspace),
  };
}

/* ------------------------------------------------------------------ *
 * The weekly event editor.
 * ------------------------------------------------------------------ */

describe("The weekly event editor", () => {
  it("saves once when Save is pressed twice before the first save settles", async () => {
    const workspace = workspaceFixture();
    workspace.events = [event];

    app = await mountApp(
      { workspace, queries: eventQueries() },
      { route: paths.timetable() },
    );
    const { core } = app;

    await openEventEditor(app);
    const save = await enabledSave();

    // Two clicks in one frame, before React can render the busy state.
    act(() => {
      save.click();
      save.click();
    });
    expect(save.disabled).toBe(true);

    await until(() => expect(intentsOfKind(core, "saveEvent")).toHaveLength(1));
    const saved = intentsOfKind(core, "saveEvent")[0];
    expect(saved.existingId).toBe(event.id);
    expect(saved.termId).toBe(event.termId);
    expect(saved.draft).toEqual({
      title: event.title,
      room: event.room,
      days: event.days,
      startTime: event.startTime,
      endTime: event.endTime,
    });
    await until(() => expect(modal()).toBeNull());
  }, 40_000);

  it("reports a save Core rejects and accepts a later retry", async () => {
    const workspace = workspaceFixture();
    workspace.events = [event];
    let rejectFirst = true;

    app = await mountApp(
      {
        workspace,
        queries: eventQueries(),
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

    await openEventEditor(app);
    await click(await enabledSave());
    await until(() =>
      expect(
        modal()?.querySelector('[role="alert"]')?.textContent,
      ).toBeTruthy(),
    );
    expect(modal()).not.toBeNull();

    await click(modalButton("Save"));
    await until(() => expect(intentsOfKind(core, "saveEvent")).toHaveLength(1));
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
    expect(toggle.getAttribute("aria-checked")).toBe("false");

    await click(toggle);

    await until(() =>
      expect(taskSwitch(host)?.getAttribute("aria-checked")).toBe("true"),
    );
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
              seq: 1,
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
