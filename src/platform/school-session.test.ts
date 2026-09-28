import { beforeEach, describe, expect, it, vi } from "vitest";

import type { SchoolSessionEvent } from "./desktop-bridge";
import { schoolSessions, signInAtLaunch } from "./school-session";

function bridge(saved = { science: true, tokyoTech: true }) {
  let emit: (event: SchoolSessionEvent) => void = () => undefined;
  const calls: string[] = [];
  return {
    calls,
    emit: (event: SchoolSessionEvent) => emit(event),
    schoolCredentialStatus: vi.fn(async () => ({
      science: {
        saved: saved.science,
        username: "",
        hasOtp: true,
        hasMatrix: false,
      },
      tokyoTech: {
        saved: saved.tokyoTech,
        username: "",
        hasOtp: false,
        hasMatrix: true,
      },
    })),
    ensureSchoolSession: vi.fn(async (profile: string) => {
      calls.push(profile);
      return true;
    }),
    onSchoolSession: vi.fn((callback: (event: SchoolSessionEvent) => void) => {
      emit = callback;
      return () => undefined;
    }),
  };
}

beforeEach(() => schoolSessions.reset());

describe("launch sign-in", () => {
  it("signs accounts in one at a time, waiting for each result", async () => {
    const native = bridge();
    const done = signInAtLaunch(native, 1_000);
    await vi.waitFor(() => expect(native.calls).toEqual(["science"]));
    native.emit({ profile: "science", state: "active" });
    await vi.waitFor(() =>
      expect(native.calls).toEqual(["science", "tokyo-tech"]),
    );
    native.emit({ profile: "tokyo-tech", state: "failed" });
    await done;
    expect(schoolSessions.get("science")).toBe("active");
    expect(schoolSessions.get("tokyo-tech")).toBe("failed");
  });

  it("marks accounts without credentials as not set up", async () => {
    const native = bridge({ science: false, tokyoTech: false });
    await signInAtLaunch(native, 1_000);
    expect(native.ensureSchoolSession).not.toHaveBeenCalled();
    expect(schoolSessions.get("science")).toBe("not-configured");
    expect(schoolSessions.get("tokyo-tech")).toBe("not-configured");
  });

  it("moves on when a sign-in never reports back", async () => {
    const native = bridge();
    await signInAtLaunch(native, 20);
    expect(native.calls).toEqual(["science", "tokyo-tech"]);
    expect(schoolSessions.get("science")).toBeUndefined();
  });

  it("does not wait when native skips the check", async () => {
    const native = bridge();
    native.ensureSchoolSession.mockResolvedValue(false);
    await signInAtLaunch(native, 60_000);
    expect(native.ensureSchoolSession).toHaveBeenCalledTimes(2);
  });

  it("keeps later renewals flowing into the store", async () => {
    const native = bridge({ science: false, tokyoTech: false });
    await signInAtLaunch(native, 1_000);
    native.emit({ profile: "science", state: "active" });
    expect(schoolSessions.get("science")).toBe("active");
  });
});
