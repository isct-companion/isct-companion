// @vitest-environment jsdom
import { act, useState } from "react";
import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import {
  advance,
  click,
  chooseOption,
  courseFor,
  emptySchoolRecords,
  installFakeCore,
  mount,
  mountApp,
  offering,
  overviewFor,
  planItemFor,
  planningWeekFor,
  scheduleViewFor,
  typeInto,
  until,
  watchConsoleErrors,
  workspaceFixture,
  type FakeCore,
  type FakeCoreOptions,
  type Mounted,
  type MountedApp,
  type QueryTable,
} from "@tests/harness";
import { paths } from "@app/app/routes";
import { notify } from "@app/app/notify";
import i18n from "@app/i18n";
import type { Workspace } from "@app/lib/model";
import type { FinderAction, FinderPage } from "@app/generated/persistence";
import type { ScheduleQuery } from "@app/platform/core-query";

/**
 * The whole shell over a scripted Core: hash routing, page retention, keyboard
 * shortcuts, the native-navigation blocker and the query caches that only
 * exist inside React hooks. These assert what a person sees, not which
 * component rendered.
 *
 * Local helpers exist where the harness has no builder: `shellOptions` (a
 * navigable workspace with the school-web commands answered), `mountShell`
 * (adopts this test's workspace before mounting, because the harness reuses one
 * module-level store), `emptyFinder`, and the `Schedule*` / `FinderProbe`
 * probe components.
 */

const placed = offering("202602421", "CSC.T252", "Placed");
const clashing = offering("202600009", "CSC.T999", "Clashing");
const placedCourse = courseFor(placed);
const clashingCourse = courseFor(clashing);

/**
 * The store and query modules capture the Tauri bridge the first time they
 * load, so a scripted host must already be installed when this suite imports
 * them. A static import would run before any test and bind the browser-preview
 * service, which is exactly what `mountApp`'s ordering rule protects against.
 */
let coreQuery: typeof import("@app/platform/core-query");
let finderOwner: typeof import("@app/pages/courses/useFinder");
let storeModule: typeof import("@app/store/workspace");

beforeAll(async () => {
  installFakeCore({ workspace: workspaceFixture() });
  coreQuery = await import("@app/platform/core-query");
  finderOwner = await import("@app/pages/courses/useFinder");
  storeModule = await import("@app/store/workspace");
  // Warm the shell's module graph too, so the first mounted test does not pay
  // the whole transform cost inside its own timeout.
  await Promise.all([
    import("@app/app/App"),
    import("@app/app/routes"),
    import("react-router"),
  ]);
}, 60_000);

let app: MountedApp | undefined;
let probe: Mounted | undefined;
let consoleWatch: ReturnType<typeof watchConsoleErrors> | undefined;

afterEach(async () => {
  await probe?.unmount();
  probe = undefined;
  await app?.unmount();
  app = undefined;
  consoleWatch?.restore();
  consoleWatch = undefined;
  await i18n.changeLanguage("en");
  delete (window as unknown as Record<string, unknown>)
    .__ISCT_COMPANION_NATIVE_TARGET__;
});

function titles(host: HTMLElement): string[] {
  return [
    ...new Set(
      [...host.querySelectorAll(".ttBlock b")].map(
        (node) => node.textContent ?? "",
      ),
    ),
  ];
}

/* ------------------------------------------------------------------ *
 * A navigable shell over the scripted host.
 * ------------------------------------------------------------------ */

const CAPABILITIES = {
  schoolImport: true,
  schoolWeb: true,
  notifications: false,
  backgroundSchedule: false,
  cloudSync: false,
  matrixOcr: false,
};

type Handle = NonNullable<FakeCoreOptions["handle"]>;

/** Answers every non-query command the shell issues on the routes we cover. */
const baseHandle: Handle = (command, args) => {
  switch (command) {
    case "bootstrap_reminders":
    case "hide_school_web":
    case "configure_school_web_policy":
    case "set_school_web_zoom":
    case "set_school_web_bounds":
    case "set_kyomu_guide":
    case "reload_school_web":
    case "navigate_school_web":
    case "school_web_back":
    case "school_web_forward":
      return true;
    case "open_school_site": {
      const target = args.targetUrl as string | null | undefined;
      return {
        site: args.site,
        url: target ?? "https://portal.isct.ac.jp/resident",
        phase: "pageLoaded",
      };
    }
    case "resolve_school_url": {
      const url = String(args.url);
      return {
        site: url.includes("lms") ? "science-lms" : args.site,
        url,
      };
    }
    case "school_credential_status":
      return {
        science: {
          availability: "not-configured",
          saved: false,
          username: "",
          hasOtp: false,
          hasMatrix: false,
        },
        tokyoTech: {
          availability: "not-configured",
          saved: false,
          username: "",
          hasOtp: false,
          hasMatrix: false,
        },
      };
    case "school_session_snapshot":
      return {
        revision: 0,
        states: { science: "pending", tokyoTech: "pending" },
      };
    default:
      return undefined;
  }
};

function shellQueries(): QueryTable {
  return {
    workspaceOverview: (_query: unknown, core: FakeCore) =>
      overviewFor(core.workspace),
    scheduleView: (_query: unknown, core: FakeCore) =>
      scheduleViewFor(core.workspace),
    planningWeek: (_query: unknown, core: FakeCore) =>
      planningWeekFor(core.workspace),
    taskView: {
      byTerm: {},
      nearestByCourse: {},
      deadlines: {},
      submittedIds: [],
    },
  };
}

/** The finder page Core returns when nothing is stored, placed or searched. */
const emptyFinder = (request: Record<string, unknown>): FinderPage => ({
  context: {
    currentQuarter: 3,
    recommended: {},
    departments: [],
    busy: [],
    officialOptions: [],
  },
  factors: [],
  duplicates: 0,
  keys: (request.keys as string[] | undefined) ?? [],
  live: String(request.input ?? ""),
  suggestions: [],
  total: 0,
  offset: 0,
  rows: [],
  stored: 0,
  ineligible: 0,
  activeSlots: [],
  placedCodes: [],
  commitKey: null,
  pasteHandled: false,
  pasted: [],
  missingCodes: [],
  ineligibleCodes: [],
  catalogVersion: 0,
  contentRevision: "",
});

function shellOptions(
  options: Partial<FakeCoreOptions> & { workspace: Workspace },
): FakeCoreOptions {
  return {
    capabilities: CAPABILITIES,
    queries: shellQueries(),
    handle: baseHandle,
    ...options,
  };
}

/**
 * `mountApp` reuses one module-level workspace store across a suite, so a later
 * mount would briefly validate routes against the previous test's snapshot and
 * replace a deep link before the new data arrived. Adopt this test's workspace
 * first, then hand off to the harness's mount helper.
 */
async function mountShell(
  options: FakeCoreOptions,
  appOptions?: { route?: string; strict?: boolean; guide?: boolean },
): Promise<MountedApp> {
  installFakeCore(options);
  await act(async () => {
    await storeModule.workspaceStore.reload();
  });
  return mountApp(options, appOptions);
}

/** An element a component must render before the test can proceed. */
function required<T extends Element>(
  find: () => T | null | undefined,
  what: string,
): Promise<T> {
  return until(() => {
    const found = find();
    if (!found) throw new Error(`${what} is not ready`);
    return found;
  });
}

/* ------------------------------------------------------------------ *
 * Shell navigation.
 * ------------------------------------------------------------------ */

