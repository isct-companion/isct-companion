import { createElement, Fragment, type ComponentType } from "react";
import { matchRoutes, replace, type RouteObject } from "react-router";
import type { Course } from "../lib/model";
import type { Page, Section } from "./shared";
import {
  sectionParent,
  sectionSupported,
  visibleSections,
  type SectionContext,
} from "../pages/settings/registry";

export const routePatterns = {
  home: "/home",
  timetable: "/timetable",
  week: "/home/week",
  lectures: "/lectures",
  exam: "/lectures/exams/:examTerm/:examKey",
  course: "/lectures/:courseId",
  assignment: "/lectures/:courseId/assignments/:aid",
  web: "/web",
  settings: "/settings",
  section: "/settings/:section",
} as const;
const encode = encodeURIComponent;
export const paths = {
  home: (view: "day" | "week" = "day") =>
    view === "week" ? routePatterns.week : routePatterns.home,
  timetable: () => routePatterns.timetable,
  exam: (key: string, term: string) =>
    `${routePatterns.lectures}/exams/${encode(term)}/${encode(key)}`,
  lectures: (courseId?: string, assignmentId?: string) =>
    courseId
      ? `${routePatterns.lectures}/${encode(courseId)}${assignmentId ? `/assignments/${encode(assignmentId)}` : ""}`
      : routePatterns.lectures,
  web: () => routePatterns.web,
  settings: (section?: Section) =>
    section ? `${routePatterns.settings}/${section}` : routePatterns.settings,
  page: (page: Page) => paths[page](),
};

const definitions = Object.entries(routePatterns).map(([id, path]) => ({
  id,
  path,
}));
export function parseRoute(location: string) {
  const [pathname] = location.replace(/^#/, "").split("?");
  const match = matchRoutes(definitions, pathname)?.at(-1);
  const id = match?.route.id;
  const page: Page =
    id === "timetable"
      ? "timetable"
      : id === "course" ||
          id === "assignment" ||
          id === "lectures" ||
          id === "exam"
        ? "lectures"
        : id === "settings" || id === "section"
          ? "settings"
          : id === "web"
            ? "web"
            : "home";
  return {
    page,
    matched: Boolean(match),
    homeView: id === "week" ? ("week" as const) : ("day" as const),
    section: match?.params.section,
    courseId: match?.params.courseId,
    assignmentId: match?.params.aid,
    examKey: match?.params.examKey,
    examTerm: match?.params.examTerm,
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
  } = {},
) {
  const route = parseRoute(location);
  let target: string | undefined;
  if (!route.matched) target = paths.home();
  else if (route.page === "settings" && !isSection(route.section))
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
                  replace(
                    paths.settings("accounts") + new URL(request.url).search,
                  ),
              }
            : {}),
        })),
        { path: "*", loader: toHome },
      ],
    },
  ];
}
