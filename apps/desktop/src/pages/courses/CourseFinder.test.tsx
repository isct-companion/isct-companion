// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MantineProvider } from "@mantine/core";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import type { FinderCourse } from "../../lib/course-query";
import type { FinderView } from "../../platform/finder-query";
import { CourseFinder } from "./CourseFinder";

const finder = vi.hoisted(() => ({
  query: vi.fn(),
  useQuery: vi.fn(),
}));
vi.mock("../../platform/finder-query", () => ({
  finderQuery: finder.query,
  useFinderQuery: finder.useQuery,
}));

const course: FinderCourse = {
  id: "202600001",
  code: "TEST.101",
  year: 2026,
  quarters: ["3Q"],
  title: { ja: "テスト", en: "Test" },
  instructors: [{ ja: "教員", en: "Teacher" }],
  credits: { lecture: 2, exercise: 0, lab: 0 },
  instructionLanguage: "ja",
  format: { kinds: ["lecture"] },
  schedule: [{ kind: "weekly", day: 1, periods: [1, 2] }],
  syllabusUrl: "https://syllabus.s.isct.ac.jp/courses/2026/test",
};
const view: FinderView = {
  context: { currentQuarter: 3, recommended: {}, departments: [], busy: [] },
  courses: [course],
  factors: [],
  duplicates: 0,
  keys: [],
  plan: { unsupported: "empty" },
  rows: [{ course, matched: [] }],
  suggestions: [],
  live: "",
  clashes: [],
  ineligibleIds: [],
  eligibility: {
    [course.id]: { status: "allowed", reason: "noKnownRestriction" },
  },
  replacements: {},
  credits: { [course.id]: 2 },
  onTimetable: [],
  activeSlots: [],
  pasteHandled: false,
  pastedIds: [],
  missingCodes: [],
  ineligibleCodes: [],
  detailIds: [],
};

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
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
let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  finder.useQuery.mockClear();
  finder.useQuery.mockReturnValue({ value: view, pending: false });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});

it("keeps selected courses and withholds Undo when adding fails", async () => {
  const add = vi.fn().mockRejectedValue(new Error("write failed"));
  await act(async () =>
    root.render(
      <MantineProvider>
        <CourseFinder
          year={2026}
          term="2026-3Q"
          language="en"
          keys={[]}
          edit={async () => view}
          replaceKeys={() => {}}
          add={add}
          undo={async () => {}}
        />
      </MantineProvider>,
    ),
  );

  const selected = host.querySelector<HTMLInputElement>(
    `#finder-${course.id}`,
  )!;
  await act(async () => selected.click());
  const addSelected = host.querySelector<HTMLButtonElement>(
    ".finderFoot .primary",
  )!;
  await act(async () => addSelected.click());

  expect(add).toHaveBeenCalledWith([course]);
  expect(selected.checked).toBe(true);
  expect(host.querySelector(".finderError")?.textContent).toContain(
    "write failed",
  );
  expect(host.querySelector(".finderFoot .textButton")).toBeNull();
});

it("shows first-year upper-level courses but disables every add path", async () => {
  const add = vi.fn();
  finder.useQuery.mockReturnValue({
    value: {
      ...view,
      ineligibleIds: [course.id],
      eligibility: {
        [course.id]: {
          status: "ineligible",
          reason: "undergraduateFirstYearUpperLevel",
        },
      },
    },
    pending: false,
  });
  await act(async () =>
    root.render(
      <MantineProvider>
        <CourseFinder
          year={2026}
          term="2026-3Q"
          language="en"
          keys={[]}
          edit={async () => view}
          replaceKeys={() => {}}
          add={add}
          undo={async () => {}}
        />
      </MantineProvider>,
    ),
  );

  expect(
    host.querySelector<HTMLInputElement>(`#finder-${course.id}`)?.disabled,
  ).toBe(true);
  expect(host.querySelector<HTMLButtonElement>(".finderAdd")?.disabled).toBe(
    true,
  );
  expect(host.textContent).toContain(
    "Not available to regular first-year undergraduate students",
  );
  const showUnavailable = [...host.querySelectorAll("button")].find(
    (button) => button.textContent === "Show unavailable courses",
  )!;
  await act(async () => showUnavailable.click());
  expect(finder.useQuery).toHaveBeenCalledWith(
    expect.objectContaining({ includeIneligible: true }),
  );
  expect(add).not.toHaveBeenCalled();
});

it("shows an actionable unknown rule without disabling planning", async () => {
  finder.useQuery.mockReturnValue({
    value: {
      ...view,
      eligibility: {
        [course.id]: {
          status: "unknown",
          reason: "graduateCourseApprovalRequired",
        },
      },
    },
    pending: false,
  });
  await act(async () =>
    root.render(
      <MantineProvider>
        <CourseFinder
          year={2026}
          term="2026-3Q"
          language="en"
          keys={[]}
          edit={async () => view}
          replaceKeys={() => {}}
          add={async () => {}}
          undo={async () => {}}
        />
      </MantineProvider>,
    ),
  );

  expect(host.querySelector<HTMLButtonElement>(".finderAdd")?.disabled).toBe(
    false,
  );
  expect(host.textContent).toContain(
    "Course approval after the research proposal presentation must be confirmed",
  );
});