describe("shell navigation", () => {
  it("opens the workspace route named in the hash", async () => {
    const workspace = workspaceFixture({
      courses: [placedCourse],
      planItems: [planItemFor(placedCourse)],
    });
    app = await mountShell(shellOptions({ workspace, revision: "rev:route" }), {
      route: paths.home("week"),
    });
    await until(() =>
      expect(app!.host.querySelector(".homeMain")).not.toBeNull(),
    );
    expect(app.host.querySelector(".homePage.week")).not.toBeNull();
    expect(window.location.hash).toBe(`#${paths.home("week")}`);
  }, 20_000);

  it("replaces unknown, unsupported and deleted-course routes", async () => {
    consoleWatch = watchConsoleErrors();
    const workspace = workspaceFixture({ courses: [placedCourse] });
    app = await mountShell(
      shellOptions({ workspace, revision: "rev:redirect" }),
      { route: paths.home() },
    );
    const { host, core, router } = app;
    await until(() => expect(host.querySelector(".homeMain")).not.toBeNull());

    await app.go("/settings/not-a-section");
    await until(() =>
      expect(window.location.hash).toBe(`#${paths.settings("accounts")}`),
    );
    expect(router.state.historyAction).toBe("REPLACE");

    // A developer-tier page stays closed until Developer options are on.
    await app.go(paths.settings("diagnostics"));
    await until(() =>
      expect(window.location.hash).toBe(`#${paths.settings("accounts")}`),
    );

    await app.go("/decidedly-not-a-route");
    await until(() => expect(window.location.hash).toBe(`#${paths.home()}`));

    // A lecture naming a deleted course falls back to the lecture list.
    await act(async () => {
      core.publish(
        workspaceFixture({
          courses: [{ ...placedCourse, deletedAt: "2026-10-02T00:00:00Z" }],
        }),
        "rev:redirect-2",
      );
      await storeModule.workspaceStore.reload();
    });
    await app.go(paths.lectures(placedCourse.id));
    await until(() =>
      expect(window.location.hash).toBe(`#${paths.lectures()}`),
    );
    expect(host.querySelector(".lectureSlot:not([hidden])")).toBeNull();

    consoleWatch.expectClean();
    consoleWatch.restore();
    consoleWatch = undefined;
  }, 20_000);

  it("keeps a desktop settings section and replaces one this build lacks", async () => {
    const options = shellOptions({
      workspace: workspaceFixture(),
      revision: "rev:platform",
    });
    app = await mountShell(options, { route: paths.settings("files") });
    await until(() =>
      expect(app!.host.querySelector(".settingsMain")).not.toBeNull(),
    );
    // Course files are a desktop page, so this build keeps it.
    expect(window.location.hash).toBe(`#${paths.settings("files")}`);

    // The native bootstrap injects the build target; a preview build without
    // desktop file settings must not leave the person on an unknown page.
    await app.unmount();
    (
      window as unknown as Record<string, unknown>
    ).__ISCT_COMPANION_NATIVE_TARGET__ = "browser-preview";
    app = await mountShell(options, { route: paths.settings("files") });
    await until(() =>
      expect(window.location.hash).toBe(`#${paths.settings("accounts")}`),
    );
    expect(app.router.state.historyAction).toBe("REPLACE");
  }, 20_000);

  it("returns each page to where it was left when switching pages", async () => {
    const workspace = workspaceFixture({ courses: [placedCourse] });
    app = await mountShell(
      shellOptions({ workspace, revision: "rev:return" }),
      { route: paths.lectures(placedCourse.id) },
    );
    const { host } = app;
    await until(() =>
      expect(host.querySelector(".lecturesMain")).not.toBeNull(),
    );

    const rail = (label: string) =>
      [...host.querySelectorAll<HTMLButtonElement>(".rail button")].find(
        (button) => button.textContent?.includes(label),
      )!;

    await click(rail("Settings"));
    await until(() =>
      expect(window.location.hash).toBe(`#${paths.settings("accounts")}`),
    );
    await app.go(paths.settings("appearance"));
    await click(rail("Lectures"));
    await until(() =>
      expect(window.location.hash).toBe(`#${paths.lectures(placedCourse.id)}`),
    );
    await click(rail("Settings"));
    await until(() =>
      expect(window.location.hash).toBe(`#${paths.settings("appearance")}`),
    );
  }, 20_000);

  it("retains settings disclosure, scroll and controls while the page is hidden", async () => {
    app = await mountShell(
      shellOptions({
        workspace: workspaceFixture(),
        revision: "rev:retain-settings",
      }),
      { route: paths.settings("appearance") },
    );
    const { host } = app;
    // The hash names the section that opens.
    await until(() =>
      expect(
        host.querySelector(".settingsNav button.active")?.textContent,
      ).toContain("Appearance"),
    );
    const disclosure = await required(
      () => host.querySelector<HTMLDetailsElement>(".settingsPane details"),
      "the folded options",
    );
    disclosure.open = true;
    const pane = await required(
      () => host.querySelector<HTMLElement>(".settingsPane"),
      "the settings pane",
    );
    pane.scrollTop = 164;
    const language = await required(
      () => host.querySelector<HTMLInputElement>('input[value="zh"]'),
      "the Chinese language choice",
    );

    await app.go(paths.home());
    await app.go(paths.settings("appearance"));

    expect(host.querySelector('input[value="zh"]')).toBe(language);
    expect(host.querySelector(".settingsPane details")).toBe(disclosure);
    expect(disclosure.open).toBe(true);
    expect(pane.scrollTop).toBe(164);
  }, 20_000);

  it("retains lecture filter, scroll and chosen quarter while the page is hidden", async () => {
    const workspace = workspaceFixture({ courses: [placedCourse] });
    app = await mountShell(
      shellOptions({
        workspace,
        revision: "rev:retain-lectures",
        queries: {
          ...shellQueries(),
          scheduleView: (_query: unknown, core: FakeCore) =>
            scheduleViewFor(core.workspace, {
              allTerms: ["2026-3Q", "2026-2Q"],
            }),
        },
      }),
      { route: paths.lectures(placedCourse.id) },
    );
    const { host } = app;
    const filter = await required(
      () => host.querySelector<HTMLInputElement>('input[aria-label="Filter"]'),
      "the lecture filter",
    );
    const term = await required(
      () =>
        host.querySelector<HTMLInputElement>('.lectureTerm [role="combobox"]'),
      "the quarter picker",
    );
    // A quarter change clears the filter, so the filter is typed after it.
    await typeInto(filter, "algebra");
    await chooseOption(term, "2026-2Q");
    expect(filter.value).toBe("");
    await typeInto(filter, "graph");
    const list = await required(
      () => host.querySelector<HTMLElement>(".lectureList"),
      "the lecture list",
    );
    list.scrollTop = 128;

    await app.go(paths.home());
    await app.go(paths.lectures(placedCourse.id));

    expect(host.querySelector('input[aria-label="Filter"]')).toBe(filter);
    expect(filter.value).toBe("graph");
    expect(host.querySelector(".lectureList")).toBe(list);
    expect(list.scrollTop).toBe(128);
    expect(host.querySelector('.lectureTerm [role="combobox"]')).toBe(term);
    expect(term.value).toBe("2026 2Q");
  }, 20_000);

  it("blocks Back while the Web page owns the native view and restores the URL when hiding fails", async () => {
    let hideFails = false;
    app = await mountShell(
      shellOptions({
        workspace: workspaceFixture(),
        revision: "rev:blocker",
        handle: (command, args, core) => {
          if (command === "hide_school_web" && hideFails)
            throw new Error("Native hide failed");
          return baseHandle(command, args, core);
        },
      }),
      { route: paths.home() },
    );
    const { host, core } = app;
    await app.go(paths.web());
    await until(() =>
      expect(core.commands("open_school_site").length).toBeGreaterThan(0),
    );

    hideFails = true;
    await act(async () => {
      window.history.back();
      await new Promise((resolve) => setTimeout(resolve, 40));
    });
    await until(() =>
      expect(document.body.textContent).toContain("Native hide failed"),
    );
    expect(window.location.hash).toBe(`#${paths.web()}`);
    expect(host.querySelector(".webMain")).not.toBeNull();
    expect(core.commands("open_school_site").length).toBeGreaterThan(0);

    hideFails = false;
    await act(async () => {
      window.history.back();
      await new Promise((resolve) => setTimeout(resolve, 40));
    });
    await until(() => expect(window.location.hash).toBe(`#${paths.home()}`));
    // The native view is hidden, not destroyed: the page and its address stay.
    expect(host.querySelector('[aria-label="Current URL"]')).not.toBeNull();
  }, 20_000);

  it("keeps an open lecture when hiding the native view fails", async () => {
    let hideFails = true;
    app = await mountShell(
      shellOptions({
        workspace: workspaceFixture({ courses: [placedCourse] }),
        revision: "rev:blocker-lecture",
        handle: (command, args, core) => {
          if (command === "hide_school_web" && hideFails)
            throw new Error("Lecture hide failed");
          return baseHandle(command, args, core);
        },
      }),
      { route: paths.lectures(placedCourse.id) },
    );
    const { host } = app;
    await until(() =>
      expect(host.querySelector(".lectureSlot:not([hidden])")).not.toBeNull(),
    );

    await app.go(paths.home());
    await until(() =>
      expect(document.body.textContent).toContain("Lecture hide failed"),
    );
    expect(window.location.hash).toBe(`#${paths.lectures(placedCourse.id)}`);
    expect(host.querySelector(".lecturesMain")).not.toBeNull();
  }, 20_000);

  it("waits for native hiding when leaving Web", async () => {
    let holdHide = true;
    let releaseHide!: () => void;
    const hidden = new Promise<boolean>((resolve) => {
      releaseHide = () => {
        holdHide = false;
        resolve(true);
      };
    });
    app = await mountShell(
      shellOptions({
        workspace: workspaceFixture(),
        revision: "rev:ingest",
        handle: (command, args, core) => {
          if (command === "hide_school_web") return holdHide ? hidden : true;
          return baseHandle(command, args, core);
        },
      }),
      { route: paths.web() },
    );
    const { host, core } = app;
    await until(() =>
      expect(core.commands("open_school_site").length).toBeGreaterThan(0),
    );

    await app.go(paths.home());
    expect(window.location.hash).toBe(`#${paths.web()}`);
    expect(host.querySelector(".webMain")).not.toBeNull();

    await act(async () => {
      releaseHide();
      await hidden;
    });
    await until(() => expect(window.location.hash).toBe(`#${paths.home()}`));
  }, 20_000);
});

