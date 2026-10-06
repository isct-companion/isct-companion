// @vitest-environment jsdom
import { beforeAll, describe, expect, it } from "vitest";
import type { ScheduledTask } from "../core/model";
import { workspaceActions, workspaceStore } from "../store/workspace";

const STORAGE_KEY = "isct-companion-workspace-v3";

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

describe("Task 10 scheduled-task run persistence", () => {
  beforeAll(async () => {
    localStorage.clear();
    await workspaceStore.initialize();
    workspaceActions.updateSettings((draft) => {
      draft.scheduledTasks = [scheduledExport()];
    });
    await workspaceStore.flush();
  });

  it("durably records the claimed occurrence, completion time, and result", async () => {
    const occurrenceAt = "2026-09-29T12:00:00.000Z";
    const runAt = "2026-09-29T12:00:01.000Z";

    await workspaceActions.recordScheduledTaskRun(
      "local-daily-export",
      occurrenceAt,
      runAt,
      "Export completed",
    );

    const task = workspaceStore.getState().workspace?.scheduledTasks[0];
    expect(task).toMatchObject({
      id: "local-daily-export",
      lastOccurrenceAt: occurrenceAt,
      lastRunAt: runAt,
      lastResult: "Export completed",
      rev: 2,
      updatedAt: runAt,
    });
    expect(task?.updatedBy).toBe(
      workspaceStore.getState().workspace?.deviceId,
    );

    const persisted = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null");
    expect(persisted.scheduledTasks[0]).toMatchObject({
      lastOccurrenceAt: occurrenceAt,
      lastRunAt: runAt,
      lastResult: "Export completed",
      rev: 2,
      updatedAt: runAt,
    });
  });
});
