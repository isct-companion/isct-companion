// @vitest-environment jsdom
import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MantineProvider } from "@mantine/core";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { appTheme } from "../app/theme";
import { seedWorkspace, type Course, type Workspace } from "../core/model";
import { AcademicSummary } from "../pages/courses/AcademicSummary";
import { Courses } from "../pages/courses/CoursesPage";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
globalThis.ResizeObserver = class ResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
};

type PublicCourse = {
  externalId: string;
  year: number;
  quarter: "1Q" | "2Q" | "3Q" | "4Q";
  code: string;
  title: string;
  instructor: string;
  credits: number;
  language: string;
  meetings: Course["meetings"];
  url: string;
  content: { fetchedAt: string; sourceUrl: string };
};
type OfficialDepartments = {
  admissionYear: number;
  source: { url: string }[];
  departments: Array<{
    id: string;
    name: { ja: string; en: string };
    syllabusScope: string;
    unitCode: string;
    guidePdf: string;
    recommended: Array<{
      category: { ja: string; en?: string };
      level: number;
      code: string;
      group: string;
      mark?: "required" | "recommended";
      title: { ja: string; en?: string };
      credits: { lecture: number; exercise: number; lab: number };
    }>;
  }>;
};

const parsed = JSON.parse(
  readFileSync(
    resolve(
      process.cwd(),
      "artifacts/syllabus-research/course-examples-parsed.json",
    ),
    "utf8",
  ),
) as PublicCourse[];
const official = JSON.parse(
  readFileSync(
    resolve(
      process.cwd(),
      "crates/isct-core/tests/fixtures/subscriptions/data/2026/departments.json",
    ),
    "utf8",
  ),
) as OfficialDepartments;
const official2025 = JSON.parse(
  readFileSync(
    resolve(
      process.cwd(),
      "crates/isct-core/tests/fixtures/subscriptions/data/2025/departments.json",
    ),
    "utf8",
  ),
) as OfficialDepartments;

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.useRealTimers();
  localStorage.removeItem("isct-companion-show-grades");
  vi.restoreAllMocks();
});

function realPlannerWorkspace(): Workspace {
  const workspace = seedWorkspace();
  const source = parsed.find((course) => course.code === "CSC.T354")!;
  const cs = official.departments.find((department) => department.id === "cs")!;
  const recommendation = cs.recommended.find(
    (course) => course.code === "CSC.T354",
  )!;
  expect(source.content.sourceUrl).toBe(source.url);
  expect(official.admissionYear).toBe(2026);
  expect(official.source[0].url).toContain("/guide/guide_2026/");
  expect(cs.guidePdf).toContain("/guide/guide_2026/");

  workspace.selectedTerm = "2026-2Q";
  workspace.terms = ["2026-2Q"];
  workspace.academicProfile.admissionYear = 2026;
  workspace.academicProfile.years = [{
    academicYear: 2026,
    quarterStarts: {
      "1Q": "2026-04-01",
      "2Q": "2026-06-01",
      "3Q": "2026-10-01",
      "4Q": "2026-12-01",
    },
  }];
  workspace.settings.departmentId = "cs";
  workspace.courses = [
    {
      id: `syllabus:${source.externalId}`,
      rev: 1,
      updatedAt: source.content.fetchedAt,
      updatedBy: "public-syllabus-fixture",
      kind: "course",
      termId: "2026-2Q",
      code: source.code,
      title: source.title,
      instructor: source.instructor,
      credits: source.credits,
      creditSplit: { lecture: source.credits, exercise: 0, lab: 0 },
      instructionLanguage: source.language === "Japanese" ? "ja" : "en",
      color: "#52657a",
      meetings: source.meetings,
      url: source.url,
      source: {
        provider: "syllabus",
        externalId: source.externalId,
        fetchedAt: source.content.fetchedAt,
      },
    },
  ];
  workspace.departments = [
    {
      id: `cohort:2026:department:${cs.id}`,
      rev: 1,
      updatedAt: "2026-09-29T00:00:00Z",
      updatedBy: "subscription",
      admissionYear: 2026,
      name: cs.name,
      syllabusScope: cs.syllabusScope,
      unitCode: cs.unitCode,
      guidePdf: cs.guidePdf,
    },
  ];
  workspace.recommendedCourses = [
    {
      id: `cohort:2026:recommended:cs:${recommendation.code}:${recommendation.group}`,
      rev: 1,
      updatedAt: "2026-09-29T00:00:00Z",
      updatedBy: "subscription",
      admissionYear: 2026,
      departmentId: "cs",
      category: recommendation.category,
      level: recommendation.level,
      code: recommendation.code,
      group: recommendation.group,
      mark: recommendation.mark,
      title: recommendation.title,
      credits: recommendation.credits,
    },
  ];
  return workspace;
}