/* ------------------------------------------------------------------ *
 * Keyboard shortcuts.
 * ------------------------------------------------------------------ */

describe("keyboard shortcuts", () => {
  it("routes ⌘1–5, replacing the section", async () => {
    app = await mountShell(
      shellOptions({
        workspace: workspaceFixture({ courses: [placedCourse] }),
        revision: "rev:keys",
      }),
      { route: paths.settings("appearance") },
    );
    const { host, router } = app;

    const study = await required(
      () =>
        [
          ...host.querySelectorAll<HTMLButtonElement>(".settingsNav button"),
        ].find((button) => button.textContent?.trim() === "Study"),
      "the Study settings entry",
    );
    await click(study);
    expect(window.location.hash).toBe(`#${paths.settings("study")}`);
    expect(router.state.historyAction).toBe("REPLACE");

    for (const [index, target] of [
      paths.home(),
      paths.timetable(),
      paths.lectures(),
      paths.web(),
      // ⌘5 returns to the section left above.
      paths.settings("study"),
    ].entries()) {
      await act(async () => {
        window.dispatchEvent(
          new KeyboardEvent("keydown", {
            key: String(index + 1),
            metaKey: true,
            cancelable: true,
          }),
        );
      });
      await until(() => expect(window.location.hash).toBe(`#${target}`));
    }
  }, 20_000);
});

/* ------------------------------------------------------------------ *
 * Native listener ordering.
 * ------------------------------------------------------------------ */

describe("native listener ordering", () => {
  it("subscribes to native changes before the recovery read that follows registration", async () => {
    let releaseListener!: () => void;
    const listenerRegistered = new Promise<void>((resolve) => {
      releaseListener = resolve;
    });
    app = await mountShell(
      shellOptions({
        workspace: workspaceFixture({ courses: [placedCourse] }),
        revision: "rev:ordering",
        handle: (command, args, core) => {
          if (
            command === "plugin:event|listen" &&
            args.event === "workspace-changed"
          )
            return listenerRegistered.then(() => 1);
          return baseHandle(command, args, core);
        },
      }),
      { route: paths.home() },
    );
    const { core } = app;
    await until(() =>
      expect(app!.host.querySelector(".homeMain")).not.toBeNull(),
    );

    const listenIndex = () =>
      core.calls.findIndex(
        (call) =>
          call.command === "plugin:event|listen" &&
          call.args.event === "workspace-changed",
      );
    const readsAfterListen = () =>
      core.calls.filter(
        (call, index) =>
          index > listenIndex() && call.command === "workspace_snapshot",
      ).length;

    await until(() => expect(listenIndex()).toBeGreaterThanOrEqual(0));
    // Registration is still in flight: the shell must not read yet.
    expect(readsAfterListen()).toBe(0);

    await act(async () => {
      releaseListener();
      await listenerRegistered;
    });
    await until(() => expect(readsAfterListen()).toBeGreaterThan(0));
  }, 20_000);

  it("patches a school change without a snapshot read, and lets a change heard during a slow read win", async () => {
    const before = workspaceFixture({ courses: [placedCourse] });
    const after = workspaceFixture({ courses: [clashingCourse] });
    let gateStale = false;
    let releaseStale!: () => void;
    const staleGate = new Promise<void>((resolve) => {
      releaseStale = resolve;
    });
    const stalePatch = {
      revision: "revision:stale",
      contentRevision: "revision:stale",
      workspace: before,
      schoolRecords: emptySchoolRecords(),
    };
    app = await mountShell(
      shellOptions({
        workspace: before,
        revision: "rev:stale",
        finder: emptyFinder,
        handle: (command, args, core) => {
          if (command === "workspace_records" && gateStale)
            return staleGate.then(() => stalePatch);
          return baseHandle(command, args, core);
        },
      }),
      { route: paths.timetable() },
    );
    const { host, core } = app;
    // The planner timetable is the person-visible view of the workspace rows.
    await until(() => expect(titles(host)).toEqual(["Placed"]));
    const snapshotReads = core.commands("workspace_snapshot").length;

    gateStale = true;
    // A school pull commits: its patch read hangs.
    await act(async () => {
      core.emit("workspace-changed", {
        source: "school-pull",
        changes: ["courses"],
      });
    });
    await until(() =>
      expect(core.commands("workspace_records")).toHaveLength(1),
    );
    // Meanwhile the next key commits a newer state.
    gateStale = false;
    await act(async () => {
      core.publish(after, "revision:new");
      core.emit("workspace-changed", {
        source: "school-pull",
        changes: ["courses", "freshness"],
      });
    });
    await act(async () => {
      releaseStale();
      await staleGate;
    });

    await until(() => expect(titles(host)).toEqual(["Clashing"]));
    expect(core.workspace.courses[0].title).toBe("Clashing");
    expect(
      core.commands("workspace_records").map((call) => call.args.changes),
    ).toEqual([["courses"], ["courses", "freshness"]]);
    expect(core.commands("workspace_snapshot")).toHaveLength(snapshotReads);
  }, 20_000);
});

/* ------------------------------------------------------------------ *
 * Web-page residency.
 * ------------------------------------------------------------------ */

