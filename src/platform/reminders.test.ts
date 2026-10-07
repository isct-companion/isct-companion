import { afterEach, describe, expect, it, vi } from "vitest";
import { seedWorkspace, type Assignment, type Course } from "../core/model";
import {
  dueReminders,
  firedReminderLog,
  startReminderRuntime,
} from "./reminders";

const meta = { rev: 1, updatedAt: "2026-01-01T00:00:00Z", updatedBy: "test" };

/** Monday 2026-04-13 is an ordinary 1Q teaching day. */
function fixture() {
  const data = seedWorkspace();
  data.selectedTerm = "2026-1Q";
  data.settings.notifications.localPermission = "granted";
  const course: Course = {
    ...meta,
    id: "course",
    kind: "course",
    termId: "2026-1Q",
    title: "Local algorithms",
    code: "LOC.T101",
    instructor: "",
    credits: 2,
    creditSplit: { lecture: 2, exercise: 0, lab: 0 },
    instructionLanguage: "en",
    color: "#123456",
    source: { provider: "manual" },
    meetings: [
      {
        days: [1],
        periods: [1, 2],
        startTime: "08:50",
        endTime: "10:30",
        room: "M-101",
      },
    ],
  };
  data.courses = [course];
  data.memberships = [
    {
      ...meta,
      id: "member",
      kind: "membership",
      termId: "2026-1Q",
      courseId: course.id,
      status: "registered",
      evidence: { provider: "manual" },
    },
  ];
  data.academicCalendar = [
    {
      ...meta,
      id: "calendar:2026:quarter-start:1Q",
      academicYear: 2026,
      quarter: "1Q",
      date: "2026-04-07",
      type: "quarter-start",
      label: { en: "1Q starts" },
    },
    {
      ...meta,
      id: "calendar:2026:quarter-end:1Q",
      academicYear: 2026,
      quarter: "1Q",
      date: "2026-06-09",
      type: "quarter-end",
      label: { en: "1Q ends" },
    },
  ];
  return data;
}

const jst = (value: string) => new Date(`${value}+09:00`);

function assignment(values: Partial<Assignment>): Assignment {
  return {
    ...meta,
    id: "assignment",
    kind: "assignment",
    courseId: "course",
    title: "Local report",
    details: "",
    dueAt: "2026-04-14T23:59:00+09:00",
    source: { provider: "manual" },
    ...values,
  };
}

describe("class reminders", () => {
  it("fires at the chosen lead time and not before", () => {
    const data = fixture();
    expect(dueReminders(data, jst("2026-04-13T08:39:00"))).toEqual([]);
    const [reminder] = dueReminders(data, jst("2026-04-13T08:40:30"));
    expect(reminder).toMatchObject({
      title: "Local algorithms",
      body: "Starts in 10 min · 08:50 · M-101",
    });
    data.settings.notifications.classLeadMinutes = 30;
    expect(dueReminders(data, jst("2026-04-13T08:20:00"))).toHaveLength(1);
  });

  it("skips reminders missed by more than five minutes", () => {
    expect(dueReminders(fixture(), jst("2026-04-13T08:46:00"))).toEqual([]);
  });

  it("follows calendar changes that cancel or move classes", () => {
    const data = fixture();
    data.exceptions = [
      {
        ...meta,
        id: "off",
        date: "2026-04-13",
        type: "no-class",
        label: "Off",
      },
      {
        ...meta,
        id: "monday-on-wednesday",
        date: "2026-04-15",
        type: "replacement",
        replacementDay: 1,
        label: "Monday classes",
      },
    ];
    expect(dueReminders(data, jst("2026-04-13T08:40:00"))).toEqual([]);
    expect(dueReminders(data, jst("2026-04-15T08:40:00"))).toHaveLength(1);
  });

  it("stays quiet when class reminders are off", () => {
    const data = fixture();
    data.settings.notifications.classes = false;
    expect(dueReminders(data, jst("2026-04-13T08:40:00"))).toEqual([]);
  });
});

describe("assignment reminders", () => {
  it("fires one day and one hour before an open deadline", () => {
    const data = fixture();
    data.assignments = [assignment({})];
    expect(
      dueReminders(data, jst("2026-04-13T23:59:00")).map((item) => item.body),
    ).toEqual(["Due in 24 hours · Local algorithms"]);
    expect(
      dueReminders(data, jst("2026-04-14T22:59:30")).map((item) => item.key),
    ).toEqual(["assignment:assignment:60"]);
  });

  it("ignores submitted, hidden and undated work", () => {
    const data = fixture();
    data.assignments = [
      assignment({ id: "done", submission: "submitted" }),
      assignment({ id: "hidden", hidden: true }),
      assignment({ id: "undated", dueAt: null }),
    ];
    expect(dueReminders(data, jst("2026-04-13T23:59:00"))).toEqual([]);
  });
});

describe("reminder runtime", () => {
  afterEach(() => vi.useRealTimers());

  it("delivers each reminder once and only with permission", () => {
    vi.useFakeTimers();
    const data = fixture();
    const store = new Map<string, string>();
    const storage = {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => void store.set(key, value),
    };
    const deliver = vi.fn();
    let clock = jst("2026-04-13T08:40:00");
    data.settings.notifications.localPermission = "default";
    const stop = startReminderRuntime({
      getWorkspace: () => data,
      deliver,
      storage,
      now: () => clock,
    });
    expect(deliver).not.toHaveBeenCalled();
    data.settings.notifications.localPermission = "granted";
    vi.advanceTimersByTime(30_000);
    clock = jst("2026-04-13T08:41:00");
    vi.advanceTimersByTime(30_000);
    stop();
    expect(deliver).toHaveBeenCalledTimes(1);
    // A restart reads the same log and does not repeat it.
    expect(firedReminderLog(storage).has(deliver.mock.calls[0][0].key)).toBe(
      true,
    );
  });
});
