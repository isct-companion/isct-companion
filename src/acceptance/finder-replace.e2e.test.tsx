// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { seedWorkspace } from "../core/model";
import type { WorkspaceIntent } from "../core/workspace-intents";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
globalThis.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as never;
window.matchMedia = ((query: string) => ({
  matches: false,
  media: query,
  addEventListener() {},
  removeEventListener() {},
  addListener() {},
  removeListener() {},
  onchange: null,
  dispatchEvent: () => false,
})) as never;

const offering = (id: string, code: string, title: string) => ({
  id,
  year: 2026,
  quarters: ["3Q"],
  code,
  title: { ja: title, en: title },
  instructors: [{ ja: "教員", en: "Teacher" }],
  syllabusUrl: `https://syllabus.s.isct.ac.jp/courses/2026/4/0-904-342300-0-0/${id}`,
  credits: { lecture: 2, exercise: 0, lab: 0 },
  instructionLanguage: "ja",
  format: { kinds: ["lecture"] },
  schedule: [{ kind: "weekly", day: 2, periods: [7, 8] }],
});
const placed = offering("202602421", "CSC.T252", "Placed");
const clashing = offering("202600009", "CSC.T999", "Clashing");

const before = (() => {
  const workspace = seedWorkspace();
  workspace.selectedTerm = "2026-3Q";
  workspace.terms = ["2026-3Q"];
  workspace.courses.push({
    id: "2026-202602421",
    kind: "course",
    rev: 1,
    updatedAt: "2026-10-01T00:00:00.000Z",
    updatedBy: workspace.deviceId,
    termId: "2026-3Q",
    offeringId: placed.id,
    names: { title: placed.title, instructor: placed.instructors[0] },
    code: placed.code,
    title: "Placed",
    instructor: "Teacher",
    credits: 2,
    creditSplit: placed.credits,
    instructionLanguage: "ja",
    color: "#556b8d",
    meetings: [
      {
        days: [2],
        periods: [7, 8],
        startTime: "15:25",
        endTime: "17:05",
        room: "",
      },
    ],
    url: placed.syllabusUrl,
    source: { provider: "syllabus", externalId: placed.id },
  });
  workspace.planItems.push({
    id: "018f0000-0000-7000-8000-000000000002",
    kind: "plan-item",
    rev: 1,
    updatedAt: "2026-10-01T00:00:00.000Z",
    updatedBy: workspace.deviceId,
    termId: "2026-3Q",
    courseId: "2026-202602421",
    offeringId: placed.id,
    state: "planned",
  });
  return workspace;
})();
const after = structuredClone(before);
after.courses = [
  {
    ...before.courses[0],
    id: "2026-202600009",
    offeringId: clashing.id,
    names: { title: clashing.title, instructor: clashing.instructors[0] },
    code: clashing.code,
    title: "Clashing",
    url: clashing.syllabusUrl,
    source: { provider: "syllabus", externalId: clashing.id },
  },
];
after.planItems = [
  {
    ...before.planItems[0],
    courseId: "2026-202600009",
    offeringId: clashing.id,
  },
];