describe("web-page residency", () => {
  it("keeps an open school page resident and hides rather than destroys it", async () => {
    app = await mountShell(
      shellOptions({
        workspace: workspaceFixture(),
        revision: "rev:resident",
      }),
      { route: paths.web() },
    );
    const { host, core } = app;
    await until(() =>
      expect(core.commands("open_school_site").length).toBeGreaterThan(0),
    );
    const address = await required(
      () => host.querySelector<HTMLInputElement>('[aria-label="Current URL"]'),
      "the school address bar",
    );
    await typeInto(address, "https://portal.isct.ac.jp/typed");

    await app.go(paths.home());
    await until(() =>
      expect(core.commands("hide_school_web").length).toBeGreaterThan(0),
    );
    // The page is hidden, not destroyed: same node, same typed address.
    expect(host.querySelector('[aria-label="Current URL"]')).toBe(address);
    expect(address.value).toBe("https://portal.isct.ac.jp/typed");

    const opened = core.commands("open_school_site").length;
    await app.go(paths.web());
    await until(() =>
      expect(core.commands("open_school_site").length).toBeGreaterThan(opened),
    );
    expect(host.querySelector('[aria-label="Current URL"]')).toBe(address);
    // An ordinary resume re-shows the resident session, without a new URL.
    expect(core.commands("open_school_site").at(-1)?.args.targetUrl).toBeNull();
    const search =
      "https://www.google.com/search?q=linear%20algebra&hl=ja#results";
    await typeInto(address, search);
    await act(async () =>
      address
        .closest("form")!
        .dispatchEvent(
          new Event("submit", { bubbles: true, cancelable: true }),
        ),
    );
    await until(() =>
      expect(core.commands("navigate_school_web").at(-1)?.args.url).toBe(
        search,
      ),
    );
    expect(core.commands("resolve_school_url").at(-1)?.args).toEqual({
      url: search,
      site: "science-portal",
    });
  }, 20_000);

  it("reopens an explicitly requested URL while an ordinary resume keeps the resident page", async () => {
    const url = "https://lms.s.isct.ac.jp/course/view.php?id=12";
    const course = courseFor(
      offering("202602421", "CSC.T252", "Placed", { id: "lms-12" }),
      { source: { provider: "lms", externalId: "12" }, url },
    );
    const workspace = workspaceFixture({
      courses: [course],
      planItems: [planItemFor(course)],
    });
    app = await mountShell(
      shellOptions({
        workspace,
        revision: "rev:explicit",
        queries: { ...shellQueries(), courseDetail: { lmsUrl: url } },
      }),
      { route: paths.timetable() },
    );
    const { core, host } = app;

    await click(
      await required(
        () => host.querySelector<HTMLButtonElement>(".ttBlock.course"),
        "the placed course block",
      ),
    );
    const link = await required(
      () =>
        host.querySelector<HTMLButtonElement>(
          ".coursePanel .officialLink button",
        ),
      "the official LMS link",
    );
    await click(link);
    await until(() =>
      expect(core.commands("open_school_site").at(-1)?.args.targetUrl).toBe(
        url,
      ),
    );
    expect(window.location.hash).toBe(`#${paths.web()}`);

    await app.go(paths.home());
    const opened = core.commands("open_school_site").length;
    await app.go(paths.web());
    await until(() =>
      expect(core.commands("open_school_site").length).toBeGreaterThan(opened),
    );
    expect(core.commands("open_school_site").at(-1)?.args.targetUrl).toBeNull();
  }, 20_000);
});

/* ------------------------------------------------------------------ *
 * Store failures.
 * ------------------------------------------------------------------ */

describe("store failures", () => {
  it("surfaces a failed store operation as a notification with a readable reason", async () => {
    app = await mountShell(
      shellOptions({
        workspace: workspaceFixture(),
        revision: "rev:error",
        handle: (command, args, core) => {
          if (command === "mutate_workspace") throw new Error("Disk is full");
          return baseHandle(command, args, core);
        },
      }),
      { route: paths.settings("appearance") },
    );
    const { host, core } = app;

    const toggle = await required(
      () =>
        host.querySelector<HTMLInputElement>(
          '[data-setting="bilingual"] input',
        ),
      "the bilingual toggle",
    );
    await click(toggle);

    await until(() =>
      expect(document.querySelector(".workspaceNotice")?.textContent).toContain(
        "Disk is full",
      ),
    );
    expect(core.mutations).toEqual([]);
    expect(host.querySelector(".pageError")).toBeNull();
  }, 20_000);

  it("offers Retry saving on an error notice and runs the retry it was given", async () => {
    app = await mountShell(
      shellOptions({
        workspace: workspaceFixture(),
        revision: "rev:retry",
      }),
      { route: paths.home() },
    );
    const { core } = app;
    const before = core.commands("workspace_snapshot").length;

    await act(async () => {
      notify.error(new Error("Disk is full"), () => {
        void storeModule.workspaceStore.reload();
      });
    });
    await until(() =>
      expect(document.querySelector(".workspaceNotice")?.textContent).toContain(
        "Disk is full",
      ),
    );
    const retry = [
      ...document.querySelectorAll<HTMLButtonElement>("button"),
    ].find((button) => button.textContent === "Retry saving");
    expect(retry).toBeDefined();
    await click(retry);
    await until(() =>
      expect(core.commands("workspace_snapshot").length).toBeGreaterThan(
        before,
      ),
    );
  }, 20_000);
});

/* ------------------------------------------------------------------ *
 * Chinese UI.
 * ------------------------------------------------------------------ */

describe("Chinese UI", () => {
  it("loads Chinese while keeping an explicit school-content language intact", async () => {
    const workspace = workspaceFixture();
    workspace.settings.locale = "zh";
    workspace.settings.contentLanguage = "en";
    app = await mountShell(shellOptions({ workspace, revision: "rev:zh" }), {
      route: paths.settings("appearance"),
    });
    const { host, core } = app;

    await until(() => expect(document.documentElement.lang).toBe("zh"));
    expect(host.querySelector(".rail button")?.textContent).toContain("今天");
    expect(document.body.textContent).toContain("中文");
    expect(
      host
        .querySelector('[data-setting="content-language"] input:checked')
        ?.getAttribute("value"),
    ).toBe("en");
    expect(
      host
        .querySelector('[data-setting="language"] input:checked')
        ?.getAttribute("value"),
    ).toBe("zh");
    expect(core.workspace.settings.contentLanguage).toBe("en");
  }, 20_000);
});

/* ------------------------------------------------------------------ *
 * Query-cache guarantees that live inside React hooks.
 * ------------------------------------------------------------------ */

type Overviewish = { term?: string; courses?: { title?: string }[] };

function ScheduleConsumer({
  queries,
  label,
}: {
  queries: ScheduleQuery[];
  label: string;
}) {
  const state =
    coreQuery.useScheduleQueries<[Overviewish, Overviewish]>(queries);
  return (
    <output
      className={`probe probe-${label}`}
      data-pending={state.pending ? "yes" : "no"}
      data-error={state.error ?? ""}
    >
      {state.value?.map((value) => value?.term ?? "").join("|")}
    </output>
  );
}

function SchedulePair({ queries }: { queries: ScheduleQuery[] }) {
  return (
    <div>
      <ScheduleConsumer queries={queries} label="one" />
      <ScheduleConsumer queries={queries} label="two" />
    </div>
  );
}

function ScheduleSwitchProbe({
  initial,
  next,
}: {
  initial: string;
  next: string;
}) {
  const [term, setTerm] = useState(initial);
  const state = coreQuery.useScheduleQuery<Overviewish>({
    kind: "workspaceOverview",
    term,
  });
  return (
    <div>
      <output className="probeTerm" data-pending={state.pending ? "yes" : "no"}>
        {state.value?.term ?? ""}
      </output>
      <output className="probeError">{state.error ?? ""}</output>
      <button className="probeSwitch" onClick={() => setTerm(next)}>
        switch
      </button>
      <button className="probeRetry" onClick={() => state.retry?.()}>
        retry
      </button>
    </div>
  );
}

/** The finder owner alone, its keys and its shown page's input. */
function FinderProbe({ term, nextTerm }: { term: string; nextTerm: string }) {
  const [current, setCurrent] = useState(term);
  const finder = finderOwner.useFinder({
    year: 2026,
    term: current,
    session: current,
    enabled: true,
    defaultKeys: [],
  });
  return (
    <div>
      <output className="queueKeys">{finder.keys.join(",")}</output>
      <output className="shownInput">{finder.page?.live ?? ""}</output>
      <button className="typeLate" onClick={() => finder.type("late")}>
        late
      </button>
      <button className="typeQuick" onClick={() => finder.type("quick")}>
        quick
      </button>
      <button className="queueA" onClick={() => finder.toggleTime("A", false)}>
        A
      </button>
      <button className="queueB" onClick={() => finder.toggleTime("B", true)}>
        B
      </button>
      <button
        className="queueRapid"
        onClick={() => {
          finder.toggleTime("A", false);
          finder.toggleTime("B", true);
        }}
      >
        rapid
      </button>
      <button className="queueReplace" onClick={() => finder.replace(["X"])}>
        replace
      </button>
      <button className="queueTerm" onClick={() => setCurrent(nextTerm)}>
        term
      </button>
    </div>
  );
}

