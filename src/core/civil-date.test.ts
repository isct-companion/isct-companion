import { afterEach, expect, it, vi } from "vitest";
import {
  isCivilDate,
  parseIsoTimestamp,
  tokyoDate,
  formatTimestamp,
} from "./civil-date";
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

it("starts in the Japan quarter when the device is still in the previous month", () => {
  vi.useFakeTimers({ now: new Date("2026-09-30T15:00:00Z") });
  vi.spyOn(Date.prototype, "getMonth").mockReturnValue(8);
  expect(seedWorkspace().currentTerm).toBe("2026-3Q");
});

it("formats timestamps in JST regardless of device zone and rejects malformed values", () => {
  const value = "2026-12-31T15:01:00Z";
  expect(formatTimestamp(value, "en-GB")).toBe(
    new Intl.DateTimeFormat("en-GB", {
      year: "numeric",
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
      timeZone: "Asia/Tokyo",
      calendar: "gregory",
    }).format(new Date(value)),
  );
  expect(
    formatTimestamp(value, "en-GB", { timeZone: "America/Los_Angeles" }),
  ).toBe(formatTimestamp(value, "en-GB"));
  for (const invalid of [
    "Invalid Date",
    "2026-02-30T10:00:00Z",
    "10/14/2026",
    null,
  ])
    expect(formatTimestamp(invalid, "en-GB")).toBeUndefined();
});
