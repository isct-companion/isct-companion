import { describe, expect, it } from "vitest";
import {
  CloudConflictError,
  CloudTransportError,
  GOOGLE_DRIVE_CLIENT_ID,
  createCloudAdapter,
  resolveCloudClientId,
  type CloudBackupTransport,
  type OAuthSession,
} from "./cloud";

class MemoryTransport implements CloudBackupTransport {
  bytes: Uint8Array | null = null;
  reads = 0;
  writes = 0;

  async read() {
    this.reads += 1;
    return this.bytes ? Uint8Array.from(this.bytes) : null;
  }

  async write(bytes: Uint8Array) {
    this.writes += 1;
    this.bytes = Uint8Array.from(bytes);
  }
}

const oauth: OAuthSession = {
  connect: async () => undefined,
  requireAccessToken: async () => "test-token",
};

const exportDocument = (marker = "one") =>
  JSON.stringify({
    schemaVersion: 6,
    exportedAt: "2026-10-06T00:00:00.000Z",
    kind: "IsctCompanion.local-backup",
    deviceId: "local-backup",
    localNotes: [],
    tables: {
      manual_courses: [],
      manual_memberships: [],
      course_overrides: [],
      plan_items: [],
      personal_events: [],
      calendar_exceptions: [],
      assignment_marks: [],
      intensive_schedules: [],
      plans: [],
      schedule_overrides: [],
      subscriptions: [],
      settings: [{ key: "marker", value: marker }],
    },
  });

const adapter = (transport: MemoryTransport, deviceId: string) =>
  createCloudAdapter("google-drive", "test-client", {
    oauth,
    transport,
    deviceId,
    now: () => new Date("2026-10-06T01:02:03.000Z"),
  });

describe("plaintext cloud workspace", () => {
  it("uses the registered Google desktop client unless development overrides it", () => {
    expect(resolveCloudClientId("google-drive", "")).toBe(
      GOOGLE_DRIVE_CLIENT_ID,
    );
    expect(resolveCloudClientId("google-drive", " local-override ")).toBe(
      "local-override",
    );
    expect(resolveCloudClientId("onedrive", "")).toBe("");
  });

  it("writes one readable JSON file and pulls its export document", async () => {
    const transport = new MemoryTransport();
    const cloud = adapter(transport, "device-a");

    await cloud.push(exportDocument());

    expect(transport.writes).toBe(1);
    const stored = JSON.parse(new TextDecoder().decode(transport.bytes!));
    expect(stored).toMatchObject({
      format: "IsctCompanion",
      formatVersion: 1,
      revision: 1,
      updatedAt: "2026-10-06T01:02:03.000Z",
      updatedBy: "device-a",
      document: { schemaVersion: 6, kind: "IsctCompanion.local-backup" },
    });
    expect(stored).not.toHaveProperty("ciphertext");
    expect(JSON.parse(await cloud.pull()).schemaVersion).toBe(6);
  });

  it("rejects a blind push over an existing cloud workspace", async () => {
    const transport = new MemoryTransport();
    await adapter(transport, "device-a").push(exportDocument());

    await expect(
      adapter(transport, "device-b").push(exportDocument("two")),
    ).rejects.toBeInstanceOf(CloudConflictError);
  });

  it("rejects a push when the remote revision changed after pull", async () => {
    const transport = new MemoryTransport();
    const first = adapter(transport, "device-a");
    const stale = adapter(transport, "device-b");
    await first.push(exportDocument());
    await stale.pull();
    await first.push(exportDocument("two"));

    await expect(stale.push(exportDocument("three"))).rejects.toBeInstanceOf(
      CloudConflictError,
    );
  });

  it("keeps local data when the cloud file is malformed", async () => {
    const transport = new MemoryTransport();
    transport.bytes = new TextEncoder().encode(
      JSON.stringify({ format: "IsctCompanion", revision: 1 }),
    );

    await expect(adapter(transport, "device-a").pull()).rejects.toMatchObject({
      name: CloudTransportError.name,
      status: 409,
      retryable: false,
    });
  });
});
