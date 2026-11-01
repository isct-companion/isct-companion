// @vitest-environment jsdom
// @ts-expect-error jsdom ships without type declarations here.
import { JSDOM } from "jsdom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type {
  SchoolAuthProfile,
  SchoolSessionSnapshot,
  SchoolSessionState,
} from "@app/platform/desktop-bridge";
import { bootstrapReminders } from "@app/platform/reminders";
import { schoolSessions } from "@app/platform/school-session";
import { schoolWebBounds } from "@app/platform/school-web-bounds";
import { networkHint } from "@app/platform/sync";
import { readRepoFile, schoolProfiles } from "@tests/harness";

/**
 * Every school-web contract reachable from the webview: child-webview geometry,
 * the script Core runs in LMS assignment pages, the session projection the
 * shell renders, reference-polling network policy and the reminder-log
 * migration.
 *
 * The script is shipped by Rust; this suite runs the shipped file rather than
 * a transcription of it. Which pages a view may load, open or save from is
 * Core's rule table, tested where it runs
 * (`crates/app-core/src/tests/school_browser*.rs`); how Core reads the school's
 * sign-in pages is tested in `crates/adpt-isct/src/tests/login.rs`.
 */

const readScript = (name: string) =>
  readRepoFile(`crates/adpt-isct/assets/${name}`);

/* ------------------------------------------------------------------ *
 * School webview bounds
 * ------------------------------------------------------------------ */

const viewport = (
  dimensions: Partial<{
    innerWidth: number;
    innerHeight: number;
    outerWidth: number;
    outerHeight: number;
  }> = {},
) =>
  ({
    innerWidth: 1180,
    innerHeight: 740,
    outerWidth: 1196,
    outerHeight: 779,
    ...dimensions,
  }) as Window;

describe("school webview bounds", () => {
  it("uses the DOM client-area rectangle unchanged on Windows", () => {
    expect(
      schoolWebBounds(
        { left: 180.4, top: 32.4, width: 987.6, height: 674.6 },
        viewport(),
        "windows",
      ),
    ).toEqual({ x: 180, y: 32, width: 988, height: 675 });
  });

  it("keeps the WKWebView title-bar compensation on macOS", () => {
    expect(
      schoolWebBounds(
        { left: 180, top: 32, width: 988, height: 675 },
        viewport({ outerWidth: 1180, outerHeight: 740 }),
        "macos",
      ),
    ).toEqual({ x: 180, y: 60, width: 988, height: 675 });
  });

  it("subtracts half the horizontal chrome inset from the macOS origin", () => {
    // 1196x779 outer against a 1180x740 client area: 16px of side chrome and a
    // 39px measured title-bar inset, so x shifts by 8 and y by 39 - 8 = 31.
    expect(
      schoolWebBounds(
        { left: 180, top: 32, width: 988, height: 675 },
        viewport(),
        "macos",
      ),
    ).toEqual({ x: 188, y: 63, width: 988, height: 675 });
  });

  it("reads the injected native OS when the caller does not name one", () => {
    const scope = window as unknown as Record<string, unknown>;
    scope.__ISCT_COMPANION_NATIVE_OS__ = "windows";
    expect(
      schoolWebBounds(
        { left: 180.4, top: 32.4, width: 987.6, height: 674.6 },
        viewport(),
      ),
    ).toEqual({ x: 180, y: 32, width: 988, height: 675 });
    scope.__ISCT_COMPANION_NATIVE_OS__ = "macos";
    expect(
      schoolWebBounds(
        { left: 180, top: 32, width: 988, height: 675 },
        viewport({ outerWidth: 1180, outerHeight: 740 }),
      ),
    ).toEqual({ x: 180, y: 60, width: 988, height: 675 });
    delete scope.__ISCT_COMPANION_NATIVE_OS__;
  });
});

/* ------------------------------------------------------------------ *
 * School session projection
 * ------------------------------------------------------------------ */

const snapshotOf = (
  revision: number,
  science: SchoolSessionState,
  tokyoTech: SchoolSessionState,
): SchoolSessionSnapshot => ({
  revision,
  profiles: schoolProfiles(science, tokyoTech),
});

const INITIAL_SESSION = snapshotOf(1, "idle", "not-configured");

