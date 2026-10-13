import { describe, expect, it, vi } from "vitest";
import { seedWorkspace } from "../core/model";
import {
  pollReferenceSubscriptions,
  referenceCachePresent,
  recordSubscriptionPoll,
  SUBSCRIPTION_POLL_MS,
  subscriptionPollDue,
} from "./subscription-polling";

describe("reference subscription polling", () => {
  it("requires valid dates on every quarter boundary", () => {
    const data = seedWorkspace();
    for (const quarter of ["1Q", "2Q", "3Q", "4Q"] as const)
      for (const type of ["quarter-start", "quarter-end"] as const)
        data.academicCalendar.push({
          id: `calendar:2026:${type}:${quarter}`,
          academicYear: 2026,
          date: "2026-04-01",
          type,
          quarter,
          label: { ja: quarter },
          rev: 1,
          updatedAt: "2026-10-01T00:00:00Z",
          updatedBy: data.deviceId,
        });
    expect(referenceCachePresent(data, "calendar", 2026)).toBe(true);
    data.academicCalendar[0].date = "2026-02-30";
    expect(referenceCachePresent(data, "calendar", 2026)).toBe(false);
  });

  it("is due once per day", () => {
    const data = seedWorkspace();
    for (const kind of [
      "calendar",
      "departments",
      "important-dates",
      "exams",
    ] as const)
      data.subscriptions.push({
        id: `subscription:${kind}`,
        kind,
        url: "https://example.com/index.json",
        enabled: false,
        rev: 1,
        updatedAt: "2026-10-01T00:00:00Z",
        updatedBy: data.deviceId,
      });
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

  it("is immediately due when calendar metadata outlives its quarter cache", () => {
    const data = seedWorkspace();
    data.selectedTerm = "2026-3Q";
    data.subscriptions.push({
      id: "subscription:calendar",
      kind: "calendar",
      url: "https://example.com/index.json",
      enabled: true,
      year: 2026,
      indexHash: "same-commit",
      fetchedAt: "2026-10-06T11:00:00Z",
      rev: 1,
      updatedAt: "2026-10-06T11:00:00Z",
      updatedBy: data.deviceId,
    });
    data.settings.subscriptionRefresh.lastCheckedAt = "2026-10-06T11:00:00Z";
    expect(subscriptionPollDue(data, Date.parse("2026-10-06T12:00:00Z"))).toBe(
      true,
    );
  });

  it("does not let a localized fetchedAt value suppress a retry", () => {
    const data = seedWorkspace();
    data.academicProfile.admissionYear = 2026;
    data.subscriptions.push({
      id: "subscription:departments",
      kind: "departments",
      url: "https://example.com/index.json",
      enabled: true,
      year: 2026,
      indexHash: "same-commit",
      fetchedAt: "10/06/2026 12:00",
      rev: 1,
      updatedAt: "2026-10-06T11:00:00Z",
      updatedBy: data.deviceId,
    });
    for (const kind of ["calendar", "important-dates", "exams"] as const)
      data.subscriptions.push({
        id: `subscription:${kind}`,
        kind,
        url: "https://example.com/index.json",
        enabled: false,
        rev: 1,
        updatedAt: "2026-10-06T11:00:00Z",
        updatedBy: data.deviceId,
      });
    data.settings.subscriptionRefresh.lastCheckedAt = "2026-10-06T11:00:00Z";
    expect(subscriptionPollDue(data, Date.parse("2026-10-06T12:00:00Z"))).toBe(
      true,
    );
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
      year: 2026,
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
    data.selectedTerm = "2026-3Q";
    data.academicProfile.admissionYear = 2025;
    for (const kind of [
      "calendar",
      "departments",
      "important-dates",
      "exams",
    ] as const) {
      data.subscriptions.push({
        id: `subscription:${kind}`,
        kind,
        url: "https://example.com/index.json",
        enabled: true,
        indexHash: "same-commit",
        year: kind === "departments" ? 2025 : 2026,
        fetchedAt: "2026-10-01T00:00:00Z",
        rev: 1,
        updatedAt: "2026-10-01T00:00:00Z",
        updatedBy: data.deviceId,
      });
    }
    for (const quarter of ["1Q", "2Q", "3Q", "4Q"] as const)
      for (const type of ["quarter-start", "quarter-end"] as const)
        data.academicCalendar.push({
          id: `calendar:2026:${type}:${quarter}`,
          academicYear: 2026,
          date: "2026-04-01",
          type,
          quarter,
          label: { ja: quarter },
          rev: 1,
          updatedAt: "2026-10-01T00:00:00Z",
          updatedBy: data.deviceId,
        });
    const probe = vi.fn(async () => ({ indexHash: "same-commit" }));
    const refresh = vi.fn();
    const results = await pollReferenceSubscriptions(data, probe, refresh);
    expect(probe).toHaveBeenCalledTimes(1);
    expect(refresh).not.toHaveBeenCalled();
    expect(results).toHaveLength(4);
    expect(results.every((item) => item.skipped)).toBe(true);
  });

  it("repairs missing quarter rows even when the index and source hashes match", async () => {
    const data = seedWorkspace();
    data.selectedTerm = "2026-3Q";
    data.subscriptions.push({
      id: "subscription:calendar",
      kind: "calendar",
      url: "https://example.com/index.json",
      enabled: true,
      year: 2026,
      indexHash: "same-commit",
      remoteHash: "known-calendar-hash",
      fetchedAt: "2026-10-01T00:00:00Z",
      rev: 1,
      updatedAt: "2026-10-01T00:00:00Z",
      updatedBy: data.deviceId,
    });
    for (const kind of ["departments", "important-dates", "exams"] as const)
      data.subscriptions.push({
        id: `subscription:${kind}`,
        kind,
        url: "https://example.com/index.json",
        enabled: false,
        rev: 1,
        updatedAt: "2026-10-01T00:00:00Z",
        updatedBy: data.deviceId,
      });
    const probe = vi.fn(async () => ({ indexHash: "same-commit" }));
    const refresh = vi.fn(async () => ({
      changed: 8,
      dataUrl: "https://example.com/calendar.json",
      remoteHash: "known-calendar-hash",
      sourceTs: "2026-10-01T00:00:00Z",
      notModified: false,
    }));
    await pollReferenceSubscriptions(data, probe, refresh);
    expect(refresh).toHaveBeenCalledWith(
      "calendar",
      "https://example.com/index.json",
      2026,
      "same-commit",
      undefined,
    );
  });

  it("fetches the newly selected year even when the repository hash is unchanged", async () => {
    const data = seedWorkspace();
    data.selectedTerm = "2026-3Q";
    data.academicProfile.admissionYear = 2024;
    data.subscriptions.push({
      id: "subscription:departments",
      kind: "departments",
      url: "https://example.com/index.json",
      enabled: true,
      indexHash: "same-commit",
      remoteHash: "guide-2025-hash",
      year: 2025,
      rev: 1,
      updatedAt: "2026-10-01T00:00:00Z",
      updatedBy: data.deviceId,
    });
    for (const kind of ["calendar", "important-dates", "exams"] as const)
      data.subscriptions.push({
        id: `subscription:${kind}`,
        kind,
        url: "https://example.com/index.json",
        enabled: false,
        rev: 1,
        updatedAt: "2026-10-01T00:00:00Z",
        updatedBy: data.deviceId,
      });
    const probe = vi.fn(async () => ({ indexHash: "same-commit" }));
    const refresh = vi.fn(async () => ({
      changed: 3,
      dataUrl: "https://example.com/departments/2024.json",
      remoteHash: "guide-2024-hash",
      sourceTs: "2026-10-06T00:00:00Z",
      notModified: false,
    }));
    const results = await pollReferenceSubscriptions(data, probe, refresh);
    expect(refresh).toHaveBeenCalledWith(
      "departments",
      "https://example.com/index.json",
      2024,
      "same-commit",
      undefined,
    );
    recordSubscriptionPoll(data, results, "2026-10-06T12:00:00Z");
    expect(data.subscriptions[0]).toMatchObject({
      year: 2024,
      remoteHash: "guide-2024-hash",
    });
  });

  it("records hashes and the daily check as device state", () => {
    const data = seedWorkspace();
    recordSubscriptionPoll(
      data,
      [
        {
          kind: "calendar",
          url: "https://example.com/index.json",
          year: 2026,
          indexHash: "commit123",
          result: {
            changed: 0,
            dataUrl: "https://example.com/calendar.json",
            remoteHash: "abc123",
            sourceTs: "2026-10-06T00:00:00Z",
            notModified: true,
          },
        },
      ],
      "2026-10-06T12:00:00Z",
    );
    expect(data.settings.subscriptionRefresh.lastCheckedAt).toBe(
      "2026-10-06T12:00:00Z",
    );
    expect(data.subscriptions[0]).toMatchObject({
      kind: "calendar",
      year: 2026,
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
      [
        {
          kind: "calendar",
          url: "https://example.com/index.json",
          year: 2026,
          indexHash: "new-commit",
          error: "download failed",
        },
      ],
      "2026-10-06T12:00:00Z",
    );
    expect(data.settings.subscriptionRefresh.lastCheckedAt).toBeUndefined();
    expect(data.subscriptions[0]).toMatchObject({
      indexHash: "old-commit",
      lastError: "download failed",
    });
  });
});