/** Core's key contract for one finder action: read keys, apply, return keys. */
function finderAnswer(request: Record<string, unknown>): FinderPage {
  const keys = [...((request.keys as string[] | undefined) ?? [])];
  const action = request.action as FinderAction | null | undefined;
  if (action?.kind === "toggleTime") {
    if (action.append) keys.push(action.key);
    else keys.splice(0, keys.length, action.key);
  }
  return { ...emptyFinder(request), keys };
}
const actions = (core: FakeCore) =>
  core
    .commands("finder_query")
    .filter((call) => (call.args.request as { action?: unknown }).action);

function scheduleAnswer(kind: string, value: unknown, revision: string) {
  return { revision, values: [{ kind, value }] };
}

describe("schedule query caches", () => {
  const twoQueries: ScheduleQuery[] = [
    { kind: "workspaceOverview", term: "cache-pair" },
    {
      kind: "scheduleView",
      term: "cache-pair",
      from: "2026-09-28",
      to: "2026-12-02",
    },
  ];

  it("shares one read per revision between two consumers that mount together", async () => {
    const workspace = workspaceFixture({ courses: [placedCourse] });
    const core = installFakeCore({
      workspace,
      revision: "rev:schedule-share",
      queries: {
        workspaceOverview: (_q: unknown, live: FakeCore) =>
          overviewFor(live.workspace, { term: "cache-pair" }),
        scheduleView: (_q: unknown, live: FakeCore) =>
          scheduleViewFor(live.workspace, { term: "cache-pair" }),
      },
    });
    probe = await mount(<SchedulePair queries={twoQueries} />);

    await until(() =>
      expect(probe!.host.querySelector(".probe-one")?.textContent).toContain(
        "cache-pair",
      ),
    );
    const calls = core.commands("schedule_query");
    // Exactly one read carried both consumers' queries.
    expect(calls).toHaveLength(1);
    expect(calls[0].args.queries).toEqual(twoQueries);
    expect(probe.host.querySelector(".probe-two")?.textContent).toContain(
      "cache-pair",
    );
  }, 20_000);

  it("shares the identical query with a consumer that joins later", async () => {
    const workspace = workspaceFixture({ courses: [placedCourse] });
    const core = installFakeCore({
      workspace,
      revision: "rev:schedule-identical",
      queries: {
        workspaceOverview: (_q: unknown, live: FakeCore) =>
          overviewFor(live.workspace, { term: "cache-identical" }),
      },
    });
    const query: ScheduleQuery = {
      kind: "workspaceOverview",
      term: "cache-identical",
    };
    probe = await mount(<ScheduleConsumer queries={[query]} label="one" />);
    await until(() =>
      expect(probe!.host.querySelector(".probe-one")?.textContent).toContain(
        "cache-identical",
      ),
    );
    expect(core.commands("schedule_query")).toHaveLength(1);

    await probe.unmount();
    probe = await mount(<ScheduleConsumer queries={[query]} label="two" />);
    await until(() =>
      expect(probe!.host.querySelector(".probe-two")?.textContent).toContain(
        "cache-identical",
      ),
    );
    expect(core.commands("schedule_query")).toHaveLength(1);
  }, 20_000);

  it("re-reads when the workspace content revision changes", async () => {
    const workspace = workspaceFixture({ courses: [placedCourse] });
    const next = workspaceFixture({ courses: [clashingCourse] });
    const core = installFakeCore({
      workspace,
      revision: "rev:content-1",
      queries: {
        workspaceOverview: (_q: unknown, live: FakeCore) =>
          overviewFor(live.workspace, { term: "cache-revision" }),
      },
    });
    await act(async () => {
      await storeModule.workspaceStore.reload();
    });
    probe = await mount(
      <ScheduleConsumer
        queries={[{ kind: "workspaceOverview", term: "cache-revision" }]}
        label="one"
      />,
    );
    await until(() =>
      expect(probe!.host.querySelector(".probe-one")?.textContent).toContain(
        "cache-revision",
      ),
    );
    expect(core.commands("schedule_query")).toHaveLength(1);

    await act(async () => {
      core.publish(next, "rev:content-2");
      await storeModule.workspaceStore.reload();
    });
    await until(() => expect(core.commands("schedule_query")).toHaveLength(2));
    expect(core.commands("schedule_query").at(-1)?.args.queries).toEqual([
      { kind: "workspaceOverview", term: "cache-revision" },
    ]);
  }, 20_000);

  it("ignores a late response once the requested quarter changed", async () => {
    const workspace = workspaceFixture({ courses: [placedCourse] });
    let holdLate = true;
    let releaseLate!: () => void;
    const late = new Promise<void>((resolve) => {
      releaseLate = resolve;
    });
    const core = installFakeCore({
      workspace,
      revision: "rev:late-quarter",
      handle: (command, args, live) => {
        if (command === "schedule_query") {
          const term = (args.queries as { term?: string }[])[0]?.term;
          if (term === "late-quarter" && holdLate)
            return late.then(() =>
              scheduleAnswer(
                "workspaceOverview",
                overviewFor(live.workspace, { term: "late-quarter" }),
                "rev:late-quarter",
              ),
            );
          if (term === "quick-quarter")
            return scheduleAnswer(
              "workspaceOverview",
              overviewFor(live.workspace, { term: "quick-quarter" }),
              "rev:late-quarter",
            );
        }
        return undefined;
      },
    });
    probe = await mount(
      <ScheduleSwitchProbe initial="late-quarter" next="quick-quarter" />,
    );
    expect(probe.host.querySelector(".probeTerm")?.textContent).toBe("");

    await click(probe.host.querySelector(".probeSwitch"));
    await until(() =>
      expect(probe!.host.querySelector(".probeTerm")?.textContent).toBe(
        "quick-quarter",
      ),
    );

    holdLate = false;
    await act(async () => {
      releaseLate();
      await late;
    });
    expect(probe.host.querySelector(".probeTerm")?.textContent).toBe(
      "quick-quarter",
    );
    expect(
      core
        .commands("schedule_query")
        .map((call) => (call.args.queries as { term?: string }[])[0]?.term),
    ).toEqual(["late-quarter", "quick-quarter"]);
  }, 20_000);

  it("retains the last complete calendar while a replacement is pending", async () => {
    const workspace = workspaceFixture({ courses: [placedCourse] });
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    installFakeCore({
      workspace,
      revision: "rev:retained-quarter",
      handle: (command, args, live) => {
        if (command !== "schedule_query") return undefined;
        const term = (args.queries as { term: string }[])[0].term;
        const answer = () =>
          scheduleAnswer(
            "workspaceOverview",
            overviewFor(live.workspace, { term }),
            "rev:retained-quarter",
          );
        return term === "replacement-quarter" ? gate.then(answer) : answer();
      },
    });
    probe = await mount(
      <ScheduleSwitchProbe
        initial="retained-quarter"
        next="replacement-quarter"
      />,
    );
    await until(() =>
      expect(probe!.host.querySelector(".probeTerm")?.textContent).toBe(
        "retained-quarter",
      ),
    );
    await click(probe.host.querySelector(".probeSwitch"));
    expect(probe.host.querySelector(".probeTerm")?.textContent).toBe(
      "retained-quarter",
    );
    expect(
      probe.host.querySelector(".probeTerm")?.getAttribute("data-pending"),
    ).toBe("yes");
    await act(async () => {
      release();
      await gate;
    });
    await until(() =>
      expect(probe!.host.querySelector(".probeTerm")?.textContent).toBe(
        "replacement-quarter",
      ),
    );
    expect(
      probe.host.querySelector(".probeTerm")?.getAttribute("data-pending"),
    ).toBe("no");
  }, 20_000);

  it("retry re-issues the read without a remount and clears the error", async () => {
    const workspace = workspaceFixture({ courses: [placedCourse] });
    let failed = false;
    const core = installFakeCore({
      workspace,
      revision: "rev:retry-hook",
      handle: (command, args, live) => {
        if (command === "schedule_query") {
          const term = (args.queries as { term?: string }[])[0]?.term;
          if (term === "retry-quarter" && !failed) {
            failed = true;
            throw new Error("Core refused the read");
          }
          if (term === "retry-quarter")
            return scheduleAnswer(
              "workspaceOverview",
              overviewFor(live.workspace, { term: "retry-quarter" }),
              "rev:retry-hook",
            );
        }
        return undefined;
      },
    });
    probe = await mount(
      <ScheduleSwitchProbe initial="retry-quarter" next="retry-quarter" />,
    );
    await until(() =>
      expect(probe!.host.querySelector(".probeError")?.textContent).toBe(
        "Core refused the read",
      ),
    );
    expect(core.commands("schedule_query")).toHaveLength(1);

    await click(probe.host.querySelector(".probeRetry"));
    await until(() =>
      expect(probe!.host.querySelector(".probeTerm")?.textContent).toBe(
        "retry-quarter",
      ),
    );
    expect(probe.host.querySelector(".probeError")?.textContent).toBe("");
    expect(
      probe.host.querySelector(".probeTerm")?.getAttribute("data-pending"),
    ).toBe("no");
    expect(core.commands("schedule_query")).toHaveLength(2);
  }, 20_000);
});