function sessionBridge(snapshot: SchoolSessionSnapshot = INITIAL_SESSION) {
  let emit: (snapshot: SchoolSessionSnapshot) => void = () => undefined;
  const order: string[] = [];
  const dispose = vi.fn();
  return {
    order,
    dispose,
    emit: (next: SchoolSessionSnapshot) => emit(next),
    onSchoolSession: vi.fn(
      async (callback: (snapshot: SchoolSessionSnapshot) => void) => {
        order.push("listen");
        emit = callback;
        return dispose;
      },
    ),
    schoolSessionSnapshot: vi.fn(async () => {
      order.push("snapshot");
      return snapshot;
    }),
    retrySchool: vi.fn(async (_profile: SchoolAuthProfile) => snapshot),
  };
}

const stateOf = (profile: SchoolAuthProfile) =>
  schoolSessions.get(profile)?.state;

describe("school session projection", () => {
  beforeEach(() => schoolSessions.reset());
  afterEach(() => schoolSessions.reset());

  it("subscribes before reading the initial snapshot and reports nothing until it lands", async () => {
    let resolveSnapshot!: (snapshot: SchoolSessionSnapshot) => void;
    const native = sessionBridge();
    native.schoolSessionSnapshot.mockImplementation(() => {
      native.order.push("snapshot");
      return new Promise<SchoolSessionSnapshot>((resolve) => {
        resolveSnapshot = resolve;
      });
    });
    const detach = schoolSessions.attach(native);
    await vi.waitFor(() =>
      expect(native.order).toEqual(["listen", "snapshot"]),
    );
    expect(schoolSessions.get("science")).toBeUndefined();
    resolveSnapshot(INITIAL_SESSION);
    await vi.waitFor(() =>
      expect(schoolSessions.get("science")).toEqual(
        INITIAL_SESSION.profiles.science,
      ),
    );
    expect(stateOf("tokyo-tech")).toBe("not-configured");
    detach();
  });

  it("does not let an older snapshot overwrite an event received during the read", async () => {
    let resolveSnapshot!: (snapshot: SchoolSessionSnapshot) => void;
    const native = sessionBridge();
    native.schoolSessionSnapshot.mockImplementation(
      () =>
        new Promise<SchoolSessionSnapshot>((resolve) => {
          resolveSnapshot = resolve;
        }),
    );
    schoolSessions.attach(native);
    await vi.waitFor(() => expect(native.onSchoolSession).toHaveBeenCalled());
    native.emit(snapshotOf(3, "active", "signing-in"));
    resolveSnapshot(snapshotOf(2, "rejected", "active"));
    // The read lands after every pending microtask has run.
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(stateOf("science")).toBe("active");
    expect(stateOf("tokyo-tech")).toBe("signing-in");
  });

  it("ignores stale and duplicate snapshot revisions", async () => {
    const native = sessionBridge(snapshotOf(4, "active", "rejected"));
    schoolSessions.attach(native);
    await vi.waitFor(() => expect(stateOf("science")).toBe("active"));
    native.emit(snapshotOf(3, "unavailable", "active"));
    native.emit(snapshotOf(4, "unavailable", "active"));
    expect(stateOf("science")).toBe("active");
    expect(stateOf("tokyo-tech")).toBe("rejected");
    native.emit(snapshotOf(5, "unavailable", "active"));
    expect(stateOf("science")).toBe("unavailable");
    expect(stateOf("tokyo-tech")).toBe("active");
  });

  it("reference-counts consumers and cleans up before reconnecting", async () => {
    const first = sessionBridge();
    const detachFirst = schoolSessions.attach(first);
    const detachSecond = schoolSessions.attach(first);
    await vi.waitFor(() =>
      expect(first.onSchoolSession).toHaveBeenCalledOnce(),
    );
    detachFirst();
    expect(first.dispose).not.toHaveBeenCalled();
    detachSecond();
    expect(first.dispose).toHaveBeenCalledOnce();

    const second = sessionBridge();
    schoolSessions.attach(second);
    await vi.waitFor(() =>
      expect(second.onSchoolSession).toHaveBeenCalledOnce(),
    );
  });

  it("rejects late events from an old bridge and accepts a new revision epoch", async () => {
    const first = sessionBridge(snapshotOf(8, "active", "rejected"));
    schoolSessions.attach(first);
    await vi.waitFor(() => expect(stateOf("science")).toBe("active"));

    const second = sessionBridge(snapshotOf(0, "idle", "not-configured"));
    schoolSessions.attach(second);
    await vi.waitFor(() => expect(stateOf("science")).toBe("idle"));
    expect(stateOf("tokyo-tech")).toBe("not-configured");

    first.emit(snapshotOf(99, "rejected", "rejected"));
    expect(stateOf("science")).toBe("idle");
    second.emit(snapshotOf(1, "active", "not-configured"));
    expect(stateOf("science")).toBe("active");
  });

  it("preserves revisions across a brief reconnect to the same bridge", async () => {
    const native = sessionBridge(snapshotOf(5, "rejected", "idle"));
    const detach = schoolSessions.attach(native);
    await vi.waitFor(() => expect(stateOf("science")).toBe("rejected"));
    native.emit(snapshotOf(6, "active", "idle"));
    detach();
    native.schoolSessionSnapshot.mockResolvedValue(
      snapshotOf(0, "rejected", "rejected"),
    );

    schoolSessions.attach(native);
    await vi.waitFor(() =>
      expect(native.schoolSessionSnapshot).toHaveBeenCalledTimes(2),
    );
    expect(stateOf("science")).toBe("active");
  });

  it("applies a Retry answer by revision, and only from the current bridge", async () => {
    const native = sessionBridge(snapshotOf(2, "rejected", "idle"));
    schoolSessions.attach(native);
    await vi.waitFor(() => expect(stateOf("science")).toBe("rejected"));

    native.retrySchool.mockResolvedValueOnce(snapshotOf(3, "active", "idle"));
    await schoolSessions.retry(native, "science");
    expect(native.retrySchool).toHaveBeenCalledWith("science");
    expect(stateOf("science")).toBe("active");

    // An answer older than an event already applied changes nothing.
    native.retrySchool.mockResolvedValueOnce(snapshotOf(1, "rejected", "idle"));
    await schoolSessions.retry(native, "science");
    expect(stateOf("science")).toBe("active");

    // A bridge that is no longer attached cannot write into the store.
    const replaced = sessionBridge(snapshotOf(0, "idle", "idle"));
    schoolSessions.attach(replaced);
    await vi.waitFor(() => expect(stateOf("science")).toBe("idle"));
    native.retrySchool.mockResolvedValueOnce(snapshotOf(9, "active", "idle"));
    await schoolSessions.retry(native, "science");
    expect(stateOf("science")).toBe("idle");
  });
});
/* ------------------------------------------------------------------ *
 * Reference polling network policy
 * ------------------------------------------------------------------ */

