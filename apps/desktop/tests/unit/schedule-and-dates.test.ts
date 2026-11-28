import { afterEach, describe, expect, it, vi } from "vitest";

import {
  formatDay,
  formatTimestamp,
  isCivilDate,
  parseIsoTimestamp,
  tokyoDate,
} from "@app/lib/civil-date";
import { axisPosition, nowPercent } from "@app/lib/timetable-layout";
import {
  agendaColumns,
  planningColumns,
} from "@app/components/timetable/workspace";
import type { DatedAgenda, PlanningWeek } from "@app/platform/core-query";
import { DEFAULT_TIMETABLE_AXIS } from "@tests/harness/fixtures";
import {
  minuteNow,
  resyncClock,
  subscribeClock,
} from "@app/platform/use-clock";

/**
 * Dates, timetable geometry and the day formatting the
 * subscription status view renders. Everything here is pure logic, so the suite
 * runs in plain node.
 */

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

const RealDateTimeFormat = Intl.DateTimeFormat;

/**
 * A device whose locale defaults are non-Gregorian. Production must pin
 * `calendar` and `numberingSystem` itself, so this is the one spy the suite
 * keeps: it makes every formatter the code does not configure explicitly fall
 * back to a Buddhist calendar with Arabic-Indic digits, and the assertions then
 * prove the machine date is still Gregorian with Latin digits.
 */
function buddhistDefaultDateTimeFormat(
  locale?: string | string[],
  options?: Intl.DateTimeFormatOptions,
): Intl.DateTimeFormat {
  return new RealDateTimeFormat(locale, {
    calendar: "buddhist",
    numberingSystem: "arab",
    ...options,
  });
}

function installNonGregorianDefaultLocale(): void {
  vi.spyOn(Intl, "DateTimeFormat").mockImplementation(
    buddhistDefaultDateTimeFormat as unknown as typeof Intl.DateTimeFormat,
  );
}

describe("civil date and ISO timestamp validation", () => {
  it("accepts zero-padded Gregorian dates and rejects localized, unpadded, impossible and non-string ones", () => {
    expect(isCivilDate("2026-04-13")).toBe(true);
    expect(isCivilDate("2028-02-29")).toBe(true);
    for (const value of [
      "04/13/2026",
      "2026-4-13",
      "2026-13-01",
      "2026-02-29",
      "2026-02-30",
      "",
      null,
    ])
      expect(isCivilDate(value)).toBe(false);
  });

  it("parses an explicit offset or Z timestamp to its instant and rejects everything else", () => {
    expect(parseIsoTimestamp("2026-10-13T12:00:00+09:00")?.toISOString()).toBe(
      "2026-10-13T03:00:00.000Z",
    );
    expect(parseIsoTimestamp("2026-10-13T03:00:00Z")?.toISOString()).toBe(
      "2026-10-13T03:00:00.000Z",
    );
    for (const value of [
      "10/13/2026",
      "2026-10-13",
      "2026-02-30T00:00:00Z",
      "2026-10-13T24:00:00Z",
      null,
    ])
      expect(parseIsoTimestamp(value)).toBeUndefined();
  });
});

describe("machine dates under a non-Gregorian device locale", () => {
  it("keeps the Japan calendar date Gregorian and its digits Latin", () => {
    installNonGregorianDefaultLocale();
    expect(tokyoDate(new Date("2026-04-12T14:59:00Z"))).toBe("2026-04-12");
    expect(tokyoDate(new Date("2026-04-12T15:01:00Z"))).toBe("2026-04-13");
  });
});

describe("JST timestamp display", () => {
  it("renders Japan time in the requested locale whatever zone is asked, and nothing for malformed timestamps", () => {
    expect(formatTimestamp("2026-12-31T15:01:00Z", "en-GB")).toBe(
      "1 Jan 2027, 00:01",
    );
    for (const locale of ["zh", "ja"]) {
      const text = formatTimestamp("2026-12-31T15:01:00Z", locale)!;
      expect(text).toContain("2027");
      expect(text).toContain("00:01");
      expect(text).not.toContain("2026");
    }
    expect(
      formatTimestamp("2026-12-31T15:01:00Z", "en-GB", {
        timeZone: "America/Los_Angeles",
      }),
    ).toBe("1 Jan 2027, 00:01");
    for (const invalid of [
      "Invalid Date",
      "2026-02-30T10:00:00Z",
      "10/14/2026",
      null,
    ])
      expect(formatTimestamp(invalid, "en-GB")).toBeUndefined();
  });
});

