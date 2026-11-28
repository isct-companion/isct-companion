// @vitest-environment jsdom
import { act, useState } from "react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import {
  advance,
  buttonByText,
  click,
  chooseOption,
  courseFor,
  installFakeCore,
  mount,
  mountApp,
  offering,
  overviewFor,
  planItemFor,
  planningWeekFor,
  lectureListFor,
  typeInto,
  until,
  watchConsoleErrors,
  workspaceFixture,
  type FakeCore,
  type FakeCoreOptions,
  type Mounted,
  type MountedApp,
  type QueryTable,
  schoolProfiles,
  FINDER_READS,
  SCHEDULE_READS,
} from "@tests/harness";
import { paths } from "@app/app/routes";
import { notify } from "@app/components/notifications/notify";
import i18n from "@app/i18n";
import type { Workspace } from "@app/lib/model";
import type { FinderAction, FinderPage } from "@app/generated/persistence";

/**
 * The whole shell over a scripted Core: hash routing, page retention, keyboard
 * shortcuts, the native-navigation blocker and the query caches that only
 * exist inside React hooks. Each case drives a path through and checks where
 * it ends: the route, the retained nodes and their state, what Core stored or
 * was asked. How a screen reads is a person's check in
 * docs/verification/manual-checks.yaml, not an assertion here.
 *
 * Local helpers exist where the harness has no builder: `shellOptions` (a
 * navigable workspace with the school-web commands answered), `mountShell`
 * (adopts this test's workspace before mounting, because the harness reuses one
 * module-level store), `emptyFinder`, and the `ScheduleProbe` / `FinderProbe`
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

/* ------------------------------------------------------------------ *
 * A navigable shell over the scripted host.
 * ------------------------------------------------------------------ */

const CAPABILITIES = {
  schoolImport: true,
  schoolWeb: true,
  notifications: false,
  backgroundSchedule: false,
  cloudSync: false,
};

type Handle = NonNullable<FakeCoreOptions["handle"]>;

/** Answers every non-query command the shell issues on the routes we cover. */
const baseHandle: Handle = (command, args) => {
  switch (command) {
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
      const entry = args.entry as { kind: string; url?: string };
      return {
        site: args.site,
        url: entry.url ?? "https://portal.isct.ac.jp/resident",
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
        profiles: schoolProfiles("not-configured", "not-configured"),
      };
    default:
      return undefined;
  }
};

