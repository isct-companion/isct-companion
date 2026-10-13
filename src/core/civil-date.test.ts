import { afterEach, expect, it, vi } from "vitest";
import { isCivilDate, parseIsoTimestamp, tokyoDate } from "./civil-date";
import { academicTermForDate } from "./academic-calendar";
import { termForDate } from "./academic";
import { seedWorkspace } from "./model";

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

it("rejects localized, unpadded and overflow civil dates", () => {
  for (const value of ["04/13/2026", "2026-4-13", "2026-02-30", "", null])
    expect(isCivilDate(value)).toBe(false);
  expect(isCivilDate("2028-02-29")).toBe(true);
});

it("parses offset timestamps without accepting locale text or overflow dates", () => {
  expect(parseIsoTimestamp("2026-10-13T12:00:00+09:00")?.toISOString()).toBe(
    "2026-10-13T03:00:00.000Z",
  );
  for (const value of [
    "10/13/2026",
    "2026-10-13",
    "2026-02-30T00:00:00Z",
    "2026-10-13T24:00:00Z",
  ])
    expect(parseIsoTimestamp(value)).toBeUndefined();
});

it("pins Gregorian dates and Latin digits despite different calendar defaults", () => {
  const Formatter = Intl.DateTimeFormat;
  vi.spyOn(Intl, "DateTimeFormat").mockImplementation(function (
    locale,
    options,
  ) {
    return new Formatter(locale, {
      calendar: "buddhist",
      numberingSystem: "arab",
      ...options,
    });
  } as typeof Intl.DateTimeFormat);
  expect(tokyoDate(new Date("2026-04-12T15:01:00Z"))).toBe("2026-04-13");
  expect(termForDate(new Date("2026-03-31T15:00:00Z"))).toBe("2026-1Q");
});

it("selects the configured quarter without comparing localized display strings", () => {
  vi.spyOn(Intl.DateTimeFormat.prototype, "format", "get").mockReturnValue(
    () => "10/13/2026",
  );
  expect(
    academicTermForDate(new Date("2026-10-13T03:00:00Z"), {
      name: "Study",
      gradePath: [],
      years: [
        {
          academicYear: 2026,
          quarterStarts: {
            "1Q": "2026-04-03",
            "2Q": "2026-06-10",
            "3Q": "2026-09-28",
            "4Q": "2026-12-03",
          },
        },
      ],
    }),
  ).toBe("2026-3Q");
});

it("starts in the Japan quarter when the device is still in the previous month", () => {
  vi.useFakeTimers({ now: new Date("2026-09-30T15:00:00Z") });
  vi.spyOn(Date.prototype, "getMonth").mockReturnValue(8);
  expect(seedWorkspace().currentTerm).toBe("2026-3Q");
});
