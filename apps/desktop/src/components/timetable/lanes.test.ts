import { describe, expect, it } from "vitest";
import { blockLanes } from "./lanes";

const block = (key: string, startTime: string, endTime: string) => ({ key, startTime, endTime });

describe("timetable lanes", () => {
  it("gives every block the full width when nothing overlaps", () => {
    const lanes = blockLanes([block("a", "08:50", "10:30"), block("b", "10:45", "12:25")]);
    expect(lanes.get("a")).toEqual({ lane: 0, lanes: 1 });
    expect(lanes.get("b")).toEqual({ lane: 0, lanes: 1 });
  });

  it("puts conflicting blocks side by side", () => {
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
});
