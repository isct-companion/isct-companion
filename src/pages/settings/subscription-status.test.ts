import { describe, expect, it } from "vitest";
import { seedWorkspace } from "../../core/model";
import { formatDay, subscriptionState } from "./subscription-status";

describe("subscription status", () => {
  it("keeps malformed source dates from reaching Intl formatting", () => {
    expect(formatDay("not-a-date", "en")).toBe("not-a-date");
    expect(formatDay("10/06/2026", "en")).toBe("10/06/2026");
  });

  it("formats valid source timestamps using the selected display language", () => {
    expect(formatDay("2026-10-06T12:00:00Z", "en")).toContain("Oct");
    expect(formatDay("2026-10-06", "ja")).toContain("10月6日");
  });

  it("does not report malformed fetchedAt metadata as current", () => {
    const data = seedWorkspace();
    data.selectedTerm = "2026-3Q";
    data.subscriptions.push({
      id: "subscription:calendar",
      kind: "calendar",
      url: "https://example.com/index.json",
      enabled: true,
      year: 2026,
      fetchedAt: "not-a-date",
      rev: 1,
      updatedAt: "2026-10-01T00:00:00Z",
      updatedBy: data.deviceId,
    });
    expect(subscriptionState(data, "calendar")).toEqual({ state: "pending" });
  });
});