function shellQueries(): QueryTable {
  return {
    workspaceOverview: (_query: unknown, core: FakeCore) =>
      overviewFor(core.workspace),
    lectureList: (_query: unknown, core: FakeCore) =>
      lectureListFor(core.workspace),
    planningWeek: (_query: unknown, core: FakeCore) =>
      planningWeekFor(core.workspace),
    taskView: {
      byTerm: {},
      nearestByCourse: {},
      deadlines: {},
      states: {},
      handInAvailability: {},
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
  placedOfferingIds: [],
  commitKey: null,
  pasteHandled: false,
  pasted: [],
  missingCodes: [],
  ineligibleCodes: [],
  catalogVersion: 0,
  contentRevision: "",
  reads: FINDER_READS,
  seq: 0,
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
  it("keeps valid deep links and replaces unknown, unsupported and deleted-course routes", async () => {
    consoleWatch = watchConsoleErrors();
    const workspace = workspaceFixture({
      courses: [placedCourse],
      planItems: [planItemFor(placedCourse)],
    });
    const options = shellOptions({ workspace, revision: "rev:redirect" });
    app = await mountShell(options, { route: paths.timetable() });
    const { core, router } = app;
    await until(() =>
      expect(app!.host.querySelector(".timetableMain")).not.toBeNull(),
    );
    expect(window.location.hash).toBe(`#${paths.timetable()}`);

    const visit = async (route: string, lands: string) => {
      await app!.go(route);
      await advance(20);
      await until(() => expect(window.location.hash).toBe(`#${lands}`));
      // A replaced route leaves no entry for Back to return to.
      expect(router.state.historyAction, route).toBe(
        lands === route ? "PUSH" : "REPLACE",
      );
    };
    // Course files are a desktop page, so this build keeps it.
    await visit(paths.settings("files"), paths.settings("files"));
    await visit(
      paths.lectures(placedCourse.id),
      paths.lectures(placedCourse.id),
    );
    await visit("/settings/not-a-section", paths.settings("accounts"));
    // A developer-tier page stays closed until Developer options are on.
    await visit(paths.settings("diagnostics"), paths.settings("accounts"));
    await visit("/decidedly-not-a-route", paths.home());

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
    await visit(paths.lectures(placedCourse.id), paths.lectures());

    consoleWatch.expectClean();
    consoleWatch.restore();
    consoleWatch = undefined;
  }, 20_000);

  it("replaces a desktop settings section a preview build lacks", async () => {
    // The native bootstrap injects the build target; a preview build without
    // desktop file settings must not leave the person on an unknown page.
    (
      window as unknown as Record<string, unknown>
    ).__ISCT_COMPANION_NATIVE_TARGET__ = "browser-preview";
    app = await mountShell(
      shellOptions({ workspace: workspaceFixture(), revision: "rev:platform" }),
      { route: paths.settings("files") },
    );
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
      () => host.querySelector<HTMLInputElement>('button[data-value="zh"]'),
      "the Chinese language choice",
    );

    await app.go(paths.home());
    await app.go(paths.settings("appearance"));

    expect(host.querySelector('button[data-value="zh"]')).toBe(language);
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
          lectureList: (_query: unknown, core: FakeCore) => ({
            ...lectureListFor(core.workspace),
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
    // The same picker node: the quarter chosen above was never reset.
    expect(host.querySelector('.lectureTerm [role="combobox"]')).toBe(term);
  }, 20_000);

  it("hides an open menu with its page and brings it back still working", async () => {
    // Overlays used to portal to body, where Activity cannot hide them: an
    // open menu stayed over the next page and, with its effects disconnected,
    // could not close until its page returned.
    app = await mountShell(
      shellOptions({
        workspace: workspaceFixture({
          courses: [placedCourse],
          planItems: [planItemFor(placedCourse)],
        }),
        revision: "rev:overlay",
      }),
      { route: paths.timetable() },
    );
    const shortcut = (key: string) =>
      act(async () => {
        window.dispatchEvent(
          new KeyboardEvent("keydown", {
            key,
            metaKey: true,
            cancelable: true,
          }),
        );
      });
    // Radix unmounts a closed dropdown, so only ancestors say
    // whether the page took it away.
    const hiddenWithPage = (node: Element) => {
      for (let el = node.parentElement; el; el = el.parentElement)
        if (el.style.display === "none") return true;
      return false;
    };

    await click(
      await required(() => {
        const button = buttonByText(app!.host, i18n.t("common.exportICS"));
        return button && !button.disabled ? button : undefined;
      }, "the timetable export button"),
    );
    const menu = await required(
      () => document.querySelector('[role="menu"]'),
      "the open menu",
    );
    expect(hiddenWithPage(menu)).toBe(false);

    await shortcut("1");
    await until(() => expect(window.location.hash).toBe(`#${paths.home()}`));
    expect(menu.isConnected).toBe(true);
    expect(hiddenWithPage(menu)).toBe(true);

    await shortcut("2");
    await until(() =>
      expect(window.location.hash).toBe(`#${paths.timetable()}`),
    );
    expect(hiddenWithPage(menu)).toBe(false);
    // Its effects are back, so it closes like any open menu.
    await act(async () => {
      menu.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
    });
    await until(() =>
      expect(document.querySelector('[role="menu"]')).toBeNull(),
    );
  }, 20_000);

  it("holds navigation away from a page that owns the native view until hiding it succeeds", async () => {
    let hide: "ok" | "fail" | "hold" = "ok";
    let releaseHide!: () => void;
    const held = new Promise<boolean>((resolve) => {
      releaseHide = () => resolve(true);
    });
    app = await mountShell(
      shellOptions({
        workspace: workspaceFixture({ courses: [placedCourse] }),
        revision: "rev:blocker",
        handle: (command, args, core) => {
          if (command === "hide_school_web" && hide === "fail")
            throw new Error("Native hide failed");
          if (command === "hide_school_web" && hide === "hold") return held;
          return baseHandle(command, args, core);
        },
      }),
      { route: paths.home() },
    );
    const { core } = app;
    const hides = () => core.commands("hide_school_web").length;
    const opens = () => core.commands("open_school_site").length;
    const back = () =>
      act(async () => {
        window.history.back();
        await new Promise((resolve) => setTimeout(resolve, 40));
      });

    await app.go(paths.web());
    await until(() => expect(opens()).toBeGreaterThan(0));

    // Back while hiding fails puts the Web page's URL back.
    hide = "fail";
    let tried = hides();
    await back();
    await until(() => expect(hides()).toBeGreaterThan(tried));
    await advance(50);
    expect(window.location.hash).toBe(`#${paths.web()}`);

    hide = "ok";
    await back();
    await until(() => expect(window.location.hash).toBe(`#${paths.home()}`));

    // Leaving while hiding is still in flight waits for it.
    const opened = opens();
    await app.go(paths.web());
    await until(() => expect(opens()).toBeGreaterThan(opened));
    hide = "hold";
    await app.go(paths.home());
    expect(window.location.hash).toBe(`#${paths.web()}`);
    hide = "ok";
    await act(async () => {
      releaseHide();
      await held;
    });
    await until(() => expect(window.location.hash).toBe(`#${paths.home()}`));

    // An open lecture's reader owns the native view as well.
    await app.go(paths.lectures(placedCourse.id));
    await until(() =>
      expect(
        app!.host.querySelector(".lectureSlot:not([hidden])"),
      ).not.toBeNull(),
    );
    hide = "fail";
    tried = hides();
    await app.go(paths.home());
    await until(() => expect(hides()).toBeGreaterThan(tried));
    await advance(50);
    expect(window.location.hash).toBe(`#${paths.lectures(placedCourse.id)}`);
  }, 20_000);
});

/* ------------------------------------------------------------------ *
 * Keyboard shortcuts.
 * ------------------------------------------------------------------ */

describe("keyboard shortcuts", () => {
  it("routes ⌘1–6 to the rail pages and ⌘, to the section left in Settings", async () => {
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
      paths.tools(),
      paths.pastExams(),
      // ⌘, returns to the section left above.
      paths.settings("study"),
    ].entries()) {
      await act(async () => {
        window.dispatchEvent(
          new KeyboardEvent("keydown", {
            key: index === 6 ? "," : String(index + 1),
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
  it("adopts a change committed while its native subscription was still registering", async () => {
    let register!: () => void;
    const registered = new Promise<void>((resolve) => {
      register = resolve;
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
            return registered.then(() => 1);
          return baseHandle(command, args, core);
        },
      }),
      { route: paths.home() },
    );
    const { core } = app;
    await until(() =>
      expect(
        core.calls.some(
          (call) =>
            call.command === "plugin:event|listen" &&
            call.args.event === "workspace-changed",
        ),
      ).toBe(true),
    );

    // Another writer commits before the subscription exists, so no event
    // will ever name it: only a read after registration can see it.
    core.publish(core.workspace, "rev:ordering-2");
    await act(async () => {
      register();
      await registered;
    });
    await until(() =>
      expect(storeModule.workspaceStore.getState().revision).toBe(
        "rev:ordering-2",
      ),
    );
  }, 20_000);

  it("asks Core again for the timetable when a school change names what it shows", async () => {
    // The course ids each answer was built from, newest last.
    const answered: string[][] = [];
    const recorded =
      (build: (workspace: Workspace) => Record<string, unknown>) =>
      (_query: unknown, core: FakeCore) => {
        answered.push(core.workspace.courses.map((course) => course.id));
        return build(core.workspace);
      };
    app = await mountShell(
      shellOptions({
        workspace: workspaceFixture({ courses: [placedCourse] }),
        revision: "rev:changed",
        finder: emptyFinder,
        queries: {
          ...shellQueries(),
          workspaceOverview: recorded((workspace) => overviewFor(workspace)),
          planningWeek: recorded((workspace) => planningWeekFor(workspace)),
        },
      }),
      { route: paths.timetable() },
    );
    const { core } = app;
    await until(() => expect(answered).toContainEqual([placedCourse.id]));

    await act(async () => {
      core.publish(workspaceFixture({ courses: [clashingCourse] }));
      core.emit("workspace-changed", {
        seq: core.commit().seq,
        source: "school-pull",
        changes: ["courses", "freshness"],
      });
    });

    await until(() => expect(answered.at(-1)).toEqual([clashingCourse.id]));
  }, 20_000);
});

/* ------------------------------------------------------------------ *
 * Web-page residency.
 * ------------------------------------------------------------------ */

describe("web-page residency", () => {
  it("opens a requested school URL, keeps the page resident while hidden and resumes it", async () => {
    const url = "https://lms.s.isct.ac.jp/course/view.php?id=12";
    const course = courseFor(
      offering("202602421", "CSC.T252", "Placed", { id: "lms-12" }),
      { source: { provider: "lms", externalId: "12" }, url },
    );
    app = await mountShell(
      shellOptions({
        workspace: workspaceFixture({
          courses: [course],
          planItems: [planItemFor(course)],
        }),
        revision: "rev:resident",
        queries: { ...shellQueries(), courseDetail: { lmsUrl: url } },
      }),
      { route: paths.timetable() },
    );
    const { core, host } = app;
    const opened = () => core.commands("open_school_site");

    // An explicit link opens its own URL.
    await click(
      await required(
        () => host.querySelector<HTMLButtonElement>(".ttBlock.course"),
        "the placed course block",
      ),
    );
    await click(
      await required(
        () =>
          host.querySelector<HTMLButtonElement>(
            ".coursePanel .officialLink button",
          ),
        "the official LMS link",
      ),
    );
    await until(() =>
      expect(opened().at(-1)?.args.entry).toEqual({ kind: "go", url }),
    );
    expect(window.location.hash).toBe(`#${paths.web()}`);

    const address = await required(
      () => host.querySelector<HTMLInputElement>('[aria-label="Current URL"]'),
      "the school address bar",
    );
    await typeInto(address, "https://portal.isct.ac.jp/typed");
    const hides = core.commands("hide_school_web").length;
    await app.go(paths.home());
    await until(() =>
      expect(core.commands("hide_school_web").length).toBeGreaterThan(hides),
    );
    // The page is hidden, not destroyed: same node, same typed address.
    expect(host.querySelector('[aria-label="Current URL"]')).toBe(address);
    expect(address.value).toBe("https://portal.isct.ac.jp/typed");

    // An ordinary return re-shows the resident session without a new URL.
    const before = opened().length;
    await app.go(paths.web());
    await until(() => expect(opened().length).toBeGreaterThan(before));
    expect(opened().at(-1)?.args.entry).toEqual({ kind: "resume" });
    expect(host.querySelector('[aria-label="Current URL"]')).toBe(address);

    // A typed address reaches the native view verbatim.
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
  }, 20_000);
});

/* ------------------------------------------------------------------ *
 * Store failures.
 * ------------------------------------------------------------------ */

describe("store failures", () => {
  it("keeps the page after a failed write and ties Retry and Undo to their own operations", async () => {
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

    await click(
      await required(
        () =>
          host.querySelector<HTMLInputElement>(
            '[data-setting="bilingual"] [role="switch"]',
          ),
        "the bilingual toggle",
      ),
    );
    await until(() =>
      expect(
        core.calls.some((call) => call.command === "mutate_workspace"),
      ).toBe(true),
    );
    await advance(50);
    // The failure is a notice; the page never falls into its error boundary.
    expect(host.querySelector(".pageError")).toBeNull();

    const action = (label: string) =>
      Array.from(document.querySelectorAll<HTMLButtonElement>("button")).find(
        (button) => button.textContent === label,
      );
    const retried: string[] = [];
    await act(async () => {
      notify.error(new Error("Disk is full"), () => retried.push("write"));
    });
    await click(await required(() => action("Retry saving"), "the retry"));
    expect(retried).toEqual(["write"]);

    const restored: string[] = [];
    const { confirmAction } = await import("@app/components/confirm");
    await act(async () => {
      notify.undoable("Removed the first exam", () => {
        restored.push("first");
      });
      notify.undoable("Removed the second exam", () => {
        restored.push("second");
      });
      notify.info("Another operation finished");
    });
    await required(() => action("Undo"), "the first undo");
    // A confirmation in between neither runs nor drops a pending Undo.
    let confirmed: Promise<boolean> | undefined;
    await act(async () => {
      confirmed = confirmAction("Delete this item?", "Delete");
    });
    await click(await required(() => action("Cancel"), "confirmation cancel"));
    expect(await confirmed).toBe(false);
    await click(await required(() => action("Undo"), "the first undo"));
    await click(await required(() => action("Undo"), "the second undo"));
    expect(restored).toEqual(["first", "second"]);
  }, 20_000);

  it("keeps an open announcement intact when another arrives, records only explicit answers, and closes withdrawn ones", async () => {
    const notice = (id: string, title: string) => ({
      id,
      level: "info" as const,
      title,
      body: `${title} body`,
      link: null,
    });
    const first = notice("a", "First announcement");
    const second = notice("b", "Second announcement");
    const third = notice("c", "Withdrawn announcement");
    let feed = { active: [first], pending: [first] };
    app = await mountShell(
      shellOptions({
        workspace: workspaceFixture(),
        revision: "rev:announcements",
        queries: {
          ...shellQueries(),
          announcements: () => feed,
        },
      }),
      { route: paths.home() },
    );
    const { core } = app;
    const dialog = () => document.querySelector<HTMLElement>('[role="dialog"]');
    const answers = () => core.mutations.map((entry) => entry.intent);
    const publishFeed = async (next: typeof feed) => {
      feed = next;
      await act(async () => {
        core.emit("workspace-changed", {
          seq: core.commit().seq,
          source: "reference-pull",
          changes: ["announcements"],
        });
      });
    };

    const firstDialog = await until(() => {
      expect(dialog()).not.toBeNull();
      return dialog()!;
    });

    // A newer announcement arriving does not replace the open one: if it
    // did, the answer recorded below would be A's, not B's.
    await publishFeed({ active: [second, first], pending: [second, first] });
    await advance(50);

    // Closing without an explicit choice records no receipt, then opens B.
    await act(async () => {
      firstDialog.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
    });
    const secondDialog = await until(() => {
      expect(dialog()).not.toBeNull();
      expect(dialog()).not.toBe(firstDialog);
      return dialog()!;
    });
    expect(core.mutations).toEqual([]);

    await click(
      [...secondDialog.querySelectorAll("button")].find(
        (button) => button.textContent === "Got it",
      ),
    );
    await until(() =>
      expect(answers()).toContainEqual({ kind: "answerAnnouncement", id: "b" }),
    );
    await until(() => expect(dialog()).toBeNull());

    // A withdrawn announcement closes itself without recording an answer.
    await publishFeed({ active: [third], pending: [third] });
    await until(() => expect(dialog()).not.toBeNull());
    await publishFeed({ active: [], pending: [] });
    await until(() => expect(dialog()).toBeNull());
    expect(answers()).toEqual([{ kind: "answerAnnouncement", id: "b" }]);
  }, 20_000);
});

/* ------------------------------------------------------------------ *
 * Chinese UI.
 * ------------------------------------------------------------------ */

describe("Chinese UI", () => {
  it("loads Chinese and saves an edit without touching the explicit school-content language", async () => {
    const workspace = workspaceFixture();
    workspace.settings.locale = "zh";
    workspace.settings.contentLanguage = "en";
    workspace.settings.theme = "paper";
    app = await mountShell(
      shellOptions({
        workspace,
        revision: "rev:zh",
        apply: (intent, core) => {
          if (intent.kind === "replaceSettings")
            core.publish(
              {
                ...core.workspace,
                settings: intent.settings as Workspace["settings"],
              },
              `${core.revision}+`,
            );
        },
      }),
      { route: paths.settings("appearance") },
    );
    const { host, core } = app;

    await until(() => expect(document.documentElement.lang).toBe("zh"));
    await click(
      await required(
        () =>
          host.querySelector(
            '[data-setting="theme"] [data-theme-value="moss"]',
          ),
        "the moss theme swatch",
      ),
    );
    await until(() => expect(core.workspace.settings.theme).toBe("moss"));
    expect(core.workspace.settings).toMatchObject({
      locale: "zh",
      contentLanguage: "en",
    });
  }, 20_000);
});

/* ------------------------------------------------------------------ *
 * Query-cache guarantees that live inside React hooks.
 * ------------------------------------------------------------------ */

type Overviewish = { term?: string; courses?: { id: string }[] };

/** One overview consumer that walks through `terms`, one step per click. */
function ScheduleProbe({ terms }: { terms: string[] }) {
  const [index, setIndex] = useState(0);
  const state = coreQuery.useScheduleQuery<Overviewish>({
    kind: "workspaceOverview",
    term: terms[index],
  });
  return (
    <div>
      <output
        className="probeValue"
        data-term={state.value?.term ?? ""}
        data-courses={
          state.value?.courses?.map((course) => course.id).join(",") ?? ""
        }
        data-pending={state.pending ? "yes" : "no"}
        data-error={state.error ?? ""}
      />
      <button
        className="probeNext"
        onClick={() => setIndex((current) => current + 1)}
      >
        next
      </button>
      <button className="probeRetry" onClick={() => state.retry?.()}>
        retry
      </button>
    </div>
  );
}

/** What the probe's hook currently holds. */
function probed(host: HTMLElement) {
  const output = host.querySelector(".probeValue")!;
  return {
    term: output.getAttribute("data-term"),
    courses: output.getAttribute("data-courses"),
    pending: output.getAttribute("data-pending"),
    error: output.getAttribute("data-error"),
  };
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

function scheduleAnswer(kind: string, value: unknown, revision: string) {
  return { revision, values: [{ kind, value }], reads: SCHEDULE_READS, seq: 0 };
}

describe("schedule query caches", () => {
  it("reads every view again when the store reloads", async () => {
    const core = installFakeCore({
      workspace: workspaceFixture({ courses: [placedCourse] }),
      revision: "rev:content-1",
      queries: {
        workspaceOverview: (_q: unknown, live: FakeCore) =>
          overviewFor(live.workspace, { term: "cache-revision" }),
      },
    });
    await act(async () => {
      await storeModule.workspaceStore.reload();
    });
    probe = await mount(<ScheduleProbe terms={["cache-revision"]} />);
    await until(() =>
      expect(probed(probe!.host).courses).toBe(placedCourse.id),
    );

    await act(async () => {
      core.publish(
        workspaceFixture({ courses: [clashingCourse] }),
        "rev:content-2",
      );
      await storeModule.workspaceStore.reload();
    });
    await until(() =>
      expect(probed(probe!.host).courses).toBe(clashingCourse.id),
    );
  }, 20_000);

  it("keeps the last complete answer while a new quarter loads and ignores an abandoned quarter's late answer", async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    installFakeCore({
      workspace: workspaceFixture({ courses: [placedCourse] }),
      revision: "rev:switch-quarter",
      handle: (command, args, live) => {
        if (command !== "schedule_query") return undefined;
        const term = (args.queries as { term: string }[])[0].term;
        const answer = () =>
          scheduleAnswer(
            "workspaceOverview",
            overviewFor(live.workspace, { term }),
            "rev:switch-quarter",
          );
        return term === "slow-quarter" ? gate.then(answer) : answer();
      },
    });
    probe = await mount(
      <ScheduleProbe
        terms={["retained-quarter", "slow-quarter", "quick-quarter"]}
      />,
    );
    await until(() =>
      expect(probed(probe!.host).term).toBe("retained-quarter"),
    );

    await click(probe.host.querySelector(".probeNext"));
    expect(probed(probe.host)).toMatchObject({
      term: "retained-quarter",
      pending: "yes",
    });

    // Leave the slow quarter before it answers.
    await click(probe.host.querySelector(".probeNext"));
    await until(() => expect(probed(probe!.host).term).toBe("quick-quarter"));
    await act(async () => {
      release();
      await gate;
    });
    expect(probed(probe.host)).toMatchObject({
      term: "quick-quarter",
      pending: "no",
    });
  }, 20_000);

  it("retry reads again without a remount and clears the error", async () => {
    let failed = false;
    installFakeCore({
      workspace: workspaceFixture({ courses: [placedCourse] }),
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
    probe = await mount(<ScheduleProbe terms={["retry-quarter"]} />);
    await until(() =>
      expect(probed(probe!.host).error).toBe("Core refused the read"),
    );

    await click(probe.host.querySelector(".probeRetry"));
    await until(() => expect(probed(probe!.host).term).toBe("retry-quarter"));
    expect(probed(probe.host)).toMatchObject({ error: "", pending: "no" });
  }, 20_000);
});

describe("the finder owner", () => {
  it("shows the latest typed search when an older one answers last", async () => {
    let releaseOld!: () => void;
    const old = new Promise<void>((resolve) => {
      releaseOld = resolve;
    });
    installFakeCore({
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
  }, 20_000);

  it("feeds each action the keys Core returned for the previous one, starting from a replaced set", async () => {
    installFakeCore({
      workspace: workspaceFixture(),
      revision: "rev:queue-rapid",
      handle: (command, args) => {
        if (command === "finder_query")
          return finderAnswer(args.request as Record<string, unknown>);
        return undefined;
      },
    });
    probe = await mount(<FinderProbe term="queue" nextTerm="queue-next" />);
    const keys = () => probe!.host.querySelector(".queueKeys")?.textContent;

    await click(probe.host.querySelector(".queueReplace"));
    expect(keys()).toBe("X");
    await click(probe.host.querySelector(".queueB"));
    await until(() => expect(keys()).toBe("X,B"));

    // Two actions in one tick: the second must start from the first's answer.
    await click(probe.host.querySelector(".queueRapid"));
    await until(() => expect(keys()).toBe("A,B"));
  }, 20_000);

  it("starts a new session on a term change and drops in-flight and queued actions", async () => {
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

    await click(probe.host.querySelector(".queueRapid"));
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
    // Core was asked about the new term.
    expect(core.commands("finder_query").at(-1)?.args.request).toMatchObject({
      term: "queue-next",
    });
  }, 20_000);
});

describe("First school connection", () => {
  it("offers account setup on an empty Home and opens the account page", async () => {
    app = await mountShell(shellOptions({ workspace: workspaceFixture() }), {
      route: paths.home(),
    });
    const setup = await required(
      () => app!.host.querySelector('[aria-label="School account setup"]'),
      "the school account setup prompt",
    );
    await click(setup.querySelector("button"));
    await until(() =>
      expect(window.location.hash).toBe(`#${paths.settings("accounts")}`),
    );
  });

  it("greets a first launch, applies the chosen language live and stays closed", async () => {
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
    const button = (root: ParentNode, label: string) =>
      [...root.querySelectorAll("button")].find(
        (candidate) => candidate.textContent === label,
      );
    const sheet = await required(
      () => document.querySelector<HTMLElement>('[role="dialog"]'),
      "the welcome sheet",
    );
    await click(button(sheet, "日本語"));
    await until(() => expect(app!.core.workspace.settings.locale).toBe("ja"));
    // The UI language does not decide the school-content language.
    expect(app.core.workspace.settings.contentLanguage ?? "auto").toBe("auto");

    // The sheet follows the choice at once: its own Later is now Japanese.
    await click(await required(() => button(sheet, "あとで"), "Later"));
    // The next window on this device opens straight to the page.
    await until(() =>
      expect(
        JSON.parse(localStorage.getItem("isct-companion-guide") ?? "{}"),
      ).toMatchObject({ welcome: true }),
    );
    // Leaving the sheet is the data notice's acknowledgement.
    await until(() =>
      expect(app!.core.workspace.settings.dataNoticeAcknowledgedAt).toEqual(
        expect.any(String),
      ),
    );

    // "Show welcome and tips again" reopens the live welcome sheet directly.
    await app.go(paths.about());
    await click(
      await required(
        () =>
          app!.host.querySelector<HTMLButtonElement>('[data-setting="guide"]'),
        "the welcome reset button",
      ),
    );
    const reopened = await required(
      () => document.querySelector<HTMLElement>('[role="dialog"]'),
      "the reopened welcome sheet",
    );
    await click(await required(() => button(reopened, "あとで"), "Later"));
    await until(() =>
      expect(document.querySelector('[role="dialog"]')).toBeNull(),
    );
  });
});

describe("workspace column sizing", () => {
  it("preserves collapsed state and content and reveals the lecture sidebar from the rail", async () => {
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
          <p data-pane={collapsed ? "collapsed" : "expanded"}>Reading pane</p>
        </ResizableWorkspace>
      );
    }
    try {
      probe = await mount(<Workbench />);
      const separator = () =>
        probe!.host.querySelector<HTMLElement>('[role="separator"]')!;
      const pane = () =>
        probe!.host.querySelector("[data-pane]")?.getAttribute("data-pane");
      await act(async () =>
        separator().dispatchEvent(
          new KeyboardEvent("keydown", { key: "Home", bubbles: true }),
        ),
      );
      expect(pane()).toBe("collapsed");
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
      expect(pane()).toBe("expanded");
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
        const toggle = () =>
          app!.host.querySelector<HTMLButtonElement>(
            '[aria-label="Toggle sidebar"]',
          )!;
        await required(toggle, "sidebar toggle");
        const list = app.host.querySelector<HTMLElement>("#lecture-sidebar")!;
        const filter = list.querySelector<HTMLInputElement>(
          'input[type="search"]',
        )!;
        await typeInto(filter, "Keep this filter");
        await click(toggle());
        await until(() =>
          expect(toggle().getAttribute("aria-expanded")).toBe("false"),
        );
        expect(list.hasAttribute("inert")).toBe(true);
        const rail = app.host.querySelector<HTMLButtonElement>(
          '.rail button[title^="Lectures"]',
        )!;
        // Hovering Lectures in the rail floats the folded list out, usable.
        await act(async () =>
          rail.dispatchEvent(new Event("pointerover", { bubbles: true })),
        );
        expect(list.hasAttribute("inert")).toBe(false);
        await act(async () =>
          rail.dispatchEvent(new Event("pointerout", { bubbles: true })),
        );
        await advance(250);
        expect(list.hasAttribute("inert")).toBe(true);
        await click(toggle());
        await until(() =>
          expect(toggle().getAttribute("aria-expanded")).toBe("true"),
        );
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

it("sends Core the hints the rail's status menu, an opened page and a page refresh ask for", async () => {
  // The rail's dot opens a native menu (the school view sits above HTML).
  // The scripted host records what the app builds; each item's action is the
  // channel the app hands Tauri, called here as Tauri would. Which row the
  // menu shows and how it reads is DESKTOP-SYNC-STATUS; the row choice is
  // unit-tested beside `progressLine`.
  type MenuCall = {
    kind: string;
    options: { text?: string; enabled?: boolean };
    handler: { onmessage: (message: unknown) => void };
  };
  let menus: MenuCall[] = [];
  let rid = 0;
  app = await mountShell(
    shellOptions({
      workspace: workspaceFixture(),
      handle: (command, args, core) => {
        if (command === "plugin:menu|new") {
          menus.push(args as MenuCall);
          return [++rid, `menu-${rid}`];
        }
        if (command === "plugin:menu|popup") return null;
        return baseHandle(command, args, core);
      },
    }),
  );
  const source = (name: string) => ({
    source: name,
    lastUpdated: null,
    running: false,
    held: null,
    failure: null as unknown,
    retry: null as unknown,
    session: null,
    maintenance: null,
  });
  let generation = 1;
  const emit = (runs: unknown[], sources = [source("lms")]) =>
    act(async () =>
      app!.core.emit("sync-snapshot", {
        generation: ++generation,
        resetting: false,
        attention: {
          primary: runs.length ? { kind: "active", run: "run" } : null,
          problem: null,
          busy: runs.length > 0,
        },
        runs,
        sources,
      }),
    );
  const hints = () =>
    app!.core.calls
      .filter((call) => call.command === "sync_hint")
      .map((call) => call.args.hint);
  /** Opens the menu and returns its items. */
  const openMenu = async () => {
    menus = [];
    await click(app!.host.querySelector(".schoolStatusDot"));
    await until(() =>
      expect(menus.some((menu) => menu.kind === "Menu")).toBe(true),
    );
    return menus.filter((menu) => menu.kind === "MenuItem");
  };
  const choose = async (
    items: MenuCall[],
    what: string,
    matches: (text: string) => boolean,
  ) => {
    const item = items.find((entry) => matches(entry.options.text ?? ""));
    expect(item?.options.enabled, what).toBe(true);
    await act(async () =>
      item!.handler.onmessage(`menu-${item!.options.text}`),
    );
  };
  const retry = (text: string) => text.startsWith("Retry · ");
  await until(() =>
    expect(app!.core.listenerCount("sync-snapshot")).toBeGreaterThan(0),
  );

  // A failed run's Retry asks for its own scope; Refresh now for everything.
  await emit([
    {
      run: "grades",
      dataset: "kyomu.page",
      label: { key: "sync.dataset.lms.course", args: {} },
      priority: "visible",
      done: 0,
      total: null,
      current: null,
      scope: "grades",
      held: null,
      failure: {
        kind: "timeout",
        reason: { key: "syncStatus.failure.timedOut", args: {} },
        keysFailed: 1,
        retryAt: null,
        detail: null,
      },
      retry: { scopes: [{ kind: "site", site: "kyomu" }] },
    },
  ]);
  await choose(await openMenu(), "the failed run's Retry", retry);
  await choose(
    await openMenu(),
    "Refresh now",
    (text) => text === "Refresh now",
  );
  await until(() =>
    expect(hints()).toEqual(
      expect.arrayContaining([
        { kind: "refresh", scope: { kind: "site", site: "kyomu" } },
        { kind: "refresh", scope: { kind: "everything" } },
      ]),
    ),
  );

  // A source's persisted failure retries every scope it names.
  await emit(
    [],
    [
      {
        ...source("catalog"),
        failure: {
          kind: "http-503",
          reason: {
            key: "syncStatus.failure.serverFailed",
            args: { status: "503" },
          },
          keysFailed: 2,
          retryAt: null,
          detail: null,
        },
        retry: {
          scopes: [
            { kind: "catalog", year: 2024 },
            { kind: "catalog", year: 2026 },
          ],
        },
      },
    ],
  );
  await choose(await openMenu(), "the catalog failure's Retry", retry);
  await until(() =>
    expect(hints()).toEqual(
      expect.arrayContaining([
        { kind: "refresh", scope: { kind: "catalog", year: 2024 } },
        { kind: "refresh", scope: { kind: "catalog", year: 2026 } },
      ]),
    ),
  );

  // Opening a page and its own refresh tell Core what is in view.
  await app.go(paths.timetable());
  await until(() =>
    expect(hints()).toContainEqual({
      kind: "open",
      view: { kind: "timetable" },
    }),
  );
  await click(
    await required(
      () =>
        [...app!.host.querySelectorAll<HTMLButtonElement>("button")].find(
          (button) => button.textContent?.trim() === "Refresh timetable",
        ),
      "the timetable refresh action",
    ),
  );
  await until(() =>
    expect(hints()).toContainEqual({
      kind: "refresh",
      scope: { kind: "timetable" },
    }),
  );
});