const onlineDescriptor = Object.getOwnPropertyDescriptor(navigator, "onLine");
const connectionDescriptor = Object.getOwnPropertyDescriptor(
  navigator,
  "connection",
);

function setNetwork(
  online: boolean,
  connection?: { type?: string; saveData?: boolean; metered?: boolean },
) {
  Object.defineProperty(navigator, "onLine", {
    configurable: true,
    value: online,
  });
  Object.defineProperty(navigator, "connection", {
    configurable: true,
    value: connection,
  });
}

function restoreProperty(
  name: "onLine" | "connection",
  descriptor?: PropertyDescriptor,
) {
  if (descriptor) Object.defineProperty(navigator, name, descriptor);
  else Reflect.deleteProperty(navigator, name);
}

describe("network hint for the sync planner", () => {
  afterEach(() => {
    restoreProperty("onLine", onlineDescriptor);
    restoreProperty("connection", connectionDescriptor);
  });

  it("pauses offline and metered connections", () => {
    setNetwork(false);
    expect(networkHint()).toBe("offline");

    for (const connection of [
      { type: "cellular" },
      { type: "wifi", saveData: true },
      { type: "wifi", metered: true },
    ]) {
      setNetwork(true, connection);
      expect(networkHint()).toBe("cellular");
    }
  });

  it("names Wi-Fi and leaves an unreported link unknown for Core", () => {
    setNetwork(true, { type: "wifi" });
    expect(networkHint()).toBe("wifi");
    setNetwork(true);
    expect(networkHint()).toBe("unknown");
  });
});

/* ------------------------------------------------------------------ *
 * Reminder log migration
 * ------------------------------------------------------------------ */

describe("reminder log migration", () => {
  it("moves the legacy log only after Core has persisted it", async () => {
    const order: string[] = [];
    const fired = { "class:abc": 123 };
    const storage = {
      getItem: () => JSON.stringify(fired),
      removeItem: vi.fn(() => {
        order.push("remove");
      }),
    };
    const bridge = {
      bootstrapReminders: vi.fn(async (value: unknown) => {
        expect(value).toEqual(fired);
        order.push("persist");
        return true;
      }),
    };
    await bootstrapReminders(bridge, storage);
    expect(order).toEqual(["persist", "remove"]);
    expect(storage.removeItem).toHaveBeenCalledWith("isct-reminders-fired");
  });

  it("keeps the legacy record when native persistence fails", async () => {
    const storage = { getItem: () => '{"class:abc":123}', removeItem: vi.fn() };
    await expect(
      bootstrapReminders(
        {
          bootstrapReminders: async () => {
            throw new Error("disk full");
          },
        },
        storage,
      ),
    ).rejects.toThrow("disk full");
    expect(storage.removeItem).not.toHaveBeenCalled();
  });

  it("transfers an empty log when the legacy browser storage is invalid", async () => {
    const bootstrap = vi.fn(async () => true);
    await bootstrapReminders(
      { bootstrapReminders: bootstrap },
      { getItem: () => "invalid", removeItem: () => undefined },
    );
    expect(bootstrap).toHaveBeenCalledWith({});
  });
});

