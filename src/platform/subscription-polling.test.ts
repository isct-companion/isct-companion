import { describe, expect, it, vi } from "vitest";
import { seedWorkspace } from "../core/model";
import {
  pollReferenceSubscriptions,
  recordSubscriptionPoll,
  SUBSCRIPTION_POLL_MS,
  subscriptionPollDue,
} from "./subscription-polling";

describe("reference subscription polling", () => {
  it("is due once per day", () => {
    const data = seedWorkspace();
    const now = Date.parse("2026-10-06T12:00:00Z");
    expect(subscriptionPollDue(data, now)).toBe(true);
    data.settings.subscriptionRefresh.lastCheckedAt = new Date(
      now - SUBSCRIPTION_POLL_MS + 1,
    ).toISOString();
    expect(subscriptionPollDue(data, now)).toBe(false);
    data.settings.subscriptionRefresh.lastCheckedAt = new Date(
      now - SUBSCRIPTION_POLL_MS,
    ).toISOString();
    expect(subscriptionPollDue(data, now)).toBe(true);
  });

  it("polls enabled GitHub datasets and carries their known hashes", async () => {
    const data = seedWorkspace();
    data.selectedTerm = "2026-2Q";
    data.academicProfile.admissionYear = 2025;
    data.subscriptions.push({
      id: "exam-subscription",
      kind: "exams",
      url: "https://example.com/index.json",
      enabled: true,
      indexHash: "old-custom-commit",
      remoteHash: "known-exam-hash",
      rev: 1,
      updatedAt: "2026-10-01T00:00:00Z",
      updatedBy: data.deviceId,
    });
    const refresh = vi.fn(async () => ({
      changed: 0,
      dataUrl: "https://example.com/data.json",
      remoteHash: "same-hash",
      sourceTs: "2026-10-06T00:00:00Z",
      notModified: true,
    }));
    const probe = vi.fn(async (url: string) => ({
      indexHash: url.includes("example.com")
        ? "new-custom-commit"
        : "public-commit",
    }));
    const results = await pollReferenceSubscriptions(data, probe, refresh);
    expect(probe).toHaveBeenCalledTimes(2);
    expect(refresh).toHaveBeenCalledTimes(4);
    expect(refresh).toHaveBeenCalledWith(
      "departments",
      expect.any(String),
      2025,
      "public-commit",
      undefined,
    );
    expect(refresh).toHaveBeenCalledWith(
      "exams",
      "https://example.com/index.json",
      2026,
      "new-custom-commit",
      "known-exam-hash",
    );
    expect(results).toHaveLength(4);
  });

  it("does not download the index or datasets when the repository hash is unchanged", async () => {
    const data = seedWorkspace();
    data.academicProfile.admissionYear = 2025;
    for (const kind of ["calendar", "departments", "important-dates", "exams"] as const) {
      data.subscriptions.push({
        id: `subscription:${kind}`,
        kind,
        url: "https://example.com/index.json",
        enabled: true,
        indexHash: "same-commit",
        rev: 1,
        updatedAt: "2026-10-01T00:00:00Z",
        updatedBy: data.deviceId,
      });
    }
    const probe = vi.fn(async () => ({ indexHash: "same-commit" }));
    const refresh = vi.fn();
    const results = await pollReferenceSubscriptions(data, probe, refresh);
    expect(probe).toHaveBeenCalledTimes(1);
    expect(refresh).not.toHaveBeenCalled();
    expect(results).toHaveLength(4);
    expect(results.every((item) => item.skipped)).toBe(true);
  });

  it("records hashes and the daily check as device state", () => {
    const data = seedWorkspace();
    recordSubscriptionPoll(
      data,
      [{
        kind: "calendar",
        url: "https://example.com/index.json",
        indexHash: "commit123",
        result: {
          changed: 0,
          dataUrl: "https://example.com/calendar.json",
          remoteHash: "abc123",
          sourceTs: "2026-10-06T00:00:00Z",
          notModified: true,
        },
      }],
      "2026-10-06T12:00:00Z",
    );
    expect(data.settings.subscriptionRefresh.lastCheckedAt).toBe(
      "2026-10-06T12:00:00Z",
    );
    expect(data.subscriptions[0]).toMatchObject({
      kind: "calendar",
      indexHash: "commit123",
      remoteHash: "abc123",
      sourceTs: "2026-10-06T00:00:00Z",
      fetchedAt: "2026-10-06T12:00:00Z",
    });
  });

  it("keeps a failed source due for retry and does not advance its hash", () => {
    const data = seedWorkspace();
    data.subscriptions.push({
      id: "subscription:calendar",
      kind: "calendar",
      url: "https://example.com/index.json",
      enabled: true,
      indexHash: "old-commit",
      rev: 1,
      updatedAt: "2026-10-01T00:00:00Z",
      updatedBy: data.deviceId,
    });
    recordSubscriptionPoll(
      data,
      [{
        kind: "calendar",
        url: "https://example.com/index.json",
        indexHash: "new-commit",
        error: "download failed",
      }],
      "2026-10-06T12:00:00Z",
    );
    expect(data.settings.subscriptionRefresh.lastCheckedAt).toBeUndefined();
    expect(data.subscriptions[0]).toMatchObject({
      indexHash: "old-commit",
      lastError: "download failed",
    });
  });
});