describe("the finder owner", () => {
  it("shows the latest typed search when an older one answers last", async () => {
    let releaseOld!: () => void;
    const old = new Promise<void>((resolve) => {
      releaseOld = resolve;
    });
    const core = installFakeCore({
      workspace: workspaceFixture(),
      revision: "rev:finder-latest",
      handle: (command, args) => {
        if (command !== "finder_query") return undefined;
        const request = args.request as Record<string, unknown>;
        return request.input === "late"
          ? old.then(() => finderAnswer(request))
          : finderAnswer(request);
      },
    });
    probe = await mount(<FinderProbe term="finder-latest" nextTerm="x" />);
    await click(probe.host.querySelector(".typeLate"));
    await click(probe.host.querySelector(".typeQuick"));
    await until(() =>
      expect(probe!.host.querySelector(".shownInput")?.textContent).toBe(
        "quick",
      ),
    );
    await act(async () => {
      releaseOld();
      await old;
    });
    expect(probe.host.querySelector(".shownInput")?.textContent).toBe("quick");
    expect(
      core
        .commands("finder_query")
        .map((call) => (call.args.request as { input?: string }).input),
    ).toEqual(["", "late", "quick"]);
  }, 20_000);

  it("feeds each rapid action the keys Core returned for the previous one", async () => {
    const core = installFakeCore({
      workspace: workspaceFixture(),
      revision: "rev:queue-rapid",
      handle: (command, args) => {
        if (command === "finder_query")
          return finderAnswer(args.request as Record<string, unknown>);
        return undefined;
      },
    });
    probe = await mount(<FinderProbe term="queue" nextTerm="queue-next" />);

    await click(probe.host.querySelector(".queueRapid"));
    await until(() =>
      expect(probe!.host.querySelector(".queueKeys")?.textContent).toBe("A,B"),
    );
    const calls = actions(core);
    expect(calls).toHaveLength(2);
    expect(calls[0].args.request).toMatchObject({
      keys: [],
      action: { kind: "toggleTime", key: "A", append: false },
    });
    expect(calls[1].args.request).toMatchObject({
      keys: ["A"],
      action: { kind: "toggleTime", key: "B", append: true },
    });
  }, 20_000);

  it("starts a new session on a term change and drops an in-flight answer", async () => {
    let holdAnswer = true;
    let releaseAnswer!: () => void;
    const pending = new Promise<void>((resolve) => {
      releaseAnswer = resolve;
    });
    const core = installFakeCore({
      workspace: workspaceFixture(),
      revision: "rev:queue-term",
      handle: (command, args) => {
        if (command === "finder_query") {
          const incoming = args.request as Record<string, unknown>;
          if (incoming.term === "queue-late" && incoming.action && holdAnswer)
            return pending.then(() => finderAnswer(incoming));
          return finderAnswer(incoming);
        }
        return undefined;
      },
    });
    probe = await mount(
      <FinderProbe term="queue-late" nextTerm="queue-next" />,
    );

    await click(probe.host.querySelector(".queueA"));
    await click(probe.host.querySelector(".queueTerm"));
    holdAnswer = false;
    await act(async () => {
      releaseAnswer();
      await pending;
    });
    // The old answer was dropped: the keys were never touched.
    expect(probe.host.querySelector(".queueKeys")?.textContent).toBe("");

    await click(probe.host.querySelector(".queueB"));
    await until(() =>
      expect(probe!.host.querySelector(".queueKeys")?.textContent).toBe("B"),
    );
    expect(core.commands("finder_query").at(-1)?.args.request).toMatchObject({
      keys: [],
      term: "queue-next",
    });
  }, 20_000);

  it("replace sets the keys the next action starts from", async () => {
    const core = installFakeCore({
      workspace: workspaceFixture(),
      revision: "rev:queue-replace",
      handle: (command, args) => {
        if (command === "finder_query")
          return finderAnswer(args.request as Record<string, unknown>);
        return undefined;
      },
    });
    probe = await mount(<FinderProbe term="queue" nextTerm="queue-next" />);

    await click(probe.host.querySelector(".queueReplace"));
    expect(probe.host.querySelector(".queueKeys")?.textContent).toBe("X");

    await click(probe.host.querySelector(".queueB"));
    await until(() =>
      expect(probe!.host.querySelector(".queueKeys")?.textContent).toBe("X,B"),
    );
    expect(actions(core).at(-1)?.args.request).toMatchObject({
      keys: ["X"],
    });
  }, 20_000);
});

describe("First school connection", () => {
  it("offers account setup on an empty Home and opens the real account page", async () => {
    app = await mountShell(shellOptions({ workspace: workspaceFixture() }), {
      route: paths.home(),
    });
    const setup = await until(() => {
      const element = app!.host.querySelector(
        '[aria-label="School account setup"]',
      );
      expect(element).not.toBeNull();
      return element!;
    });
    expect(setup.textContent).toContain(
      "Tokyo Tech for your timetable and course search",
    );
    await click(setup.querySelector("button"));
    await until(() =>
      expect(window.location.hash).toBe(`#${paths.settings("accounts")}`),
    );
    await until(() =>
      expect(app!.host.querySelector(".settingsPane")?.textContent).toContain(
        "Science Tokyo Portal",
      ),
    );
  });

  it("greets a first launch with both language choices, applies them live and stays closed", async () => {
    const workspace = workspaceFixture();
    delete workspace.settings.dataNoticeAcknowledgedAt;
    const options = shellOptions({
      workspace,
      apply: (intent, core) => {
        if (intent.kind === "replaceSettings")
          core.publish(
            {
              ...core.workspace,
              settings: intent.settings as Workspace["settings"],
            },
            "revision:welcome-language",
          );
      },
    });
    app = await mountShell(options, { route: paths.home(), guide: true });
    // The data notice comes first; the welcome sheet waits behind it.
    const notice = await until(() => {
      const dialogs = [
        ...document.querySelectorAll<HTMLElement>('[role="dialog"]'),
      ];
      expect(dialogs.map((dialog) => dialog.textContent).join()).toContain(
        "Before you rely on this data",
      );
      expect(dialogs.map((dialog) => dialog.textContent).join()).not.toContain(
        "Welcome to IsctCompanion",
      );
      return dialogs[0];
    });
    await click(
      [...notice.querySelectorAll("button")].find(
        (button) => button.textContent === "I understand",
      ),
    );
    const sheet = await until(() => {
      const dialog = document.querySelector<HTMLElement>('[role="dialog"]');
      expect(dialog?.textContent).toContain("Welcome to IsctCompanion");
      return dialog!;
    });
    // The school-content language is offered beside the UI language.
    expect(sheet.textContent).toContain("Course and syllabus language");
    const japanese = [...sheet.querySelectorAll("label")].find(
      (label) => label.textContent === "日本語",
    );
    await click(japanese);
    await until(() => expect(app!.core.workspace.settings.locale).toBe("ja"));
    await until(() =>
      expect(sheet.textContent).toContain("IsctCompanion へようこそ"),
    );
    expect(app.core.workspace.settings.contentLanguage ?? "auto").toBe("auto");

    const later = [...sheet.querySelectorAll("button")].find(
      (button) => button.textContent === "あとで",
    );
    await click(later);
    await until(() =>
      expect(
        document.querySelector('[role="dialog"]')?.textContent ?? "",
      ).not.toContain("IsctCompanion へようこそ"),
    );
    // The next window on this device opens straight to the page.
    const stored = localStorage.getItem("isct-companion-guide");
    expect(JSON.parse(stored ?? "{}")).toMatchObject({ welcome: true });
  });

  it("keeps the setup prompt out of an existing timetable", async () => {
    const workspace = workspaceFixture({ courses: [placedCourse] });
    app = await mountShell(shellOptions({ workspace }), {
      route: paths.home(),
    });
    expect(
      app.host.querySelector('[aria-label="School account setup"]'),
    ).toBeNull();
  });
});

