// @vitest-environment jsdom
import { act, StrictMode, useEffect } from "react";
import { createRoot, type Root } from "react-dom/client";
import { createHashRouter } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { notifications } from "@mantine/notifications";
import { seedWorkspace, type Workspace } from "../lib/model";
import type { WorkspaceSnapshot } from "../store/workspace";
import type { SchoolIngestOutcome } from "../platform/desktop-bridge";

const fixture = vi.hoisted(() => ({
  state: {} as WorkspaceSnapshot,
  listeners: new Set<() => void>(),
  ingest: new Set<(event: SchoolIngestOutcome) => void>(),
  hide: vi.fn(),
  capabilities: vi.fn(),
  reload: vi.fn(),
  guide: vi.fn(),
  policy: vi.fn(),
  openWeb: vi.fn(),
  urlListeners: new Set<(event: { site: string; url: string }) => void>(),
  lectureEffects: vi.fn(),
  lectureCleanup: vi.fn(),
  pull: vi.fn(),
  refreshEvent: vi.fn().mockResolvedValue(undefined),
  nativeScheduler: false,
  workspaceChanged: vi.fn(async (_callback: () => void) => () => {}),
  platform: "native-desktop",
}));
vi.mock("../store/workspace", () => ({
  workspaceStore: {
    getState: () => fixture.state,
    getInitialState: () => fixture.state,
    subscribe: (listener: () => void) => {
      fixture.listeners.add(listener);
      return () => fixture.listeners.delete(listener);
    },
    initialize: async () => {},
    reload: fixture.reload,
    flush: async () => {},
  },
  workspaceActions: {
    refreshSchool: async () => {},
    pullSchool: fixture.pull,
    updateSettings: async (operation: (workspace: Workspace) => Workspace) => {
      fixture.state = {
        ...fixture.state,
        workspace: operation(structuredClone(fixture.state.workspace!)),
      };
      fixture.listeners.forEach((listener) => listener());
    },
  },
}));
vi.mock("../platform/core-query", () => {
  const value = (query: { kind: string; term?: string; year?: number }) => {
    const data = fixture.state.workspace!;
    const term = query.term || "2026-3Q";
    const courses = data.courses.filter(
      (course) => course.termId === term && !course.deletedAt,
    );
    const overview = {
      term,
      currentTerm: "2026-3Q",
      selectedTerm: "2026-3Q",
      todayTerm: "2026-3Q",
      coursesTerm: term,
      range: null,
      courses,
      termCourseIds: courses.map((course) => course.id),
      declaredCredits: null,
      termCredits: courses.reduce((sum, course) => sum + course.credits, 0),
      colors: Object.fromEntries(
        courses.map((course) => [course.id, course.color]),
      ),
      enrolledCourseIds: courses.map((course) => course.id),
      onTimetableCourseIds: courses.map((course) => course.id),
      allTerms: ["2026-3Q", "2026-2Q"],
      defaultFinderKeys: [],
    };
    switch (query.kind) {
      case "workspaceOverview":
        return overview;
      case "scheduleView":
        return { ...overview, agendas: [] };
      case "planningWeek":
        return {
          courses,
          days: Array.from({ length: 7 }, (_, day) => ({
            day: day + 1,
            blocks: [],
          })),
          conflictCount: 0,
        };
      case "academicYears":
        return { currentYear: 2026, years: [2026] };
      case "academicYear":
        return {
          year: query.year ?? 2026,
          quarters: [],
          days: [],
          importantDates: [],
          published: false,
        };
      case "courseTimeline":
        return { lastStarted: null, next: null, classAt: null };
      case "subscriptionViews":
        return { byKind: {} };
      default:
        return null;
    }
  };
  return {
    useScheduleQuery: (query: {
      kind: string;
      term?: string;
      year?: number;
    }) => ({ value: value(query), pending: false }),
    useScheduleQueries: (
      queries: { kind: string; term?: string; year?: number }[],
    ) => ({ value: queries.map(value), pending: false }),
    scheduleQuery: async (query: {
      kind: string;
      term?: string;
      year?: number;
    }) => value(query),
  };
});
vi.mock("../platform/finder-query", () => {
  const value = (request: { keys?: string[]; input?: string }) => ({
    context: {
      currentQuarter: 3,
      recommended: {},
      departments: [],
      busy: [],
    },
    courses: [],
    factors: [],
    duplicates: 0,
    keys: request.keys ?? [],
    plan: { unsupported: "empty" },
    rows: [],
    suggestions: [],
    live: request.input ?? "",
    clashes: [],
    ineligibleIds: [],
    replacements: {},
    credits: {},
    onTimetable: [],
    activeSlots: [],
    pasteHandled: false,
    pastedIds: [],
    missingCodes: [],
    detailIds: [],
  });
  return {
    useFinderEditQueue: ({
      setKeys,
    }: {
      setKeys: (keys: string[]) => void;
    }) => ({
      run: async () => undefined,
      replace: setKeys,
    }),
    useFinderQuery: (request: { keys?: string[]; input?: string }) => ({
      value: value(request),
      pending: false,
    }),
    finderQuery: async (request: { keys?: string[]; input?: string }) =>
      value(request),
  };
});
vi.mock("./native", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  desktopBridge: {
    getRuntimeCapabilities: fixture.capabilities,
    schoolRefreshEvent: fixture.refreshEvent,
    onWorkspaceChanged: fixture.workspaceChanged,
    get runScheduledTask() {
      return fixture.nativeScheduler ? async () => "Updated" : undefined;
    },
    onSchoolIngest: (callback: (event: SchoolIngestOutcome) => void) => {
      fixture.ingest.add(callback);
      return () => fixture.ingest.delete(callback);
    },
    setKyomuGuide: fixture.guide,
    configureSchoolWebPolicy: fixture.policy,
    setSchoolWebBounds: async () => true,
    resolveSchoolUrl: async (url: string) => ({ site: "science-lms", url }),
    openSchoolSite: fixture.openWeb,
    setSchoolWebZoom: async () => true,
    onSchoolWebUrl: (
      callback: (event: { site: string; url: string }) => void,
    ) => {
      fixture.urlListeners.add(callback);
      return () => fixture.urlListeners.delete(callback);
    },
    schoolCredentialStatus: async () => ({
      science: { saved: false },
      tokyoTech: { saved: false },
    }),
    bootstrapReminders: async () => false,
    pullSchool: true,
  },
  hideNativeSchoolWeb: fixture.hide,
  schoolWebExit: { settled: async () => {}, pending: false },
  claimSchoolView: () => {},
  saveExportFile: async () => {},
}));
vi.mock("../platform/capabilities", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  detectPlatform: () => fixture.platform,
}));
vi.mock("../pages/courses/CoursesPage", () => ({
  Courses: () => (
    <div className="coursesPage">
      <input aria-label="Home retained input" />
    </div>
  ),
  coursesTerm: (data: Workspace) => data.selectedTerm,
}));
vi.mock("../pages/today/TodayPage", () => ({
  Today: () => <div className="todayPage">Today</div>,
}));
vi.mock("../features/lms/LmsWorkspace", () => ({
  LmsWorkspace: ({
    entry,
  }: {
    entry: { courseId: string; assignmentId?: string };
  }) => {
    useEffect(() => {
      fixture.lectureEffects();
      return () => fixture.lectureCleanup();
    }, []);
    return (
      <div
        className="testReader"
        data-course={entry.courseId}
        data-assignment={entry.assignmentId}
      >
        <input aria-label="Lecture retained input" />
      </div>
    );
  },
}));
import App, { WorkspaceApp } from "./App";
import { paths, workspaceRoutes } from "./routes";
import { STACKED_QUERY } from "./form-factor";
import { notify } from "./notify";
import i18n from "../i18n";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
globalThis.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
};
window.matchMedia = ((media: string) => ({
  media,
  matches: false,
  addEventListener() {},
  removeEventListener() {},
})) as never;
let host: HTMLDivElement;
let root: Root;
let router: ReturnType<typeof createHashRouter>;
const waitFor = async (check: () => void) => {
  await act(async () => {
    await vi.waitFor(check);
  });
};
const go = async (target: string) => {
  await act(async () => {
    await router.navigate(target);
  });
};
const currentPage = () => host.querySelector(".workspaceMain, .mobileMain");

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  sessionStorage.clear();
  fixture.platform = "native-desktop";
  fixture.nativeScheduler = false;
  const data = seedWorkspace();
  data.currentTerm = data.selectedTerm = "2026-3Q";
  data.terms = ["2026-3Q", "2026-2Q"];
  data.scheduledTasks = [];
  data.courses = [
    {
      id: "course-1",
      kind: "course",
      rev: 0,
      updatedAt: "now",
      updatedBy: data.deviceId,
      termId: "2026-3Q",
      title: "Route course",
      code: "TEST",
      instructor: "Teacher",
      credits: 2,
      creditSplit: { lecture: 2, exercise: 0, lab: 0 },
      instructionLanguage: "en",
      color: "#3478f6",
      meetings: [],
      source: { provider: "manual" },
    } as never,
  ];
  fixture.state = {
    workspace: data,
    status: "ready",
    revision: "fixture:ready",
    contentRevision: "fixture:ready",
  };
  fixture.hide.mockResolvedValue(undefined);
  fixture.capabilities.mockResolvedValue({ schoolWeb: true });
  fixture.reload.mockResolvedValue(undefined);
  fixture.guide.mockResolvedValue(true);
  fixture.policy.mockResolvedValue(true);
  fixture.openWeb.mockResolvedValue({
    url: "https://portal.isct.ac.jp/resident",
  });
  fixture.pull.mockResolvedValue([{ state: "ok", message: "Updated" }]);
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  router?.dispose();
  host.remove();
  fixture.listeners.clear();
  fixture.ingest.clear();
  fixture.urlListeners.clear();
  act(() => notifications.clean());
  await i18n.changeLanguage("en");
});
async function mount(target = paths.home(), strict = false) {
  window.location.hash = target;
  router = createHashRouter(workspaceRoutes(WorkspaceApp));
  await act(async () => {
    root.render(
      strict ? (
        <StrictMode>
          <App router={router} />
        </StrictMode>
      ) : (
        <App router={router} />
      ),
    );
    await vi.dynamicImportSettled();
  });
  await waitFor(() => expect(currentPage()).not.toBeNull());
  expect(host.querySelector(".pageError")).toBeNull();
}