function realRecommendation(
  source: OfficialDepartments,
  admissionYear: number,
): Workspace["recommendedCourses"][number] {
  const cs = source.departments.find((department) => department.id === "cs")!;
  const recommendation = cs.recommended.find(
    (course) => course.code === "CSC.T354",
  )!;
  return {
    id: `cohort:${admissionYear}:recommended:cs:${recommendation.code}:${recommendation.group}`,
    rev: 1,
    updatedAt: "2026-09-29T00:00:00Z",
    updatedBy: "subscription",
    admissionYear,
    departmentId: "cs",
    category: recommendation.category,
    level: recommendation.level,
    code: recommendation.code,
    group: recommendation.group,
    mark: recommendation.mark,
    title: recommendation.title,
    credits: recommendation.credits,
  };
}

function renderPlanner(
  data: Workspace,
  props: Partial<React.ComponentProps<typeof Courses>> = {},
) {
  act(() => {
    root.render(
      <MantineProvider env="test" theme={appTheme} defaultColorScheme="light">
        <Courses
          data={data}
          currentTerm="2026-2Q"
          readOnly={false}
          editing
          update={vi.fn()}
          open={vi.fn()}
          openEvent={vi.fn()}
          addTerm={vi.fn()}
          exportIcs={vi.fn()}
          side={<aside className="finderStub">syllabus search</aside>}
          {...props}
        />
      </MantineProvider>,
    );
  });
}

function onTimetableWorkspace() {
  const data = realPlannerWorkspace();
  const course = data.courses[0];
  data.planItems = [
    {
      id: "plan:parallel",
      rev: 1,
      updatedAt: "2026-09-29T00:00:00Z",
      updatedBy: "test",
      kind: "plan-item",
      termId: course.termId,
      courseId: course.id,
    } as Workspace["planItems"][number],
  ];
  return data;
}

