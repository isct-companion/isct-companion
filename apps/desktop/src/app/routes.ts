import { createElement, Fragment, type ComponentType } from "react";
import { matchRoutes, replace, type RouteObject } from "react-router";
import type { CourseTab } from "../pages/courses/CourseDetails";
import type { Course } from "../lib/model";
import type { Page, Section } from "./shared";
import { isStacked } from "./form-factor";
import {
  sectionParent,
  sectionSupported,
  visibleSections,
  type SectionContext,
} from "../pages/settings/registry";

export type DrawerTab = CourseTab;
const drawerTabs = new Set<string>([
  "overview",
  "materials",
  "assignments",
  "syllabus",
]);
export const routePatterns = {
  home: "/home",
  week: "/home/week",
  lectures: "/lectures",
  course: "/lectures/:courseId",
  assignment: "/lectures/:courseId/assignments/:aid",
  web: "/web",
  webSite: "/web/:site",
  settings: "/settings",
  section: "/settings/:section",
} as const;
const encode = encodeURIComponent;
export const paths = {
  home: (view: "day" | "week" = "day") =>
    view === "week" ? routePatterns.week : routePatterns.home,
  lectures: (courseId?: string, assignmentId?: string) =>
    courseId
      ? `${routePatterns.lectures}/${encode(courseId)}${assignmentId ? `/assignments/${encode(assignmentId)}` : ""}`
      : routePatterns.lectures,
  web: (site?: string) =>
    site ? `${routePatterns.web}/${encode(site)}` : routePatterns.web,
  settings: (section?: Section) =>
    section ? `${routePatterns.settings}/${section}` : routePatterns.settings,
  page: (page: Page) => paths[page](),
  drawer: (
    location: string,
    courseId?: string | null,
    tab: DrawerTab = "overview",
  ) => {
    const [pathname, search = ""] = location.split("?");
    const params = new URLSearchParams(search);
    params.delete("course");
    params.delete("tab");
    if (courseId) {
      params.set("course", courseId);
      params.set("tab", tab);
    }
    return pathname + (params.size ? `?${params}` : "");
  },
};

const definitions = Object.entries(routePatterns).map(([id, path]) => ({
  id,
  path,
}));
export function parseDrawer(params: URLSearchParams) {
  const value = params.get("tab");
  return {
    courseId: params.get("course") || null,
    tab: (value && drawerTabs.has(value) ? value : "overview") as DrawerTab,
  };
}
export function parseRoute(location: string) {
  const [pathname, search = ""] = location.replace(/^#/, "").split("?");
  const match = matchRoutes(definitions, pathname)?.at(-1);
  const id = match?.route.id;
  const page: Page =
    id === "course" || id === "assignment" || id === "lectures"
      ? "lectures"
      : id === "settings" || id === "section"
        ? "settings"
        : id === "web" || id === "webSite"
          ? "web"
          : "home";
  return {
    page,
    matched: Boolean(match),
    homeView: id === "week" ? ("week" as const) : ("day" as const),
    section: match?.params.section,
    courseId: match?.params.courseId,
    assignmentId: match?.params.aid,
    drawer: parseDrawer(new URLSearchParams(search)),
  };
}

const sections = new Set<string>([
  "accounts",
  "study",
  "notifications",
  "calendar",
  "appearance",
  "files",
  "backup",
  "sources",
  "automation",
  "wifi",
  "developer",
  "agent",
  "extensions",
  "diagnostics",
  "about",
]);
export function isSection(value: string | undefined): value is Section {
  return value !== undefined && sections.has(value);
}
export function settingsSection(
  value: string | undefined,
  context: SectionContext,
): Section {
  if (
    isSection(value) &&
    sectionSupported(value, context) &&
    visibleSections(context).some((item) => item.id === sectionParent(value))
  )
    return value;
  return "accounts";
}

/** Runtime validation waits for courses/capabilities; redirects always replace history. */
export function routeRedirect(
  location: string,
  options: {
    context?: SectionContext;
    courses?: Pick<Course, "id" | "deletedAt">[];
    /** The mobile shell shows the bare /settings route as the section list. */
    stacked?: boolean;
  } = {},
) {
  const route = parseRoute(location);
  let target: string | undefined;
  if (!route.matched) target = paths.home();
  else if (
    route.page === "settings" &&
    !isSection(route.section) &&
    !(options.stacked && route.section === undefined)
  )
    target = paths.settings("accounts");
  else if (
    route.page === "settings" &&
    options.context &&
    settingsSection(route.section, options.context) !== route.section
  )
    target = paths.settings("accounts");
  else if (
    route.courseId &&
    options.courses &&
    !options.courses.some(
      (course) => course.id === route.courseId && !course.deletedAt,
    )
  )
    target = paths.lectures();
  return target
    ? target + (location.includes("?") ? `?${location.split("?")[1]}` : "")
    : undefined;
}

/** Shared data route table. The layout owns Activities instead of rendering an Outlet. */
export function workspaceRoutes(Layout: ComponentType): RouteObject[] {
  const toHome: RouteObject["loader"] = ({ request }) =>
    replace(paths.home() + new URL(request.url).search);
  return [
    {
      path: "/",
      Component: Layout,
      children: [
        { index: true, loader: toHome },
        ...definitions.map(({ id, path }) => ({
          id,
          path,
          element: createElement(Fragment),
          ...(id === "settings"
            ? {
                loader: ({ request }: { request: Request }) =>
                  isStacked()
                    ? null
                    : replace(
                        paths.settings("accounts") +
                          new URL(request.url).search,
                      ),
              }
            : {}),
        })),
        { path: "*", loader: toHome },
      ],
    },
  ];
}
