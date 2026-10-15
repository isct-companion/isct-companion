// @vitest-environment jsdom
import { createMemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";
import {
  parseRoute,
  paths,
  routeRedirect,
  workspaceRoutes,
  type DrawerTab,
} from "./routes";
import type { SectionContext } from "../pages/settings/registry";

const desktop: SectionContext = {
  platform: "native-desktop",
  developer: true,
  extensionsAvailable: true,
};
describe("shared workspace routes", () => {
  it("round-trips page, section, course and assignment paths including encoded IDs", () => {
    expect(parseRoute(paths.home()).homeView).toBe("day");
    expect(parseRoute(paths.home("week")).homeView).toBe("week");
    expect(parseRoute(paths.web()).page).toBe("web");
    expect(parseRoute(paths.lectures()).courseId).toBeUndefined();
    expect(parseRoute(paths.settings("appearance")).section).toBe("appearance");
    const courseId = "中文 /?#%";
    const assignmentId = "task /?#%";
    expect(parseRoute(paths.lectures(courseId)).courseId).toBe(courseId);
    expect(parseRoute(paths.lectures(courseId, assignmentId))).toMatchObject({
      page: "lectures",
      courseId,
      assignmentId,
    });
  });
  it.each<DrawerTab>(["overview", "materials", "assignments", "syllabus"])(
    "round-trips drawer tab %s on every page",
    (tab) => {
      for (const page of [
        paths.home("week"),
        paths.lectures("c", "a"),
        paths.web(),
        paths.settings("study"),
      ]) {
        const target = paths.drawer(page + "?other=value", "课程 /?#", tab);
        expect(parseRoute(target).drawer).toEqual({
          courseId: "课程 /?#",
          tab,
        });
        expect(paths.drawer(target, null)).toBe(page + "?other=value");
      }
    },
  );
  it("defaults missing or invalid tabs to overview", () => {
    for (const query of ["?course=c", "?course=c&tab=bad", "?course=c&tab="]) {
      expect(parseRoute(paths.web() + query).drawer).toEqual({
        courseId: "c",
        tab: "overview",
      });
    }
    expect(parseRoute(paths.home()).drawer).toEqual({
      courseId: null,
      tab: "overview",
    });
  });
  it("redirects unknown and invalid paths, missing/deleted courses and unsupported sections", () => {
    expect(routeRedirect("/")).toBe(paths.home());
    expect(routeRedirect("/unknown?course=c&tab=syllabus")).toBe(
      paths.home() + "?course=c&tab=syllabus",
    );
    expect(routeRedirect(paths.settings())).toBe(paths.settings("accounts"));
    expect(routeRedirect("/settings/bad")).toBe(paths.settings("accounts"));
    expect(routeRedirect(paths.lectures("missing"), { courses: [] })).toBe(
      paths.lectures(),
    );
    expect(
      routeRedirect(paths.lectures("deleted", "a"), {
        courses: [{ id: "deleted", deletedAt: "now" }],
      }),
    ).toBe(paths.lectures());
    expect(
      routeRedirect(paths.lectures("c"), { courses: [{ id: "c" }] }),
    ).toBeUndefined();
    expect(
      routeRedirect(paths.settings("calendar"), { context: desktop }),
    ).toBeUndefined();
    for (const section of [
      "calendar",
      "files",
      "agent",
      "developer",
      "diagnostics",
    ] as const) {
      expect(
        routeRedirect(paths.settings(section), {
          context: { ...desktop, platform: "native-mobile" },
        }),
      ).toBe(paths.settings("accounts"));
    }
    for (const section of ["diagnostics", "extensions"] as const) {
      expect(
        routeRedirect(paths.settings(section), {
          context: { ...desktop, developer: false },
        }),
      ).toBe(paths.settings("accounts"));
    }
    expect(
      routeRedirect(paths.settings("extensions"), {
        context: { ...desktop, extensionsAvailable: false },
      }),
    ).toBe(paths.settings("accounts"));
  });
  it.each(["/", "/unknown", "/settings"])(
    "the data router replaces %s without an Outlet",
    async (initial) => {
      const router = createMemoryRouter(
        workspaceRoutes(() => null),
        { initialEntries: [initial + "?course=c&tab=syllabus"] },
      );
      await new Promise<void>((resolve) => {
        const off = router.subscribe((state) => {
          if (state.initialized && state.navigation.state === "idle") {
            off();
            resolve();
          }
        });
      });
      expect(router.state.location.pathname).toBe(
        initial === "/settings" ? paths.settings("accounts") : paths.home(),
      );
      expect(router.state.location.search).toBe("?course=c&tab=syllabus");
      expect(router.state.historyAction).toBe("REPLACE");
      router.dispose();
    },
  );
});