describe("Task 8 one-page planner UI", () => {
  it("puts the syllabus search on the right with no local search or add button", () => {
    renderPlanner(realPlannerWorkspace());
    expect(host.querySelector(".coursesSide .finderStub")).toBeTruthy();
    expect(host.querySelector('input[type="search"]')).toBeNull();
    expect(host.textContent).not.toMatch(/＋ Course|Filters/);
  });

  it("replaces the time on click and appends with ⌘ or Ctrl", () => {
    const onSlot = vi.fn();
    renderPlanner(realPlannerWorkspace(), { onSlot, activeSlots: new Set() });
    const slot = host.querySelector<HTMLButtonElement>(".ttSlot")!;
    act(() => slot.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    act(() =>
      slot.dispatchEvent(
        new MouseEvent("click", { bubbles: true, metaKey: true }),
      ),
    );
    act(() =>
      slot.dispatchEvent(
        new MouseEvent("click", { bubbles: true, ctrlKey: true }),
      ),
    );
    expect(onSlot.mock.calls.map((call) => call[3])).toEqual([
      false,
      true,
      true,
    ]);
    // Weekdays by default; the full week on request.
    expect(host.querySelectorAll(".ttCol")).toHaveLength(5);
    act(() => host.querySelector<HTMLButtonElement>(".weekLength")!.click());
    const columns = host.querySelectorAll(".ttCol");
    expect(columns).toHaveLength(7);
    // Sunday has no slots to pick.
    expect(columns[6].querySelector(".ttSlot")).toBeNull();
    expect(columns[5].querySelector(".ttSlot")).toBeTruthy();
    act(() => host.querySelector<HTMLButtonElement>(".weekLength")!.click());
  });

  it("opens a course's details instead of removing it on click", () => {
    const open = vi.fn();
    const update = vi.fn();
    const data = onTimetableWorkspace();
    renderPlanner(data, { open, update });
    const block = host.querySelector<HTMLButtonElement>(
      ".ttBlock.course, .intensiveLane > div button",
    )!;
    act(() => block.click());
    expect(open).toHaveBeenCalledWith(data.courses[0].id);
    expect(update).not.toHaveBeenCalled();
  });

  it("shows the course panel over the search when given one", () => {
    renderPlanner(realPlannerWorkspace(), {
      panel: <div className="panelStub">details</div>,
    });
    expect(host.querySelector(".coursesSidePanel .panelStub")).toBeTruthy();
    expect(host.querySelector(".coursesSide .finderStub")).toBeTruthy();
  });

  it("shows a search result's details over the timetable", () => {
    renderPlanner(realPlannerWorkspace(), {
      leftPanel: <div className="offeringStub">result</div>,
    });
    expect(
      host.querySelector(".coursesLeft .coursesSidePanel .offeringStub"),
    ).toBeTruthy();
    expect(host.querySelector(".coursesLeft .ttCard")).toBeTruthy();
  });

  it("uses the real 2025 guide row for a 2025 admission profile", () => {
    expect(official2025.admissionYear).toBe(2025);
    expect(official2025.source).toEqual([
      expect.objectContaining({
        url: "https://www.titech.ac.jp/guide/guide_2025/gakubu1/",
      }),
    ]);
    const cs2025 = official2025.departments.find(
      (department) => department.id === "cs",
    )!;
    expect(cs2025.guidePdf).toBe(
      "https://www.titech.ac.jp/guide/guide_2025/gakubu1/pdf/26-13.pdf",
    );
    expect(realRecommendation(official2025, 2025).admissionYear).toBe(2025);
  });
});

describe("Unified schedule view and editing", () => {
  it("uses actual dates in view mode, keeps plans on school holidays, and removes editing controls", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-06-01T12:00:00+09:00"));
    const data = onTimetableWorkspace();
    data.courses[0].meetings = [
      { days: [1], startTime: "10:45", endTime: "12:25", room: "Classroom" },
    ];
    data.exceptions.push({
      id: "holiday",
      date: "2026-06-01",
      type: "no-class",
      rev: 1,
      updatedAt: "2026-06-01T00:00:00Z",
      updatedBy: "test",
    });
    data.plans.push({
      id: "personal",
      title: "Personal meeting",
      start: "2026-06-01T14:00",
      duration: 60,
      exdates: [],
      room: "Library",
      rev: 1,
      updatedAt: "2026-06-01T00:00:00Z",
      updatedBy: "test",
    });
    const onEditingChange = vi.fn();
    const openPlan = vi.fn();
    const exportIcs = vi.fn();
    renderPlanner(data, {
      editing: false,
      onEditingChange,
      openPlan,
      exportIcs,
      pullLms: vi.fn(),
      onSlot: vi.fn(),
    });
    expect(host.querySelectorAll(".ttHead small")).toHaveLength(7);
    expect(host.querySelector(".ttBlock.course.cancelled")).not.toBeNull();
    expect(host.querySelector(".ttCol.noClasses")).not.toBeNull();
    expect(host.querySelector(".ttBlock.plan")?.textContent).toContain(
      "Personal meeting",
    );
    expect(host.querySelector(".finderStub")).toBeNull();
    expect(host.querySelector(".ttSlot")).toBeNull();
    expect(host.textContent).not.toContain("Pull from LMS");
    expect(host.textContent).not.toContain("＋ Event");
    const exportButton = [
      ...host.querySelectorAll<HTMLButtonElement>("button"),
    ].find((button) => button.textContent === "Export ICS")!;
    await act(async () => exportButton.click());
    expect(exportIcs).toHaveBeenCalledWith("2026-2Q", "courses");
    await act(async () =>
      host.querySelector<HTMLButtonElement>(".ttBlock.plan")!.click(),
    );
    expect(document.querySelector('[role="dialog"]')?.textContent).toContain(
      "Library",
    );
    expect(openPlan).not.toHaveBeenCalled();
    const editButton = [
      ...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button'),
    ].find((button) => button.textContent === "Edit")!;
    await act(async () => editButton.click());
    expect(onEditingChange).toHaveBeenCalledWith(true);
    expect(openPlan).toHaveBeenCalledWith(data.plans[0]);
  });

  it("renders grades only after Details is clicked, ignoring the old visibility preference", async () => {
    localStorage.setItem("isct-companion-show-grades", "true");
    const renderSummary = async () =>
      await act(async () =>
        root.render(
          <MantineProvider env="test">
            <AcademicSummary
              termCredits={12}
              summaries={[
                {
                  id: "grades",
                  scope: "all",
                  gpa: 3.71,
                  gpt: 99.5,
                  earnedCredits: 120,
                  fetchedAt: "2026-10-01T00:00:00Z",
                  rev: 1,
                  updatedAt: "2026-10-01T00:00:00Z",
                  updatedBy: "test",
                },
              ]}
              openOfficial={vi.fn()}
            />
          </MantineProvider>,
        ),
      );
    await renderSummary();
    const details = () =>
      [...host.querySelectorAll<HTMLButtonElement>("button")].find(
        (button) => button.textContent === "Details",
      )!;
    expect(host.textContent).toContain("This term 12 credits");
    expect(document.querySelector("table")).toBeNull();
    expect(host.textContent).not.toContain("GPA");
    await act(async () => details().click());
    expect(document.querySelector("table")?.textContent).toContain("3.71");
    expect(host.querySelector(".academicQuarterLabel")?.textContent).not.toContain("3.71");
    expect(document.querySelector('input[role="switch"]')).toBeNull();
    await act(async () => details().click());
    expect(document.querySelector("table")).toBeNull();
    await act(async () =>
      root.render(
        <MantineProvider env="test">
          <div />
        </MantineProvider>,
      ),
    );
    await renderSummary();
    expect(document.querySelector("table")).toBeNull();
  });
});
