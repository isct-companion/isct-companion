// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { SchoolSessionEvent } from "./desktop-bridge";
import {
  checkSessionsOnFocus,
  schoolSessions,
  signInAtLaunch,
} from "./school-session";

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
  it("coalesces duplicate launch effects into one session sequence", async () => {
    const native = bridge();
    const first = signInAtLaunch(native, 1_000);
    const duplicate = signInAtLaunch(native, 1_000);
    expect(duplicate).toBe(first);
    await vi.waitFor(() =>
      expect(native.calls).toEqual(["science", "tokyo-tech"]),
    );
    native.emit({ profile: "science", state: "active" });
    native.emit({ profile: "tokyo-tech", state: "active" });
    await Promise.all([first, duplicate]);
    expect(native.ensureSchoolSession).toHaveBeenCalledTimes(2);
  });

  it("submits both accounts to the native queue and waits for both results", async () => {
    const native = bridge();
    const done = signInAtLaunch(native, 1_000);
    await vi.waitFor(() =>
      expect(native.calls).toEqual(["science", "tokyo-tech"]),
    );
    native.emit({ profile: "science", state: "active" });
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

  it("does not retry a request the native queue discarded", async () => {
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

describe("focus re-check", () => {
  it("moves on when an already signed-in account reports active again", async () => {
    const native = bridge();
    schoolSessions.attach(native);
    native.emit({ profile: "science", state: "active" });
    const done = signInAtLaunch(native, 60_000, true);
    await vi.waitFor(() =>
      expect(native.calls).toEqual(["science", "tokyo-tech"]),
    );
    expect(schoolSessions.get("science")).toBe("active"); // quiet: badge kept
    native.emit({ profile: "science", state: "active" });
    native.emit({ profile: "tokyo-tech", state: "active" });
    await done;
  });

  it("checks on focus at most once per interval", async () => {
    const native = bridge({ science: true, tokyoTech: false });
    let clock = 0;
    const stop = checkSessionsOnFocus(native, 1_000, () => clock);
    window.dispatchEvent(new Event("focus"));
    expect(native.ensureSchoolSession).not.toHaveBeenCalled(); // too soon after launch
    clock = 1_500;
    window.dispatchEvent(new Event("focus"));
    await vi.waitFor(() => expect(native.ensureSchoolSession).toHaveBeenCalledTimes(1));
    native.emit({ profile: "science", state: "active" });
    clock = 2_000;
    window.dispatchEvent(new Event("focus"));
    await Promise.resolve();
    expect(native.ensureSchoolSession).toHaveBeenCalledTimes(1);
    stop();
  });
});
