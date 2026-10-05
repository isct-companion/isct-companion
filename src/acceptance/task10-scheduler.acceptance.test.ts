import { afterEach, describe, expect, it, vi } from "vitest";
import type { ScheduledTask } from "../core/model";
import {
  createScheduledTaskRuntime,
  dueTaskOccurrence,
  parseTaskSchedule,
} from "../platform/scheduled-tasks";

function task(
  overrides: Partial<ScheduledTask> = {},
): ScheduledTask {
  return {
    id: "local-daily-export",
    rev: 1,
    updatedAt: "2026-09-29T10:00:00+09:00",
    updatedBy: "local-device",
    name: "Local daily export",
    action: "export",
    schedule: "FREQ=DAILY;BYHOUR=21;BYMINUTE=0",
    enabled: true,
    ...overrides,
  };
}

afterEach(() => {
  vi.useRealTimers();
});

describe("Task 10 in-process scheduled tasks", () => {
  it("automatically runs a due daily export once and records its occurrence and result", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-09-29T20:59:30+09:00"));
    const scheduled = task();
    const execute = vi.fn(async () => "Export completed");
    const started = vi.fn(async (
      _taskId: string,
      occurrenceAt: string,
      runAt: string,
    ) => {
      // This mirrors the durable preference write used by the app: the
      // occurrence is claimed before export starts.
      scheduled.lastOccurrenceAt = occurrenceAt;
      scheduled.lastRunAt = runAt;
      scheduled.lastResult = "Running";
    });
    const finished = vi.fn(async (
      _taskId: string,
      occurrenceAt: string,
      runAt: string,
      result: string,
    ) => {
      scheduled.lastOccurrenceAt = occurrenceAt;
      scheduled.lastRunAt = runAt;
      scheduled.lastResult = result;
    });
    const runtime = createScheduledTaskRuntime({
      getTasks: () => [scheduled],
      markStarted: started,
      execute,
      markFinished: finished,
      pollMilliseconds: 30_000,
    });

    await runtime.tick();
    expect(execute).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(30_000);

    const occurrence = "2026-09-29T12:00:00.000Z";
    expect(started).toHaveBeenCalledWith(
      scheduled.id,
      occurrence,
      occurrence,
    );
    expect(execute).toHaveBeenCalledTimes(1);
    expect(execute).toHaveBeenCalledWith(scheduled);
    expect(finished).toHaveBeenCalledWith(
      scheduled.id,
      occurrence,
      occurrence,
      "Export completed",
    );
    expect(scheduled).toMatchObject({
      lastOccurrenceAt: occurrence,
      lastRunAt: occurrence,
      lastResult: "Export completed",
    });

    // Further polls inside the same scheduled occurrence must not export again.
    await vi.advanceTimersByTimeAsync(59_000);
    await runtime.tick();
    expect(execute).toHaveBeenCalledTimes(1);
    runtime.stop();
  });

  it("does not run disabled or future tasks and rejects sub-hour schedules", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-09-29T21:00:00+09:00"));
    const tasks = [
      task({ id: "disabled", enabled: false }),
      task({
        id: "future",
        updatedAt: "2026-09-30T10:00:00+09:00",
      }),
      task({ id: "sub-hour", schedule: "FREQ=MINUTELY;INTERVAL=30" }),
    ];
    const execute = vi.fn(async () => "Export completed");
    const markStarted = vi.fn(async () => undefined);
    const markFinished = vi.fn(async () => undefined);
    const runtime = createScheduledTaskRuntime({
      getTasks: () => tasks,
      markStarted,
      execute,
      markFinished,
    });

    await runtime.tick();
    await vi.advanceTimersByTimeAsync(30_000);
    expect(execute).not.toHaveBeenCalled();
    expect(markStarted).toHaveBeenCalledTimes(1);
    expect(markStarted).toHaveBeenCalledWith(
      "sub-hour",
      "2026-09-29T12:00:00.000Z",
      "2026-09-29T12:00:00.000Z",
    );
    expect(markFinished).toHaveBeenCalledTimes(1);
    expect(markFinished).toHaveBeenCalledWith(
      "sub-hour",
      "2026-09-29T12:00:00.000Z",
      "2026-09-29T12:00:00.000Z",
      expect.stringMatching(/^Failed:.*hourly/i),
    );

    // A rejected rule is reported once, not retried every polling interval.
    await runtime.tick();
    expect(markFinished).toHaveBeenCalledTimes(1);
    runtime.stop();
  });

  it("rejects every recurrence primitive below the one-hour boundary", () => {
    for (const schedule of [
      "FREQ=MINUTELY",
      "FREQ=SECONDLY",
      "FREQ=DAILY;BYHOUR=21;BYSECOND=30",
    ]) {
      expect(() => parseTaskSchedule(schedule), schedule).toThrow();
    }
    expect(parseTaskSchedule("FREQ=HOURLY;INTERVAL=1;BYMINUTE=15")).toMatchObject({
      kind: "rrule",
      frequency: "HOURLY",
      interval: 1,
      minute: 15,
    });
  });

  it("does not catch up a daily occurrence missed while the app was closed", () => {
    const scheduled = task();
    expect(
      dueTaskOccurrence(
        scheduled,
        new Date("2026-09-29T21:05:00+09:00"),
        new Date("2026-09-29T21:05:00+09:00"),
      ),
    ).toBeUndefined();
  });
});
