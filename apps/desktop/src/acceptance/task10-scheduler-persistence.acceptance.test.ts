import { describe, expect, it, vi } from "vitest";
import { seedWorkspace, type ScheduledTask } from "../lib/model";
import type { CoreWorkspaceSnapshot } from "../lib/workspace-intents";
import { createWorkspaceStore } from "../store/workspace";

const emptySchoolRecords = {
  lastRuns: {},
  lmsCourses: [],
  registrations: [],
  registrationStates: [],
  grades: [],
  gpaByTerm: [],
  requirements: [],
};

function scheduledExport(): ScheduledTask {
  return {
    id: "local-daily-export",
    rev: 1,
    updatedAt: "2026-09-29T10:00:00.000Z",
    updatedBy: "local-device",
    name: "Local daily export",
    action: "export",
    schedule: "FREQ=DAILY;BYHOUR=21;BYMINUTE=0",
    enabled: true,
  };
}

describe("Task 10 scheduled-task Core persistence boundary", () => {
  it("sends the claimed occurrence and adopts Core's durable result", async () => {
    const workspace = seedWorkspace();
    workspace.scheduledTasks = [scheduledExport()];
    const initial: CoreWorkspaceSnapshot = {
      revision: "before",
      contentRevision: "before",
      workspace,
      schoolRecords: emptySchoolRecords,
    };
    const occurrenceAt = "2026-09-29T12:00:00.000Z";
    const runAt = "2026-09-29T12:00:01.000Z";
    const persisted = structuredClone(workspace);
    Object.assign(persisted.scheduledTasks[0], {
      lastOccurrenceAt: occurrenceAt,
      lastRunAt: runAt,
      lastResult: "Export completed",
      rev: 2,
      updatedAt: runAt,
    });
    const mutate = vi.fn(
      async () =>
        ({
          revision: "after",
          contentRevision: "after",
          workspace: persisted,
          schoolRecords: emptySchoolRecords,
        }) satisfies CoreWorkspaceSnapshot,
    );
    const store = createWorkspaceStore({
      snapshot: vi.fn(async () => initial),
      mutate,
    });
    await store.initialize();
    await store.mutate({
      kind: "recordScheduledTaskRun",
      taskId: "local-daily-export",
      occurrenceAt,
      runAt,
      result: "Export completed",
    });

    expect(mutate).toHaveBeenCalledWith({
      expectedRevision: "before",
      intent: {
        kind: "recordScheduledTaskRun",
        taskId: "local-daily-export",
        occurrenceAt,
        runAt,
        result: "Export completed",
      },
    });
    expect(store.getState().workspace?.scheduledTasks[0]).toMatchObject({
      lastOccurrenceAt: occurrenceAt,
      lastRunAt: runAt,
      lastResult: "Export completed",
      rev: 2,
      updatedAt: runAt,
    });
  });
});