describe("shell clock", () => {
  // The today column once came from a component's own interval, which stopped
  // while the page was hidden: after a night away the now line sat in
  // yesterday's column. The clock keeps no copy of the time to go stale.
  it("reads the new day after a night its timer never ran, and wakes subscribers on resync", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-10-22T13:10:30Z")); // Thu 22:10 in Tokyo
    const woke = vi.fn();
    const stop = subscribeClock(woke);
    expect(tokyoDate(new Date(minuteNow()))).toBe("2026-10-22");

    vi.setSystemTime(new Date("2026-10-23T00:30:10Z")); // Fri 09:30, no tick
    expect(woke).not.toHaveBeenCalled();
    expect(minuteNow()).toBe("2026-10-23T00:30:00.000Z");
    expect(tokyoDate(new Date(minuteNow()))).toBe("2026-10-23");

    // Coming back to the foreground re-renders at once, then the next minute
    // boundary wakes subscribers again.
    resyncClock();
    expect(woke).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(50_000);
    expect(woke).toHaveBeenCalledTimes(2);

    // The last subscriber leaving stops the timer.
    stop();
    expect(vi.getTimerCount()).toBe(0);
  });
});

// Where times land (one row per period, lunch half a row, short breaks
// folded) is Core's `timetable_axis` and its tests; these cover the
// shell's interpolation over Core's stops.
describe("timetable axis placement", () => {
  const axis = DEFAULT_TIMETABLE_AXIS;

  it("fills a class's cell through the short break and keeps exact times", () => {
    expect(axisPosition(axis, "08:50", "10:30")).toEqual({
      topPercent: 0,
      heightPercent: 16,
    });
    // An exam at 09:40–10:20 sits inside its cell.
    expect(axisPosition(axis, "09:40", "10:20").topPercent).toBe(8);
    // A few minutes still leave room for the title.
    expect(axisPosition(axis, "09:00", "09:01").heightPercent).toBeCloseTo(3.2);
  });

  it("draws the now line only inside the drawn day", () => {
    expect(nowPercent(axis, 13 * 60 + 30)).toBe(36);
    expect(nowPercent(axis, 7 * 60)).toBeUndefined();
    expect(nowPercent(axis, 21 * 60)).toBeUndefined();
  });

  // The now line and today tint follow `isToday`; a column adapter that
  // dropped it once left the calendar without a now line.
  it("marks the Tokyo civil date's column as today, dated or recurring", () => {
    const agendas = ["2026-10-19", "2026-10-20", "2026-10-21"].map(
      (date) =>
        ({ date, occurrences: [], noClasses: false }) as unknown as DatedAgenda,
    );
    expect(
      agendaColumns(agendas, "2026-10-20").map((column) => column.isToday),
    ).toEqual([false, true, false]);
    const planning = {
      days: [1, 2, 3].map((day) => ({ day, blocks: [] })),
    } as unknown as PlanningWeek;
    // 2026-10-21 is a Wednesday; a quarter not running marks no weekday.
    expect(
      planningColumns(planning, "2026-10-21").map((column) => column.isToday),
    ).toEqual([false, false, true]);
    expect(planningColumns(planning).some((column) => column.isToday)).toBe(
      false,
    );
  });
});

describe("subscription status day formatting", () => {
  it("formats a timestamp or civil date in the display language, without shifting the day", () => {
    for (const value of ["2026-10-06T12:00:00Z", "2026-10-06"]) {
      expect(formatDay(value, "en"), value).toBe("6 Oct");
      expect(formatDay(value, "ja"), value).toBe("10月6日");
    }
    // Caller options pass through, as the quarter ranges ask for a weekday.
    expect(formatDay("2026-10-06", "en", { weekday: "short" })).toBe(
      "Tue 6 Oct",
    );
    // Malformed source dates come back unchanged instead of reaching Intl.
    expect(formatDay("not-a-date", "en")).toBe("not-a-date");
    expect(formatDay("10/06/2026", "en")).toBe("10/06/2026");
  });
});