/* Assignment draft upload: the external Moodle uploader owns the network work. */
describe("assignment upload injection", () => {
  const pages: { window: { close(): void } }[] = [];
  afterEach(() => pages.splice(0).forEach((page) => page.window.close()));

  function uploadPage(
    url = "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=100&action=editsubmission",
    loaded = true,
  ) {
    const page = new JSDOM(
      `<form><div class="fitem"><input name="files_filemanager"><div class="filemanager ${loaded ? "fm-loaded" : "fm-loading"}"><div class="filemanager-container"></div></div></div><button type="submit" id="final-submit">Submit</button></form>`,
      { url, runScripts: "outside-only" },
    );
    pages.push(page);
    const w = page.window;
    // jsdom has no drag-transfer APIs; retain real File, DOM events and the injected script.
    w.DataTransfer = class {
      files: File[] = [];
      types = ["Files"];
      items = { add: (file: File) => this.files.push(file) };
    };
    w.DragEvent = class extends w.Event {
      dataTransfer: unknown;
      constructor(type: string, options: { dataTransfer: unknown }) {
        super(type, options);
        this.dataTransfer = options.dataTransfer;
      }
    };
    const drops: File[][] = [];
    w.document
      .querySelector(".filemanager-container")!
      .addEventListener("drop", (event: Event) => {
        event.preventDefault();
        drops.push([
          ...(event as unknown as { dataTransfer: { files: File[] } })
            .dataTransfer.files,
        ]);
      });
    const submitted = vi.fn((event: Event) => event.preventDefault());
    w.document.querySelector("form")!.addEventListener("submit", submitted);
    const start = () =>
      w.eval(
        `${readScript("assignment-upload.js")}(${JSON.stringify({ request: "test-upload", url: "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=100&action=editsubmission", files: [{ name: "report.txt", data: Buffer.from("my report").toString("base64") }], messages: { waiting: "waiting", queued: "Review and submit yourself.", failed: "Choose files in LMS." } })})`,
      );
    return { w, drops, submitted, start };
  }

  it("hands files to Moodle once and leaves the final submission to the user", () => {
    const page = uploadPage();
    page.start();
    page.start();
    expect(page.drops).toHaveLength(1);
    expect(page.drops[0][0].name).toBe("report.txt");
    expect(page.drops[0][0].size).toBe(9);
    expect(page.w.document.querySelector('[role="status"]')?.textContent).toBe(
      "Review and submit yourself.",
    );
    expect(page.submitted).not.toHaveBeenCalled();
    page.w.document.querySelector("#final-submit")!.click();
    expect(page.submitted).toHaveBeenCalledTimes(1);
  });

  it.each([
    "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=200&action=editsubmission",
    "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=100",
    "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=100&id=200&action=editsubmission",
    "https://example.com/2025/mod/assign/view.php?id=100&action=editsubmission",
  ])("never transfers files on an unrequested page: %s", (url) => {
    const page = uploadPage(url);
    page.start();
    expect(page.drops).toHaveLength(0);
    expect(page.submitted).not.toHaveBeenCalled();
  });

  it("waits for Moodle initialization and cancels on navigation", async () => {
    const page = uploadPage(undefined, false);
    page.start();
    expect(page.drops).toHaveLength(0);
    page.w.document.querySelector(".filemanager")!.className =
      "filemanager fm-loaded";
    await Promise.resolve();
    expect(page.drops).toHaveLength(1);
    const leaving = uploadPage(undefined, false);
    leaving.start();
    leaving.w.dispatchEvent(new leaving.w.Event("pagehide"));
    leaving.w.document.querySelector(".filemanager")!.className =
      "filemanager fm-loaded";
    await Promise.resolve();
    expect(leaving.drops).toHaveLength(0);
    const changed = uploadPage(undefined, false);
    changed.start();
    changed.w.history.replaceState(null, "", "?id=200&action=editsubmission");
    changed.w.document.querySelector(".filemanager")!.className =
      "filemanager fm-loaded";
    await Promise.resolve();
    expect(changed.drops).toHaveLength(0);
  });
});
