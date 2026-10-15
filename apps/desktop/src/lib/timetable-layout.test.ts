import { describe, expect, it } from "vitest";
import { DEFAULT_TEACHING_PERIODS } from "../generated/persistence";
import {
  classSlots,
  timetableBounds,
  timetablePosition,
} from "./timetable-layout";

describe("timetable layout", () => {
  it("includes the official 11–12 evening block", () => {
    expect(classSlots(DEFAULT_TEACHING_PERIODS).at(-1)).toEqual({
      label: "11-12",
      periods: [11, 12],
      start: "19:05",
      end: "20:45",
    });
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
