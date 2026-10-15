// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

import type {
  SchoolSessionEvent,
  SchoolSessionSnapshot,
} from "./desktop-bridge";
import { schoolSessions } from "./school-session";

const initial: SchoolSessionSnapshot = {
  revision: 1,
  states: { science: "pending", tokyoTech: "not-configured" },
};

function bridge(snapshot: SchoolSessionSnapshot = initial) {
  let emit: (event: SchoolSessionEvent) => void = () => undefined;
  const order: string[] = [];
  const dispose = vi.fn();
  return {
    order,
    dispose,
    emit: (event: SchoolSessionEvent) => emit(event),
    onSchoolSession: vi.fn(
      async (callback: (event: SchoolSessionEvent) => void) => {
        order.push("listen");
        emit = callback;
        return dispose;
      },
    ),
    schoolSessionSnapshot: vi.fn(async () => {
      order.push("snapshot");
      return snapshot;
    }),
    schoolSessionLifecycle: vi.fn(async () => undefined),
  };
}

beforeEach(() => {
  schoolSessions.reset();
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value: "visible",
  });
});

describe("school session projection", () => {
  it("subscribes before reading the initial snapshot and maps pending to no badge", async () => {
    const native = bridge();
    const detach = schoolSessions.attach(native);
    await vi.waitFor(() =>
      expect(native.order).toEqual(["listen", "snapshot"]),
    );
    expect(schoolSessions.get("science")).toBeUndefined();
    expect(schoolSessions.get("tokyo-tech")).toBe("not-configured");
    expect(native.schoolSessionLifecycle).toHaveBeenCalledWith({
      kind: "launch",
      visible: true,
    });
    detach();
  });

  it("does not let an older snapshot overwrite an event received during the read", async () => {
    let resolveSnapshot!: (snapshot: SchoolSessionSnapshot) => void;
    const native = bridge();
    native.schoolSessionSnapshot.mockImplementation(
      () =>
        new Promise<SchoolSessionSnapshot>((resolve) => {
          resolveSnapshot = resolve;
        }),
    );
    schoolSessions.attach(native);
    await vi.waitFor(() => expect(native.onSchoolSession).toHaveBeenCalled());
    native.emit({ profile: "science", state: "active", revision: 3 });
    resolveSnapshot({
      revision: 2,
      states: { science: "failed", tokyoTech: "active" },
    });
    await vi.waitFor(() =>
      expect(schoolSessions.get("tokyo-tech")).toBe("active"),
    );
    expect(schoolSessions.get("science")).toBe("active");
  });

  it("ignores stale and duplicate revisions for each profile", async () => {
    const native = bridge({
      revision: 4,
      states: { science: "active", tokyoTech: "failed" },
    });
    schoolSessions.attach(native);
    await vi.waitFor(() =>
      expect(schoolSessions.get("science")).toBe("active"),
    );
    native.emit({ profile: "science", state: "failed", revision: 3 });
    native.emit({ profile: "science", state: "failed", revision: 4 });
    expect(schoolSessions.get("science")).toBe("active");
    native.emit({ profile: "science", state: "failed", revision: 5 });
    expect(schoolSessions.get("science")).toBe("failed");
  });

  it("reference-counts consumers and cleans up before reconnecting", async () => {
    const first = bridge();
    const detachFirst = schoolSessions.attach(first);
    const detachSecond = schoolSessions.attach(first);
    await vi.waitFor(() =>
      expect(first.onSchoolSession).toHaveBeenCalledOnce(),
    );
    detachFirst();
    expect(first.dispose).not.toHaveBeenCalled();
    detachSecond();
    expect(first.dispose).toHaveBeenCalledOnce();

    const second = bridge();
    schoolSessions.attach(second);
    await vi.waitFor(() =>
      expect(second.onSchoolSession).toHaveBeenCalledOnce(),
    );
  });

  it("rejects late events from an old bridge and accepts a new revision epoch", async () => {
    const first = bridge({
      revision: 8,
      states: { science: "active", tokyoTech: "failed" },
    });
    schoolSessions.attach(first);
    await vi.waitFor(() =>
      expect(schoolSessions.get("science")).toBe("active"),
    );

    const second = bridge({
      revision: 0,
      states: { science: "pending", tokyoTech: "not-configured" },
    });
    schoolSessions.attach(second);
    await vi.waitFor(() =>
      expect(schoolSessions.get("tokyo-tech")).toBe("not-configured"),
    );
    expect(schoolSessions.get("science")).toBeUndefined();

    first.emit({ profile: "science", state: "failed", revision: 99 });
    expect(schoolSessions.get("science")).toBeUndefined();
    second.emit({ profile: "science", state: "active", revision: 1 });
    expect(schoolSessions.get("science")).toBe("active");
  });

  it("preserves revisions across a brief reconnect to the same bridge", async () => {
    const native = bridge({
      revision: 5,
      states: { science: "failed", tokyoTech: "pending" },
    });
    const detach = schoolSessions.attach(native);
    await vi.waitFor(() =>
      expect(schoolSessions.get("science")).toBe("failed"),
    );
    native.emit({ profile: "science", state: "active", revision: 6 });
    detach();
    native.schoolSessionSnapshot.mockResolvedValue({
      revision: 0,
      states: { science: "failed", tokyoTech: "failed" },
    });

    schoolSessions.attach(native);
    await vi.waitFor(() =>
      expect(native.schoolSessionSnapshot).toHaveBeenCalledTimes(2),
    );
    expect(schoolSessions.get("science")).toBe("active");
  });

  it("forwards focus and visibility facts without browser-side throttling", async () => {
    const native = bridge();
    const detach = schoolSessions.attach(native);
    await vi.waitFor(() =>
      expect(native.schoolSessionLifecycle).toHaveBeenCalledWith({
        kind: "launch",
        visible: true,
      }),
    );
    window.dispatchEvent(new Event("focus"));
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: "hidden",
    });
    document.dispatchEvent(new Event("visibilitychange"));
    expect(native.schoolSessionLifecycle.mock.calls.slice(1)).toEqual([
      [{ kind: "focus", visible: true }],
      [{ kind: "focus", visible: false }],
    ]);
    detach();
  });
});