describe("workspace column sizing", () => {
  it("resizes, sticks then collapses, preserves content, and reveals the lecture sidebar from the rail", async () => {
    const { ResizableWorkspace } =
      await import("@app/components/ResizableWorkspace");
    // jsdom has no layout engine; give the real splitter a measured 1000px workspace.
    const width = vi
      .spyOn(HTMLElement.prototype, "offsetWidth", "get")
      .mockImplementation(function (this: HTMLElement) {
        return this.matches("[data-separator]")
          ? 20
          : this.matches("[data-panel]")
            ? 490
            : 1000;
      });
    const height = vi
      .spyOn(HTMLElement.prototype, "offsetHeight", "get")
      .mockReturnValue(600);
    const rect = vi
      .spyOn(HTMLElement.prototype, "getBoundingClientRect")
      .mockImplementation(function (this: HTMLElement) {
        if (this.matches('[data-panel][id$="-first"]'))
          return new DOMRect(0, 0, 392, 600);
        if (this.matches('[data-panel][id$="-second"]'))
          return new DOMRect(412, 0, 588, 600);
        if (this.matches("[data-separator]"))
          return new DOMRect(392, 0, 20, 600);
        return new DOMRect(0, 0, 1000, 600);
      });
    const left = vi
      .spyOn(HTMLElement.prototype, "offsetLeft", "get")
      .mockImplementation(function (this: HTMLElement) {
        return this.matches('[data-panel][id$="-second"]')
          ? 412
          : this.matches("[data-separator]")
            ? 392
            : 0;
      });
    const browserObserver = window.ResizeObserver;
    window.ResizeObserver = class implements ResizeObserver {
      constructor(private callback: ResizeObserverCallback) {}
      observe(target: Element) {
        queueMicrotask(() =>
          this.callback(
            [
              {
                target,
                contentRect: new DOMRect(0, 0, 1000, 600),
                borderBoxSize: [{ inlineSize: 1000, blockSize: 600 }],
              } as unknown as ResizeObserverEntry,
            ],
            this,
          ),
        );
      }
      unobserve() {}
      disconnect() {}
    };
    localStorage.clear();
    function Workbench({ enabled = true }: { enabled?: boolean }) {
      const [collapsed, setCollapsed] = useState(false);
      return (
        <ResizableWorkspace
          id="test-columns"
          firstCollapsible
          onFirstCollapse={setCollapsed}
          enabled={enabled}
          firstSize="40%"
          firstMin="20%"
          secondMin="30%"
        >
          <textarea aria-label="Draft" defaultValue="Unsaved draft" />
          <p>Reading pane {collapsed ? "collapsed" : "expanded"}</p>
        </ResizableWorkspace>
      );
    }
    try {
      probe = await mount(<Workbench />);
      const separator = () =>
        probe!.host.querySelector<HTMLElement>('[role="separator"]')!;
      const beforeDrag = Number(separator().getAttribute("aria-valuenow"));
      const pointer = (kind: string, x: number) =>
        Object.assign(
          new MouseEvent(kind, {
            bubbles: true,
            cancelable: true,
            clientX: x,
            clientY: 200,
            buttons: kind === "pointerup" ? 0 : 1,
          }),
          { pointerId: 1, pointerType: "mouse", isPrimary: true },
        );
      await act(async () => {
        separator().dispatchEvent(pointer("pointerdown", 402));
        separator().dispatchEvent(pointer("pointermove", 450));
        separator().dispatchEvent(pointer("pointerup", 450));
      });
      expect(Number(separator().getAttribute("aria-valuenow"))).toBeGreaterThan(
        beforeDrag,
      );
      const initial = Number(separator().getAttribute("aria-valuenow"));
      await act(async () =>
        separator().dispatchEvent(
          new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }),
        ),
      );
      const resized = Number(separator().getAttribute("aria-valuenow"));
      expect(resized).toBeGreaterThan(initial);

      // The minimum holds for a short drag, then gives way to collapse.
      await act(async () =>
        separator().dispatchEvent(
          new KeyboardEvent("keydown", { key: "Home", bubbles: true }),
        ),
      );
      await act(async () =>
        separator().dispatchEvent(
          new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }),
        ),
      );
      expect(Number(separator().getAttribute("aria-valuenow"))).toBe(20);
      await act(async () => {
        separator().dispatchEvent(pointer("pointerdown", 402));
        separator().dispatchEvent(pointer("pointermove", 382));
      });
      expect(Number(separator().getAttribute("aria-valuenow"))).toBe(20);
      await act(async () => {
        separator().dispatchEvent(pointer("pointermove", 342));
        separator().dispatchEvent(pointer("pointerup", 342));
      });
      expect(Number(separator().getAttribute("aria-valuenow"))).toBe(0);
      expect(probe.host.textContent).toContain("Reading pane collapsed");
      await probe.unmount();
      probe = await mount(<Workbench />);
      expect(Number(separator().getAttribute("aria-valuenow"))).toBe(0);
      await act(async () =>
        separator().dispatchEvent(
          new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }),
        ),
      );
      expect(Number(separator().getAttribute("aria-valuenow"))).toBeGreaterThan(
        0,
      );
      expect(probe.host.textContent).toContain("Reading pane expanded");
      const expanded = Number(separator().getAttribute("aria-valuenow"));
      const draft = probe.host.querySelector("textarea")!;
      await typeInto(draft, "Still writing");
      await act(async () => probe!.root.render(<Workbench enabled={false} />));
      expect(probe.host.querySelector("textarea")).toBe(draft);
      expect(draft.value).toBe("Still writing");
      await act(async () => probe!.root.render(<Workbench />));
      expect(probe.host.querySelector("textarea")).toBe(draft);
      expect(Number(separator().getAttribute("aria-valuenow"))).toBe(expanded);
      await probe.unmount();
      probe = await mount(<Workbench />);
      expect(Number(separator().getAttribute("aria-valuenow"))).toBe(expanded);
      await act(async () =>
        separator().dispatchEvent(
          new KeyboardEvent("keydown", { key: "End", bubbles: true }),
        ),
      );
      expect(
        Number(separator().getAttribute("aria-valuenow")),
      ).toBeLessThanOrEqual(70);
      await probe.unmount();
      probe = undefined;
      const matchMedia = vi
        .spyOn(window, "matchMedia")
        .mockImplementation((query) => ({
          matches: query === "(min-width: 769px)",
          media: query,
          onchange: null,
          addEventListener() {},
          removeEventListener() {},
          addListener() {},
          removeListener() {},
          dispatchEvent: () => false,
        }));
      try {
        app = await mountShell(
          shellOptions({
            workspace: workspaceFixture({ courses: [placedCourse] }),
          }),
          { route: paths.lectures() },
        );
        await click(
          await required(
            () => app!.host.querySelector<HTMLButtonElement>(".lectureRow"),
            "lecture row",
          ),
        );
        const toggle = await required(
          () =>
            app!.host.querySelector<HTMLButtonElement>(
              '[aria-label="Toggle sidebar"]',
            ),
          "sidebar toggle",
        );
        expect(toggle.closest(".lmsWorkspaceHeader")).not.toBeNull();
        const list = app.host.querySelector<HTMLElement>("#lecture-sidebar")!;
        const filter = list.querySelector<HTMLInputElement>(
          'input[type="search"]',
        )!;
        await typeInto(filter, "Keep this filter");
        await click(toggle);
        await until(() =>
          expect(toggle.getAttribute("aria-expanded")).toBe("false"),
        );
        expect(list.hasAttribute("inert")).toBe(true);
        const rail = app.host.querySelector<HTMLButtonElement>(
          '.rail button[title^="Lectures"]',
        )!;
        await act(async () =>
          rail.dispatchEvent(new Event("pointerover", { bubbles: true })),
        );
        expect(
          app.host
            .querySelector(".lectureDesk")!
            .getAttribute("data-sidebar-floating"),
        ).toBe("true");
        expect(list.hasAttribute("inert")).toBe(false);
        await act(async () =>
          rail.dispatchEvent(new Event("pointerout", { bubbles: true })),
        );
        await advance(250);
        expect(list.hasAttribute("inert")).toBe(true);
        await click(toggle);
        expect(toggle.getAttribute("aria-expanded")).toBe("true");
        expect(app.host.querySelector("#lecture-sidebar")).toBe(list);
        expect(filter.value).toBe("Keep this filter");
      } finally {
        await app?.unmount();
        app = undefined;
        matchMedia.mockRestore();
      }
    } finally {
      await probe?.unmount();
      probe = undefined;
      width.mockRestore();
      height.mockRestore();
      rect.mockRestore();
      left.mockRestore();
      window.ResizeObserver = browserObserver;
      localStorage.clear();
    }
  });
});

