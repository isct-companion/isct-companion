import { describe, expect, it } from "vitest";
import { formatDay } from "./subscription-status";

describe("subscription status", () => {
  it("keeps malformed source dates from reaching Intl formatting", () => {
    expect(formatDay("not-a-date", "en")).toBe("not-a-date");
    expect(formatDay("10/06/2026", "en")).toBe("10/06/2026");
  });

  it("formats valid source timestamps using the selected display language", () => {
    expect(formatDay("2026-10-06T12:00:00Z", "en")).toContain("Oct");
    expect(formatDay("2026-10-06", "ja")).toContain("10月6日");
  });
});
