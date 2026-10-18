import { afterEach, describe, expect, it, vi } from "vitest";

import { blockLanes } from "@app/components/timetable/lanes";
import { DEFAULT_TEACHING_PERIODS } from "@app/generated/persistence";
import { termForDate } from "@app/lib/academic";
import {
  formatTimestamp,
  isCivilDate,
  parseIsoTimestamp,
  tokyoDate,
} from "@app/lib/civil-date";
import { seedWorkspace } from "@app/lib/model";
import {
  classSlots,
  timetableBounds,
  timetablePosition,
} from "@app/lib/timetable-layout";
import { formatDay } from "@app/pages/settings/subscription-status";

/**
 * Dates, Japan academic quarters, timetable geometry and the day formatting the
 * subscription status view renders. Everything here is pure logic, so the suite
 * runs in plain node.
 */

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

const block = (key: string, startTime: string, endTime: string) => ({
  key,
  startTime,
  endTime,
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

describe("civil date validation", () => {
  it("accepts zero-padded Gregorian dates, including a leap day", () => {
    expect(isCivilDate("2026-04-13")).toBe(true);
    expect(isCivilDate("2028-02-29")).toBe(true);
  });

  it("rejects localized, unpadded, impossible and non-string civil dates", () => {
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
});

describe("ISO timestamp parsing", () => {
  it("parses an explicit offset or Z timestamp to the instant it names", () => {
    expect(parseIsoTimestamp("2026-10-13T12:00:00+09:00")?.toISOString()).toBe(
      "2026-10-13T03:00:00.000Z",
    );
    expect(parseIsoTimestamp("2026-10-13T03:00:00Z")?.toISOString()).toBe(
      "2026-10-13T03:00:00.000Z",
    );
  });

  it("rejects locale text, bare dates, overflow days and out-of-range clock hours", () => {
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

describe("Japan academic quarters", () => {
  it("selects the quarter and academic year from the Japan-time date", () => {
    const cases: [instant: string, term: string][] = [
      ["2026-03-31T14:59:00Z", "2025-4Q"], // 31 March, 23:59 JST
      ["2026-03-31T15:00:00Z", "2026-1Q"], // 1 April, 00:00 JST
      ["2026-05-31T15:00:00Z", "2026-2Q"], // 1 June
      ["2026-09-30T15:00:00Z", "2026-3Q"], // 1 October
      ["2026-11-30T15:00:00Z", "2026-4Q"], // 1 December
      ["2026-12-31T15:00:00Z", "2026-4Q"], // 1 January, still the 2026 academic year
    ];
    for (const [instant, term] of cases)
      expect(termForDate(new Date(instant))).toBe(term);
  });

  it("starts a fresh workspace in the Japan quarter even when the device is still in the previous month", () => {
    // 2026-09-30T15:00:00Z is already 1 October in Japan.
    vi.useFakeTimers({ now: new Date("2026-09-30T15:00:00Z") });
    expect(seedWorkspace().currentTerm).toBe("2026-3Q");
  });
});

describe("machine dates under a non-Gregorian device locale", () => {
  it("keeps the Japan calendar date Gregorian and its digits Latin", () => {
    installNonGregorianDefaultLocale();
    expect(tokyoDate(new Date("2026-04-12T14:59:00Z"))).toBe("2026-04-12");
    expect(tokyoDate(new Date("2026-04-12T15:01:00Z"))).toBe("2026-04-13");
  });

  it("picks the academic quarter from the Gregorian Japan date", () => {
    installNonGregorianDefaultLocale();
    expect(termForDate(new Date("2026-03-31T15:00:00Z"))).toBe("2026-1Q");
  });
});

describe("JST timestamp display", () => {
  it("renders a timestamp in Japan time as the requested locale would", () => {
    expect(formatTimestamp("2026-12-31T15:01:00Z", "en-GB")).toBe(
      "1 Jan 2027, 00:01",
    );
    for (const locale of ["zh", "ja"]) {
      const text = formatTimestamp("2026-12-31T15:01:00Z", locale)!;
      expect(text).toContain("2027");
      expect(text).toContain("00:01");
      expect(text).not.toContain("2026");
    }
  });

  it("ignores a caller-supplied time zone and always renders Japan time", () => {
    expect(
      formatTimestamp("2026-12-31T15:01:00Z", "en-GB", {
        timeZone: "America/Los_Angeles",
      }),
    ).toBe("1 Jan 2027, 00:01");
  });

  it("returns undefined for malformed legacy timestamps instead of throwing", () => {
    for (const invalid of [
      "Invalid Date",
      "2026-02-30T10:00:00Z",
      "10/14/2026",
      null,
    ])
      expect(formatTimestamp(invalid, "en-GB")).toBeUndefined();
  });
});

describe("timetable class slots", () => {
  it("labels the configured periods in order and keeps the 11–12 evening block last", () => {
    const slots = classSlots(DEFAULT_TEACHING_PERIODS);
    expect(slots.map((slot) => slot.label)).toEqual([
      "1-2",
      "3-4",
      "5-6",
      "7-8",
      "9-10",
      "11-12",
    ]);
    expect(slots.at(-1)).toEqual({
      label: "11-12",
      periods: [11, 12],
      start: "19:05",
      end: "20:45",
    });
  });
});

describe("timetable bounds and position", () => {
  it("spans the configured periods when a block stays inside them", () => {
    expect(timetableBounds(DEFAULT_TEACHING_PERIODS)).toEqual({
      start: 530,
      end: 1245,
    });
    expect(
      timetableBounds(DEFAULT_TEACHING_PERIODS, [
        { startTime: "15:25", endTime: "17:05" },
      ]),
    ).toEqual({ start: 530, end: 1245 });
  });

  it("expands around explicit custom course times instead of clipping them", () => {
    const bounds = timetableBounds(DEFAULT_TEACHING_PERIODS, [
      { startTime: "07:30", endTime: "22:00" },
    ]);
    expect(bounds).toEqual({ start: 450, end: 1320 });
    expect(timetablePosition("07:30", "22:00", bounds)).toEqual({
      topPercent: 0,
      heightPercent: 100,
    });
  });
});

describe("overlap lane packing", () => {
  it("gives every block the full width when nothing overlaps", () => {
    const lanes = blockLanes([
      block("a", "08:50", "10:30"),
      block("b", "10:45", "12:25"),
    ]);
    expect(lanes.get("a")).toEqual({ lane: 0, lanes: 1 });
    expect(lanes.get("b")).toEqual({ lane: 0, lanes: 1 });
  });

  it("puts conflicting blocks side by side and leaves a later block alone", () => {
    const lanes = blockLanes([
      block("a", "10:45", "12:25"),
      block("b", "10:45", "12:25"),
      block("c", "11:00", "11:30"),
      block("d", "13:30", "15:10"),
    ]);
    expect([lanes.get("a"), lanes.get("b"), lanes.get("c")]).toEqual([
      { lane: 0, lanes: 3 },
      { lane: 1, lanes: 3 },
      { lane: 2, lanes: 3 },
    ]);
    expect(lanes.get("d")).toEqual({ lane: 0, lanes: 1 });
  });

  it("reuses a lane once its block has ended inside the same cluster", () => {
    const lanes = blockLanes([
      block("long", "08:50", "12:25"),
      block("first", "08:50", "10:30"),
      block("second", "10:45", "12:25"),
    ]);
    expect(lanes.get("long")).toEqual({ lane: 0, lanes: 2 });
    expect(lanes.get("first")).toEqual({ lane: 1, lanes: 2 });
    expect(lanes.get("second")).toEqual({ lane: 1, lanes: 2 });
  });

  it("treats back-to-back blocks as non-overlapping", () => {
    const lanes = blockLanes([
      block("a", "10:45", "12:25"),
      block("b", "12:25", "14:00"),
    ]);
    expect(lanes.get("a")).toEqual({ lane: 0, lanes: 1 });
    expect(lanes.get("b")).toEqual({ lane: 0, lanes: 1 });
  });
});

describe("subscription status day formatting", () => {
  it("formats a source timestamp using the selected display language", () => {
    expect(formatDay("2026-10-06T12:00:00Z", "en")).toBe("6 Oct");
    expect(formatDay("2026-10-06T12:00:00Z", "ja")).toBe("10月6日");
  });

  it("formats a civil date without shifting it across the Japan midnight", () => {
    expect(formatDay("2026-10-06", "en")).toBe("6 Oct");
    expect(formatDay("2026-10-06", "ja")).toBe("10月6日");
  });

  it("passes caller options through, as the quarter ranges ask for a weekday", () => {
    expect(formatDay("2026-10-06", "en", { weekday: "short" })).toBe(
      "Tue 6 Oct",
    );
  });

  it("returns malformed source dates unchanged instead of handing them to Intl", () => {
    expect(formatDay("not-a-date", "en")).toBe("not-a-date");
    expect(formatDay("10/06/2026", "en")).toBe("10/06/2026");
  });
});