it("renders Core's sync snapshot in the footer and sends hints for refresh and opened pages", async () => {
  app = await mountShell(shellOptions({ workspace: workspaceFixture() }));
  // Core decides what is due and how far a run is (`sync_engine` and its
  // tests); the shell only picks the row to show and names it.
  const run = (overrides: Record<string, unknown>) => ({
    run: "run",
    dataset: "lms.course",
    label: { key: "sync.dataset.lms.course", args: {} },
    priority: "visible",
    done: 0,
    total: null,
    current: null,
    scope: null,
    held: null,
    failure: null,
    ...overrides,
  });
  const source = (name: string, lastUpdated: string | null) => ({
    source: name,
    lastUpdated,
    running: false,
    held: null,
    failure: null,
    session: null,
    maintenance: null,
  });
  let generation = 1;
  const emit = (runs: unknown[], sources = [source("lms", null)]) =>
    act(async () =>
      app!.core.emit("sync-snapshot", {
        generation: ++generation,
        resetting: false,
        runs,
        sources,
      }),
    );
  const status = () => app!.host.querySelector(".lmsStatus")!;
  const hints = () =>
    app!.core.calls
      .filter((call) => call.command === "sync_hint")
      .map((call) => call.args.hint);

  await emit([run({ done: 7, total: 62 })]);
  await until(() => {
    expect(status().getAttribute("data-kind")).toBe("bar");
    expect(status().textContent).toContain("Course content · 7 / 62");
    expect(status().querySelector("progress")!.value).toBe(7);
  });
  // A run of unknown size never gets a bar or a guessed percentage.
  await emit([run({ done: 3 })]);
  await until(() => {
    expect(status().getAttribute("data-kind")).toBe("busy");
    expect(status().querySelector("progress")).toBeNull();
    expect(status().textContent).not.toContain("%");
  });
  // Background-only work is a spinner and a quiet note.
  await emit([run({ priority: "background", done: 1, total: 40 })]);
  await until(() => {
    expect(status().getAttribute("data-kind")).toBe("background");
    expect(status().querySelector(".schoolStatusSpinner")).not.toBeNull();
    expect(status().querySelector("progress")).toBeNull();
  });
  await emit([run({ held: { kind: "cellular" } })]);
  await until(() =>
    expect(status().textContent).toContain("Waiting for Wi-Fi"),
  );
  await emit([], [source("lms", null)]);
  await until(() => {
    expect(status().getAttribute("data-kind")).toBe("idle");
    expect(status().textContent).toContain("Never updated");
  });
  await emit([
    run({
      run: "grades",
      dataset: "kyomu.page",
      scope: "grades",
      failure: { kind: "timeout", keysFailed: 1, retryAt: null },
    }),
  ]);
  const retry = await required(
    () =>
      [...status().querySelectorAll<HTMLButtonElement>("button")].find(
        (button) => button.textContent === "Retry",
      ),
    "the failed dataset's Retry",
  );
  expect(status().textContent).toContain("教務Web pages failed · timed out");
  await click(retry);
  const force = await required(
    () =>
      [...status().querySelectorAll<HTMLButtonElement>("button")].find(
        (button) => button.textContent === "Refresh now",
      ),
    "Refresh now",
  );
  await click(force);
  await until(() =>
    expect(hints()).toEqual(
      expect.arrayContaining([
        { kind: "refresh", scope: { kind: "site", site: "kyomu" } },
        { kind: "refresh", scope: { kind: "everything" } },
      ]),
    ),
  );
  await app.go(paths.timetable());
  await until(() =>
    expect(hints()).toContainEqual({
      kind: "open",
      view: { kind: "timetable" },
    }),
  );
  const timetableRefresh = await required(
    () =>
      [...app!.host.querySelectorAll<HTMLButtonElement>("button")].find(
        (button) => button.textContent?.trim() === "Refresh timetable",
      ),
    "the timetable refresh action",
  );
  await click(timetableRefresh);
  await until(() =>
    expect(hints()).toContainEqual({
      kind: "refresh",
      scope: { kind: "timetable" },
    }),
  );
  expect(
    app.core.calls.some((call) =>
      ["pull_school", "school_refresh_event"].includes(call.command),
    ),
  ).toBe(false);
});

/**
 * Proper names, product names and protocol words that stay in Latin script in
 * every UI language, plus the fixture's own course data. Anything else in
 * Latin script on a Chinese screen is an English string that skipped `t()`.
 */
const LATIN_ALLOWED = new Set(
  (
    "LMS Web Science Tokyo TokyoTech Tech Portal Mail TSUBAME ICS PDF URL ID " +
    "AI Agent MCP Matrix Wi-Fi Google Drive Microsoft OAuth iCloud CSS " +
    "JavaScript HTTP Streamable Bearer IsctCompanion GPA GPT ZIP macOS session " +
    "English https isct-companion Isct-Data test Placed CSC.T Q MB " +
    "A B C D E F G H I J " +
    // Course-search operators in the planner's placeholder are typed as is.
    "teacher room W"
  ).split(" "),
);

/** Latin-script words a person would see or hear on screen. */
function latinWords(root: HTMLElement): string[] {
  const words = new Set<string>();
  const add = (text: string | null | undefined) => {
    for (const word of text?.match(/[A-Za-z][A-Za-z'’.-]*/g) ?? [])
      if (!LATIN_ALLOWED.has(word.replace(/[.]$/, ""))) words.add(word);
  };
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode())
    if (!["STYLE", "SCRIPT"].includes(node.parentElement?.tagName ?? ""))
      add(node.textContent);
  for (const element of root.querySelectorAll(
    "[aria-label],[title],[placeholder]",
  ))
    for (const name of ["aria-label", "title", "placeholder"])
      add(element.getAttribute(name));
  return [...words];
}

it("shows every page and settings section in Chinese without English leftovers", async () => {
  const workspace = workspaceFixture({
    courses: [placedCourse],
    planItems: [planItemFor(placedCourse)],
  });
  workspace.settings.locale = "zh";
  app = await mountShell(shellOptions({ workspace }));
  const sections = [
    "accounts",
    "study",
    "notifications",
    "calendar",
    "appearance",
    "files",
    "backup",
    "sources",
    "agent",
    "developer",
    "about",
  ] as const;
  const leaks: Record<string, string[]> = {};
  for (const route of [
    paths.home(),
    paths.home("week"),
    paths.timetable(),
    paths.lectures(),
    paths.web(),
    ...sections.map((section) => paths.settings(section)),
  ]) {
    await app.go(route);
    await advance(300);
    const words = latinWords(app.host);
    if (words.length) leaks[route] = words;
  }
  expect(leaks).toEqual({});
}, 60_000);