describe("hash-driven desktop navigation", () => {
  it("loads reference data that finished refreshing before the native listener mounted", async () => {
    fixture.nativeScheduler = true;
    fixture.reload.mockImplementationOnce(async () => {
      fixture.state = {
        ...fixture.state,
        workspace: {
          ...fixture.state.workspace!,
          academicCalendar: [
            {
              id: "calendar:2026:quarter-start:3Q",
              academicYear: 2026,
              quarter: "3Q",
              type: "quarter-start",
              date: "2026-09-28",
              label: { en: "3Q starts" },
              rev: 1,
              updatedAt: "2026-10-13T03:00:00Z",
              updatedBy: "subscription",
            },
          ],
        },
      };
      fixture.listeners.forEach((listener) => listener());
    });
    await mount();
    expect(fixture.workspaceChanged).toHaveBeenCalledOnce();
    expect(fixture.state.workspace!.academicCalendar[0].quarter).toBe("3Q");
  });
  it("waits for native listener registration before recovering the startup snapshot", async () => {
    fixture.nativeScheduler = true;
    let finishRegistration!: (dispose: () => void) => void;
    fixture.workspaceChanged.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finishRegistration = resolve;
        }),
    );
    await mount();
    expect(fixture.reload).not.toHaveBeenCalled();
    await act(async () => {
      finishRegistration(() => {});
    });
    await waitFor(() => expect(fixture.reload).toHaveBeenCalledOnce());
  });
  it.each([
    [paths.home("week"), "homeMain"],
    [paths.settings("appearance"), "settingsMain"],
    [paths.lectures("course-1", "assignment-1"), "lecturesMain"],
    [paths.web(), "webMain"],
  ])(
    "opens the initial hash %s and restores it after remount",
    async (target, className) => {
      await mount(target);
      expect(currentPage()?.classList.contains(className)).toBe(true);
      if (className === "homeMain")
        expect(host.querySelector(".homePage.week")).not.toBeNull();
      if (className === "settingsMain")
        expect(
          host.querySelector(".settingsNav button.active")?.textContent,
        ).toContain("Appearance");
      if (className === "lecturesMain")
        expect(
          host.querySelector(".testReader")?.getAttribute("data-assignment"),
        ).toBe("assignment-1");
      await act(async () => root.unmount());
      router.dispose();
      root = createRoot(host);
      await mount(target);
      expect(currentPage()?.classList.contains(className)).toBe(true);
      expect(window.location.hash).toBe("#" + target);
    },
  );
  it("returns each page to where it was left when switching pages", async () => {
    await mount(paths.lectures("course-1"));
    const rail = (label: string) =>
      [...host.querySelectorAll<HTMLButtonElement>(".rail button")].find(
        (button) => button.textContent?.includes(label),
      )!;
    await act(async () => rail("Settings").click());
    await waitFor(() =>
      expect(window.location.hash).toBe("#" + paths.settings("accounts")),
    );
    await go(paths.settings("appearance"));
    await act(async () => rail("Lectures").click());
    await waitFor(() =>
      expect(window.location.hash).toBe("#" + paths.lectures("course-1")),
    );
    await act(async () => rail("Settings").click());
    await waitFor(() =>
      expect(window.location.hash).toBe("#" + paths.settings("appearance")),
    );
  });
  it("replaces invalid, deleted-course and platform-unsupported routes", async () => {
    await mount(paths.home());
    for (const target of ["/settings/bad", paths.settings("diagnostics")]) {
      await go(target);
      expect(window.location.hash).toBe("#" + paths.settings("accounts"));
      expect(router.state.historyAction).toBe("REPLACE");
    }
    fixture.platform = "browser-preview";
    await go(paths.settings("files"));
    expect(window.location.hash).toBe("#" + paths.settings("accounts"));
    await go(paths.lectures("missing"));
    expect(window.location.hash).toBe("#" + paths.lectures());
    fixture.state.workspace!.courses[0].deletedAt = "now";
    await go(paths.lectures("course-1"));
    expect(window.location.hash).toBe("#" + paths.lectures());
    expect(host.querySelector(".testReader")).toBeNull();
  });
  it.each([true, false])(
    "waits for capabilities before validating the extensions deep link (supported: %s)",
    async (supported) => {
      localStorage.setItem("isct-developer-mode", "1");
      let resolveCapabilities!: (snapshot: { schoolWeb: boolean }) => void;
      fixture.capabilities.mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveCapabilities = resolve;
          }),
      );
      const target = paths.settings("extensions");
      await mount(target);
      expect(window.location.hash).toBe("#" + target);
      await act(async () => resolveCapabilities({ schoolWeb: supported }));
      await waitFor(() =>
        expect(window.location.hash).toBe(
          "#" + (supported ? target : paths.settings("accounts")),
        ),
      );
      if (supported)
        expect(host.querySelector(".settingsPane")?.textContent).toContain(
          "Site styles & scripts",
        );
      else expect(router.state.historyAction).toBe("REPLACE");
    },
  );
  it.each([paths.web(), paths.lectures("course-1")])(
    "keeps %s and its URL when hiding the native view fails",
    async (start) => {
      await mount(start);
      fixture.hide.mockRejectedValueOnce(new Error("Native hide failed"));
      await go(paths.home());
      await waitFor(() =>
        expect(document.body.textContent).toContain("Native hide failed"),
      );
      expect(window.location.hash).toBe("#" + start);
      expect(
        currentPage()?.classList.contains(
          start === paths.web() ? "webMain" : "lecturesMain",
        ),
      ).toBe(true);
      expect(fixture.guide).not.toHaveBeenCalled();
    },
  );
  it("blocks browser Back, restores the hash on failure, then allows Back on success", async () => {
    await mount(paths.home());
    await go(paths.web());
    fixture.hide.mockClear().mockRejectedValueOnce("Back hide failed");
    await act(async () => {
      window.history.back();
      await new Promise((resolve) => setTimeout(resolve, 40));
    });
    await waitFor(() =>
      expect(document.body.textContent).toContain("Back hide failed"),
    );
    expect(window.location.hash).toBe("#" + paths.web());
    expect(currentPage()?.classList.contains("webMain")).toBe(true);
    expect(fixture.hide).toHaveBeenCalledTimes(1);
    await act(async () => {
      window.history.back();
      await new Promise((resolve) => setTimeout(resolve, 40));
    });
    await waitFor(() => expect(window.location.hash).toBe("#" + paths.home()));
    expect(fixture.guide).toHaveBeenCalledWith([]);
  });
  it("waits for native hiding and reloads ingested data in the background when leaving Web", async () => {
    await mount(paths.web());
    let finishHide!: () => void;
    fixture.hide.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finishHide = resolve;
        }),
    );
    fixture.reload.mockImplementationOnce(() => new Promise(() => {}));
    act(() =>
      fixture.ingest.forEach((callback) =>
        callback({ site: "kyomu", state: "ok" } as never),
      ),
    );
    await go(paths.home());
    expect(window.location.hash).toBe("#" + paths.web());
    expect(fixture.reload).not.toHaveBeenCalled();
    await act(async () => finishHide());
    await waitFor(() => expect(window.location.hash).toBe("#" + paths.home()));
    expect(fixture.reload).toHaveBeenCalledTimes(1);
    expect(fixture.guide).toHaveBeenCalledWith([]);
  });
  it("retains input, scroll and the chosen lecture quarter while cleaning and restarting Activity effects", async () => {
    await mount(paths.lectures("course-1"), true);
    const input = host.querySelector<HTMLInputElement>(
      '[aria-label="Lecture retained input"]',
    )!;
    input.value = "unsent text";
    const list = host.querySelector<HTMLElement>(".lectureList")!;
    list.scrollTop = 128;
    const term = host.querySelector<HTMLSelectElement>(
      ".lectureTerm select, select.lectureTerm",
    )!;
    await act(async () => {
      term.value = "2026-2Q";
      term.dispatchEvent(new Event("change", { bubbles: true }));
    });
    const effectCount = fixture.lectureEffects.mock.calls.length;
    const cleanupCount = fixture.lectureCleanup.mock.calls.length;
    await go(paths.home());
    expect(fixture.lectureCleanup.mock.calls.length).toBeGreaterThan(
      cleanupCount,
    );
    await go(paths.lectures("course-1"));
    expect(host.querySelector('[aria-label="Lecture retained input"]')).toBe(
      input,
    );
    expect(input.value).toBe("unsent text");
    expect(list.scrollTop).toBe(128);
    expect(term.value).toBe("2026-2Q");
    expect(fixture.lectureEffects.mock.calls.length).toBeGreaterThan(
      effectCount,
    );
  });
  it("reports lecture opens to Rust without starting a frontend pull", async () => {
    fixture.state.workspace!.courses[0].source.externalId = "12";
    fixture.state.schoolRecords = {
      lmsCourses: [{ lmsId: 12 }],
      registrations: [],
    } as never;
    await mount(paths.lectures("course-1"));
    expect(fixture.refreshEvent).toHaveBeenCalledWith({
      kind: "lecture-opened",
      courseId: 12,
    });
    expect(fixture.pull).not.toHaveBeenCalled();
    await go(paths.home());
    await go(paths.lectures("course-1"));
    expect(fixture.refreshEvent).toHaveBeenLastCalledWith({
      kind: "lecture-opened",
      courseId: 12,
    });
    expect(fixture.pull).not.toHaveBeenCalled();
  });
  it("retains settings subpage DOM, disclosure state and scroll across page switches", async () => {
    await mount(paths.settings("appearance"));
    const disclosure = host.querySelector<HTMLDetailsElement>("details")!;
    expect(disclosure).not.toBeNull();
    disclosure.open = true;
    const pane = host.querySelector<HTMLElement>(".settingsPane")!;
    pane.scrollTop = 164;
    const language = host.querySelector<HTMLInputElement>('input[value="zh"]')!;
    await go(paths.home());
    await go(paths.settings("appearance"));
    expect(host.querySelector('input[value="zh"]')).toBe(language);
    expect(host.querySelector("details")).toBe(disclosure);
    expect(disclosure.open).toBe(true);
    expect(pane.scrollTop).toBe(164);
  });
  it("keeps the resident Web page when Activity becomes visible again", async () => {
    await mount(paths.web());
    await waitFor(() => expect(fixture.openWeb).toHaveBeenCalled());
    const address = host.querySelector<HTMLInputElement>(
      '[aria-label="Current URL"]',
    )!;
    expect(address).not.toBeNull();
    await act(async () => {
      address.focus();
      address.value = "https://portal.isct.ac.jp/typed";
      address.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await go(paths.home());
    await go(paths.web());
    expect(host.querySelector('[aria-label="Current URL"]')).toBe(address);
    expect(fixture.openWeb.mock.calls.at(-1)?.[4]).toBeUndefined();
  });
  it("opens an explicitly requested URL again after leaving Web, while ordinary resume keeps the resident page", async () => {
    const url = "https://lms.s.isct.ac.jp/course/view.php?id=12";
    const course = fixture.state.workspace!.courses[0];
    course.source.provider = "lms";
    course.url = url;
    const drawer = paths.drawer(paths.home(), course.id);
    await mount(drawer);
    for (let attempt = 0; attempt < 2; attempt++) {
      fixture.openWeb.mockClear();
      await act(async () => {
        document
          .querySelector<HTMLButtonElement>(".officialLink button")!
          .click();
      });
      await waitFor(() =>
        expect(fixture.openWeb.mock.calls.at(-1)?.[4]).toBe(url),
      );
      expect(window.location.hash).toBe("#" + paths.web());
      await go(paths.home());
      fixture.openWeb.mockClear();
      await go(paths.web());
      await waitFor(() => expect(fixture.openWeb).toHaveBeenCalled());
      expect(fixture.openWeb.mock.calls.at(-1)?.[4]).toBeUndefined();
      await go(drawer);
    }
  });
  it("routes ⌘1–4 and Escape, replacing section and all four drawer tab changes", async () => {
    await mount(paths.settings("appearance"));
    const study = [
      ...host.querySelectorAll<HTMLButtonElement>(".settingsNav button"),
    ].find((button) => button.textContent?.trim() === "Study")!;
    await act(async () => study.click());
    expect(window.location.hash).toBe("#" + paths.settings("study"));
    expect(router.state.historyAction).toBe("REPLACE");
    await go(paths.drawer(paths.home(), "course-1", "overview"));
    for (const label of ["Materials", "Assignments", "Syllabus", "Overview"]) {
      const tab = [
        ...document.querySelectorAll<HTMLButtonElement>('[role="tab"]'),
      ].find((button) => button.textContent?.includes(label))!;
      await act(async () => tab.click());
      expect(window.location.hash).toContain("tab=" + label.toLowerCase());
      expect(router.state.historyAction).toBe("REPLACE");
    }
    await act(async () =>
      document.body.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      ),
    );
    expect(window.location.hash).toBe("#" + paths.home());
    for (const [index, target] of [
      paths.home(),
      paths.lectures(),
      paths.web(),
      // ⌘4 returns to the section left above.
      paths.settings("study"),
    ].entries()) {
      await act(async () =>
        window.dispatchEvent(
          new KeyboardEvent("keydown", {
            key: String(index + 1),
            metaKey: true,
            cancelable: true,
          }),
        ),
      );
      await waitFor(() => expect(window.location.hash).toBe("#" + target));
    }
  });
  it("loads Chinese UI and preserves explicit school-content language", async () => {
    fixture.state.workspace!.settings.locale = "zh";
    fixture.state.workspace!.settings.contentLanguage = "en";
    await mount(paths.settings("appearance"));
    expect(document.documentElement.lang).toBe("zh");
    expect(host.querySelector(".rail button")?.textContent).toContain("首页");
    expect(document.body.textContent).toContain("中文");
    expect(fixture.state.workspace!.settings.contentLanguage).toBe("en");
  });
  it("shows Mantine error notifications for store errors and keeps the save retry action", async () => {
    fixture.state.error = "Disk is full";
    await mount();
    expect(document.querySelector(".workspaceNotice")?.textContent).toContain(
      "Disk is full",
    );
    const retry = [
      ...document.querySelectorAll<HTMLButtonElement>("button"),
    ].find((button) => button.textContent === "Retry saving")!;
    fixture.reload.mockClear();
    await act(async () => retry.click());
    expect(fixture.reload).toHaveBeenCalledTimes(1);
    await act(async () => notify.error("Plain Tauri rejection"));
    expect(document.querySelector(".workspaceNotice")?.textContent).toContain(
      "Plain Tauri rejection",
    );
  });
});