let saved = before;
let revision = "fixture:before";
const intents: WorkspaceIntent[] = [];
const schoolRecords = {
  lastRuns: {},
  lmsCourses: [],
  registrations: [],
  registrationStates: [],
  grades: [],
  gpaByTerm: [],
  requirements: [],
};
const activeCourse = () => saved.courses[0];
const overview = () => ({
  term: "2026-3Q",
  currentTerm: "2026-3Q",
  selectedTerm: "2026-3Q",
  todayTerm: "2026-3Q",
  coursesTerm: "2026-3Q",
  range: { from: "2026-09-28", to: "2026-12-02" },
  courses: [activeCourse()],
  declaredCredits: null,
  termCredits: 2,
  colors: { [activeCourse().id]: activeCourse().color },
  enrolledCourseIds: [activeCourse().id],
  onTimetableCourseIds: [activeCourse().id],
  allTerms: ["2026-3Q"],
  defaultFinderKeys: [],
});
const planningWeek = () => ({
  courses: [activeCourse()],
  days: Array.from({ length: 7 }, (_, index) => ({
    day: index + 1,
    blocks:
      index === 1
        ? [
            {
              key: `${activeCourse().id}-fixture`,
              id: activeCourse().id,
              courseId: activeCourse().id,
              kind: "course",
              date: "",
              title: activeCourse().title,
              subtitle: activeCourse().code,
              room: "",
              startTime: "15:25",
              endTime: "17:05",
              color: activeCourse().color,
            },
          ]
        : [],
  })),
  conflictCount: 0,
});
const finderView = (request: Record<string, unknown>) => ({
  context: {
    currentQuarter: 3,
    recommended: {},
    departments: [],
    busy: [{ day: 2, from: 7, to: 8 }],
  },
  courses: [placed, clashing],
  factors: [],
  duplicates: 0,
  keys: request.keys ?? [],
  plan: { unsupported: "empty" },
  rows: [placed, clashing].map((course) => ({ course, matched: [] })),
  suggestions: [],
  live: String(request.input ?? ""),
  clashes: saved === before ? [clashing.id] : [],
  replacements:
    saved === before ? { [clashing.id]: [before.courses[0].id] } : {},
  credits: { [placed.id]: 2, [clashing.id]: 2 },
  onTimetable: [activeCourse().code],
  activeSlots: ["火7-8"],
  pasteHandled: false,
  pastedIds: [],
  missingCodes: [],
  detailIds: [],
});
(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
  transformCallback: () => 1,
  unregisterCallback: () => {},
  invoke: async (command: string, args: Record<string, unknown>) => {
    switch (command) {
      case "workspace_snapshot":
        return { revision, workspace: structuredClone(saved), schoolRecords };
      case "mutate_workspace": {
        const mutation = args.mutation as {
          expectedRevision: string;
          intent: WorkspaceIntent;
        };
        expect(mutation.expectedRevision).toBe(revision);
        intents.push(mutation.intent);
        if (mutation.intent.kind === "addCatalogCourses") {
          saved = after;
          revision = "fixture:after";
        } else if (mutation.intent.kind === "undoCatalogCourses") {
          saved = before;
          revision = "fixture:undo";
        }
        return { revision, workspace: structuredClone(saved), schoolRecords };
      }
      case "schedule_query":
        return {
          revision,
          values: (args.queries as { kind: string }[]).map((query) => ({
            kind: query.kind,
            value:
              query.kind === "workspaceOverview"
                ? overview()
                : query.kind === "planningWeek"
                  ? planningWeek()
                  : query.kind === "scheduleView"
                    ? { ...overview(), agendas: [] }
                    : null,
          })),
        };
      case "finder_query":
        return finderView(args.request as Record<string, unknown>);
      case "catalog_offerings":
        return [placed, clashing];
      case "catalog_query":
        return {
          items: [{ id: placed.id }, { id: clashing.id }],
          total: 2,
          returned: 2,
          truncated: false,
        };
      case "catalog_sync_offerings":
        return [];
      case "school_records":
        return {
          lastRuns: {},
          lmsCourses: [],
          registrations: [],
          registrationStates: [],
          grades: [],
          gpaByTerm: [],
          requirements: [],
        };
      case "runtime_capabilities":
        return {};
      case "school_credential_status":
        return {
          science: {
            saved: false,
            username: "",
            hasOtp: false,
            hasMatrix: false,
          },
          tokyoTech: {
            saved: false,
            username: "",
            hasOtp: false,
            hasMatrix: false,
          },
        };
      default:
        return null;
    }
  },
};

const wait = (ms: number) =>
  act(async () => new Promise((resolve) => setTimeout(resolve, ms)));
const titles = (host: HTMLElement) => [
  ...new Set(
    [...host.querySelectorAll(".ttBlock b")].map((b) => b.textContent),
  ),
];

it("adds a clashing course as a replacement, and Undo brings the old one back", async () => {
  const errors = vi.spyOn(console, "error").mockImplementation(() => {});
  const { default: App } = await import("../app/App");
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  await act(async () => root.render(<App />));
  const nav = await vi.waitFor(() => {
    const button = [...host.querySelectorAll("button")].find((b) =>
      /^Home/.test(b.getAttribute("aria-label") ?? b.textContent ?? ""),
    );
    if (!button) throw new Error("app not ready");
    return button;
  });
  await act(async () => nav!.click());
  const edit = host.querySelector<HTMLInputElement>('input[role="switch"]')!;
  await act(async () => edit.click());
  const input = host.querySelector<HTMLInputElement>("#course-finder-input")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(input, "CSC ");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(titles(host)).toEqual(["Placed"]);
  const replace = await vi.waitFor(
    () => {
      const button = [
        ...host.querySelectorAll<HTMLButtonElement>(".finderAdd"),
      ].find((candidate) => candidate.textContent === "Replace");
      if (!button) throw new Error("no Replace yet");
      return button;
    },
    { timeout: 5000 },
  );
  await act(async () => replace.click());
  await vi.waitFor(() => expect(titles(host)).toEqual(["Clashing"]));

  const undo = [...host.querySelectorAll<HTMLButtonElement>("button")].find(
    (b) => b.textContent === "Undo",
  );
  await act(async () => undo!.click());
  await vi.waitFor(() => expect(titles(host)).toEqual(["Placed"]));
  expect(intents).toEqual([
    {
      kind: "addCatalogCourses",
      termId: "2026-3Q",
      offerings: [clashing],
    },
    {
      kind: "undoCatalogCourses",
      termId: "2026-3Q",
      offeringIds: [clashing.id],
    },
  ]);
  await wait(50);
  expect(host.querySelector(".pageError")).toBeNull();
  // React's "not wrapped in act" notices come from async polling, not from the app.
  const real = errors.mock.calls.filter(
    (call) => !String(call[0]).includes("not wrapped in act"),
  );
  expect(real).toEqual([]);
  errors.mockRestore();
  act(() => root.unmount());
}, 15_000);
