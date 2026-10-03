// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { can, performAction, requireAction } from "./actions";
import { resolveCapabilities, type CapabilityRuntime } from "./capabilities";
import { createTauriDesktopBridge } from "./desktop-bridge";

describe("public action policy", () => {
  it.each(["view", "change-local-preference", "export-data"] as const)(
    "keeps %s available in browser preview",
    (action) => expect(can(action, "browser-preview").allowed).toBe(true),
  );

  it.each(["edit-academic-data", "import-school", "push-cloud"] as const)(
    "keeps %s native-only when it changes core data",
    (action) => {
      expect(can(action, "browser-preview")).toMatchObject({
        allowed: false,
      });
      expect(() => requireAction(action, "browser-preview")).toThrow(
        "read-only",
      );
      expect(can(action, "native-desktop").allowed).toBe(true);
    },
  );

  it.each([
    "edit-calendar-advanced",
    "customize-school-site",
    "configure-unattended-schedule",
    "resolve-cloud-manually",
  ] as const)("keeps %s desktop-only", (action) => {
    expect(can(action, "native-mobile").allowed).toBe(false);
    expect(can(action, "browser-preview").allowed).toBe(false);
    expect(can(action, "native-desktop").allowed).toBe(true);
  });

  it("does not run blocked operations", async () => {
    const operation = vi.fn();
    await expect(
      performAction("import-school", "browser-preview", operation),
    ).rejects.toThrow("desktop app");
    expect(operation).not.toHaveBeenCalled();
  });
});

const runtime = (
  overrides: Partial<CapabilityRuntime> = {},
): CapabilityRuntime => ({
  native: {
    schoolImport: true,
    schoolWeb: true,
    notifications: true,
    backgroundSchedule: true,
    cloudSync: true,
  },
  school: { status: "not-configured" },
  background: { scheduler: "off" },
  noBackendPush: true,
  ...overrides,
});

describe("runtime capabilities", () => {
  it("reports a validated native school session as ready", () => {
    const capabilities = resolveCapabilities(
      "native-desktop",
      false,
      "granted",
      runtime({
        school: { status: "connected", lastResult: "fresh" },
        background: { scheduler: "active" },
      }),
    );
    expect(capabilities["school-web-session"].readiness).toBe("ready");
    expect(capabilities["background-sync"].readiness).toBe("ready");
    expect(capabilities["site-customization"].support).toBe("supported");
  });

  it("does not claim native adapters exist until runtime acknowledges them", () => {
    const capabilities = resolveCapabilities(
      "native-desktop",
      false,
      "default",
      runtime({ native: undefined }),
    );
    expect(capabilities["school-web-session"].support).toBe("unknown");
    expect(capabilities["background-sync"].support).toBe("unknown");
    expect(capabilities["site-customization"].support).toBe("unknown");
  });

  it("describes browser preview without PWA capabilities", () => {
    const capabilities = resolveCapabilities(
      "browser-preview",
      false,
      "denied",
      runtime(),
    );
    expect(capabilities["school-web-session"].support).toBe("unsupported");
    expect(capabilities["background-sync"].support).toBe("unsupported");
    expect(capabilities["site-customization"].support).toBe("unsupported");
  });
});

describe("Tauri bridge contract", () => {
  it("maps credential, session, and webview calls to narrow commands", async () => {
    const invoke = vi.fn(async (command: string) => {
      if (command === "save_school_credentials") return true;
      if (command === "ensure_school_session") return false;
      if (command === "navigate_school_web") return true;
      if (command === "set_school_web_zoom") return true;
      if (command === "open_school_site")
        return {
          site: "science-portal",
          url: "https://example.test",
          phase: "pageLoaded",
        };
      throw new Error(command);
    });
    const bridge = createTauriDesktopBridge(invoke, vi.fn(), "native-desktop");
    const credentials = {
      username: "student",
      password: "secret",
      otpSecret: "OTP",
      matrix: "A".repeat(70),
    };
    await bridge.saveSchoolCredentials("science", credentials);
    await expect(bridge.ensureSchoolSession?.("science")).resolves.toBe(false);
    await bridge.openSchoolSite("science-portal", "body{}", "window.x=1", {
      x: 1,
      y: 2,
      width: 3,
      height: 4,
    });
    await bridge.navigateSchoolWeb?.(
      "science-portal",
      "https://isct.ex-tic.com/idm/user/portal/",
    );
    await bridge.setSchoolWebZoom?.("science-portal", 1.2);
    expect(invoke).toHaveBeenNthCalledWith(1, "save_school_credentials", {
      profile: "science",
      credentials,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "ensure_school_session", {
      profile: "science",
    });
    expect(invoke).toHaveBeenNthCalledWith(3, "open_school_site", {
      site: "science-portal",
      css: "body{}",
      js: "window.x=1",
      bounds: { x: 1, y: 2, width: 3, height: 4 },
      targetUrl: null,
    });
    expect(invoke).toHaveBeenNthCalledWith(4, "navigate_school_web", {
      site: "science-portal",
      url: "https://isct.ex-tic.com/idm/user/portal/",
    });
    expect(invoke).toHaveBeenNthCalledWith(5, "set_school_web_zoom", {
      site: "science-portal",
      scale: 1.2,
    });
  });

  it("exports the saved Matrix through its own native command", async () => {
    const invoke = vi.fn(async () => true);
    const bridge = createTauriDesktopBridge(invoke, vi.fn(), "native-desktop");
    await expect(bridge.exportSchoolMatrix()).resolves.toBe(true);
    expect(invoke).toHaveBeenCalledWith("export_school_matrix", undefined);
  });

  it("rejects malformed native return values", async () => {
    const bridge = createTauriDesktopBridge(
      vi.fn(async () => "yes"),
      vi.fn(),
      "native-desktop",
    );
    await expect(bridge.deleteSchoolCredentials("science")).rejects.toThrow(
      "invalid result",
    );
  });

  it("filters malformed session events and disposes listeners", async () => {
    let handler: ((value: unknown) => void) | undefined;
    const unlisten = vi.fn();
    const listen = vi.fn(
      async (_event: string, callback: (value: unknown) => void) => {
        handler = callback;
        return unlisten;
      },
    );
    const bridge = createTauriDesktopBridge(vi.fn(), listen, "native-desktop");
    const callback = vi.fn();
    const dispose = bridge.onSchoolSession?.(callback);
    await Promise.resolve();
    handler?.({ profile: "science", state: "active" });
    handler?.({ profile: "other", state: "active" });
    handler?.({ profile: "science", state: "broken" });
    expect(callback).toHaveBeenCalledOnce();
    dispose?.();
    expect(unlisten).toHaveBeenCalledOnce();
  });
});