describe("mobile shell", () => {
  const desktopMedia = window.matchMedia;
  beforeEach(() => {
    window.matchMedia = ((media: string) => ({
      media,
      matches: media === STACKED_QUERY,
      addEventListener() {},
      removeEventListener() {},
    })) as never;
  });
  afterEach(() => {
    window.matchMedia = desktopMedia;
  });

  it("drops the fixed status bar and shows four bottom tabs", async () => {
    await mount(paths.home());
    expect(host.querySelector(".mobileShell")).not.toBeNull();
    expect(host.querySelector(".schoolStatusBar")).toBeNull();
    expect(
      [...host.querySelectorAll(".mobileTabs button")].map(
        (button) => button.textContent,
      ),
    ).toEqual(["Home", "Lectures", "Web", "Settings"]);
    expect(host.querySelector(".mobileStatusDot")).not.toBeNull();
  });

  it("keeps bare /settings as the section list and pushes a section", async () => {
    await mount(paths.settings());
    expect(window.location.hash).toBe("#" + paths.settings());
    expect(host.querySelector(".settings.list")).not.toBeNull();
    const appearance = [
      ...host.querySelectorAll<HTMLButtonElement>(".settingsNav button"),
    ].find((button) => button.textContent?.trim() === "Appearance")!;
    await act(async () => appearance.click());
    expect(window.location.hash).toBe("#" + paths.settings("appearance"));
    expect(router.state.historyAction).toBe("PUSH");
    expect(host.querySelector(".settings.detail .settingsBack")).not.toBeNull();
  });

  it("lists school services on Web and browses one through the route", async () => {
    await mount(paths.web());
    const rows = [...host.querySelectorAll<HTMLButtonElement>(".mobileWebRow")];
    expect(rows.length).toBeGreaterThan(3);
    expect(host.querySelector(".servicePicker")).toBeNull();
    await waitFor(() => expect(fixture.capabilities).toHaveBeenCalled());
    const lms = rows.find((row) => row.dataset.site === "science-lms")!;
    await act(async () => lms.click());
    await waitFor(() =>
      expect(window.location.hash).toBe("#" + paths.web("science-lms")),
    );
    await waitFor(() =>
      expect(host.querySelector(".mobileBrowserBar")).not.toBeNull(),
    );
    expect(host.querySelector(".mobileBrowserTitle")?.textContent).toContain(
      "LMS",
    );
    // Browsing marks the Web tab; tapping it again opens the services sheet.
    const webTab = [
      ...host.querySelectorAll<HTMLButtonElement>(".mobileTabs button"),
    ].find((button) => button.querySelector(".mobileTabMenuMark"))!;
    expect(webTab).toBeDefined();
    await act(async () => webTab.click());
    await waitFor(() =>
      expect(document.querySelector(".mobileSiteSheet")).not.toBeNull(),
    );
    expect(window.location.hash).toBe("#" + paths.web("science-lms"));
  });
  it("goes up one level, reusing the parent entry when it is just below", async () => {
    await mount(paths.settings());
    const appearance = [
      ...host.querySelectorAll<HTMLButtonElement>(".settingsNav button"),
    ].find((button) => button.textContent?.trim() === "Appearance")!;
    await act(async () => appearance.click());
    await act(async () =>
      host.querySelector<HTMLButtonElement>(".settingsBack")!.click(),
    );
    await waitFor(() =>
      expect(window.location.hash).toBe("#" + paths.settings()),
    );
    expect(router.state.historyAction).toBe("POP");
  });

  it("opened deep, back replaces with the parent instead of leaving", async () => {
    await mount(paths.settings("appearance"));
    await act(async () =>
      host.querySelector<HTMLButtonElement>(".settingsBack")!.click(),
    );
    await waitFor(() =>
      expect(window.location.hash).toBe("#" + paths.settings()),
    );
    expect(router.state.historyAction).toBe("REPLACE");
  });

  it("tapping the current tab again returns to its root", async () => {
    await mount(paths.settings("appearance"));
    const settingsTab = [
      ...host.querySelectorAll<HTMLButtonElement>(".mobileTabs button"),
    ].find((button) => button.textContent === "Settings")!;
    await act(async () => settingsTab.click());
    await waitFor(() =>
      expect(window.location.hash).toBe("#" + paths.settings()),
    );
    await go(paths.home("week"));
    const homeTab = [
      ...host.querySelectorAll<HTMLButtonElement>(".mobileTabs button"),
    ].find((button) => button.textContent === "Home")!;
    await act(async () => homeTab.click());
    await waitFor(() => expect(window.location.hash).toBe("#" + paths.home()));
  });

  it("opens a lecture as a dialog over the list", async () => {
    await mount(paths.lectures("course-1"));
    const content = host.querySelector(".lectureContent");
    expect(content?.getAttribute("role")).toBe("dialog");
    expect(host.querySelector(".testReader")).not.toBeNull();
  });
});
