// @vitest-environment jsdom
import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { seedWorkspace, type Course } from "../../lib/model";
import type { LmsCourseRecord } from "../../platform/desktop-bridge";
import { LmsWorkspace } from "./LmsWorkspace";
import { safeContent } from "./content";
import { TodayWorkbench } from "./TodayWorkbench";

vi.mock("./PdfDocument", () => ({
  PdfDocument: () => <canvas aria-label="PDF page" />,
}));
const core = vi.hoisted(() => ({
  timeline: undefined as
    { classAt?: string; lastStarted?: string; next?: string } | undefined,
  tasks: undefined as unknown,
  requests: [] as Array<{ now: string }>,
}));
vi.mock("../../platform/core-query", () => ({
  useScheduleQuery: (request: { kind: string; now: string }) => {
    core.requests.push(request);
    return {
      value: request.kind === "taskView" ? core.tasks : core.timeline,
      pending: false,
    };
  },
}));

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement;
let root: Root;
let fixtureIndex = 0;
beforeEach(() => {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  sessionStorage.clear();
  core.timeline = undefined;
  core.tasks = undefined;
  core.requests = [];
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
function fixture() {
  const data = seedWorkspace();
  data.deviceId = `lms-test-${fixtureIndex++}`;
  const meta = {
    rev: 1,
    updatedAt: "2026-10-05T00:00:00Z",
    updatedBy: "test",
    source: { provider: "lms" as const, externalId: "42" },
  };
  const course: Course = {
    ...meta,
    id: "test-course",
    kind: "course",
    termId: "2026-3Q",
    code: "TEST",
    title: "Test course",
    instructor: "",
    credits: 2,
    creditSplit: { lecture: 2, exercise: 0, lab: 0 },
    instructionLanguage: "ja",
    color: "blue",
    meetings: [],
  };
  data.courses = [course];
  data.assignments = [
    {
      ...meta,
      id: "a1",
      kind: "assignment",
      courseId: course.id,
      lmsCmid: 100,
      title: "Report",
      details: "Requirement body",
      dueAt: "2026-10-10T00:00:00Z",
      submission: "open",
      sourceUrl: "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=100",
    },
  ];
  data.materials = [
    {
      ...meta,
      id: "m1",
      kind: "material",
      courseId: course.id,
      title: "Lecture file",
      type: "link",
      session: 9,
      sectionNumber: 9,
      sectionTitle: "Duplicate",
      bodyHtml: "<p>Readable file body</p>",
      module: "resource",
    },
  ];
  const record: LmsCourseRecord = {
    id: "lms42",
    lmsId: 42,
    shortname: "TEST[2026]",
    title: { en: "Test" },
    quarters: ["3Q"],
    url: "https://lms.s.isct.ac.jp/2025/course/view.php?id=42",
    sections: [
      {
        id: 91,
        number: 9,
        title: "Duplicate",
        visible: true,
        bodyHtml: `<p>${"Earlier text ".repeat(30)}searchable tail</p>`,
      },
      {
        id: 81,
        number: 8,
        title: "Duplicate",
        visible: true,
        bodyHtml: "<p>Text-only lesson</p>",
      },
      { id: 71, number: 7, title: "Empty section", visible: true },
    ],
  };
  return { data, course, record };
}
const click = (button: Element) =>
  act(() => (button as HTMLButtonElement).click());
const button = (text: string) =>
  [...host.querySelectorAll("button")].find((b) =>
    b.textContent?.includes(text),
  )!;
describe("LMS reading workspace", () => {
  it("refreshes the course timeline when resumed and while active", async () => {
    vi.useFakeTimers({ now: new Date("2026-10-08T00:00:00Z") });
    const f = fixture();
    await act(async () =>
      root.render(
        <LmsWorkspace
          {...f}
          entry={{ courseId: f.course.id, request: 1 }}
          active={false}
          schoolWebAvailable={false}
          close={() => {}}
        />,
      ),
    );
    await act(async () => vi.advanceTimersByTime(120_000));
    expect(core.requests.at(-1)?.now).toBe("2026-10-08T00:00:00.000Z");

    await act(async () =>
      root.render(
        <LmsWorkspace
          {...f}
          entry={{ courseId: f.course.id, request: 1 }}
          active
          schoolWebAvailable={false}
          close={() => {}}
        />,
      ),
    );
    expect(core.requests.at(-1)?.now).toBe("2026-10-08T00:02:00.000Z");

    await act(async () => vi.advanceTimersByTime(60_000));
    expect(core.requests.at(-1)?.now).toBe("2026-10-08T00:03:00.000Z");
  });

  it("renders common text activities inline without opening a document panel", async () => {
    const f = fixture();
    f.data.materials[0].module = "page";
    await act(async () =>
      root.render(
        <LmsWorkspace
          {...f}
          entry={{ courseId: f.course.id, request: 99 }}
          active
          schoolWebAvailable={false}
          close={() => {}}
        />,
      ),
    );
    expect(
      host.querySelector(".lmsSectionBody .lmsInlineActivity")?.textContent,
    ).toContain("Readable file body");
    expect(host.querySelector(".lmsViewer")).toBeNull();
  });

  it("lays out every lesson as a page, opens where the course is, and keeps other sections as rail tabs", async () => {
    const f = fixture();
    f.record.sections = [
      { id: 10, number: 0, title: "一般", visible: true },
      {
        id: 11,
        number: 1,
        title: "第1回",
        visible: true,
        bodyHtml: "<p>授業計画: 導入</p>",
      },
      {
        id: 12,
        number: 2,
        title: "第2回",
        visible: true,
        bodyHtml: "<p>授業計画: Java の基本</p>",
      },
      {
        id: 13,
        number: 3,
        title: "第3回",
        visible: true,
        bodyHtml: "<p>授業計画: クラス</p>",
      },
    ];
    const material = f.data.materials[0];
    f.data.materials = [11, 12].map((sectionId) => ({
      ...material,
      id: `m${sectionId}`,
      title: `File ${sectionId}`,
      sectionId,
      sectionNumber: sectionId - 10,
      sectionTitle: `第${sectionId - 10}回`,
      bodyHtml: undefined,
    }));
    await act(async () =>
      root.render(
        <LmsWorkspace
          {...f}
          entry={{ courseId: f.course.id, request: 97 }}
          active
          schoolWebAvailable={false}
          close={() => {}}
        />,
      ),
    );
    const rail = [
      ...host.querySelectorAll(".lmsLessonRail button:not(.lmsRailTab)"),
    ];
    expect(rail.map((b) => b.textContent)).toEqual(["1", "2", "3"]);
    // The course is at 第2回, the latest lesson with material.
    expect(rail[1].getAttribute("aria-current")).toBe("step");
    // Every lesson is laid out at once, each as a page of the outline.
    const pages = [...host.querySelectorAll(".lmsLessonPage")];
    expect(pages.map((page) => page.getAttribute("data-page"))).toEqual([
      "11",
      "12",
      "13",
    ]);
    expect(pages[0].textContent).toContain("File 11");
    expect(pages[1].textContent).toContain("File 12");
    // 一般 has no number, and the report belongs to no section: both are tabs before the numbers.
    expect(
      [...host.querySelectorAll(".lmsRailTab")].map((tab) => tab.textContent),
    ).toEqual(["一般", "Activities"]);
    // Let the arrival (which opens the course's page) finish before navigating.
    await act(
      async () =>
        new Promise((resolve) =>
          requestAnimationFrame(() => resolve(undefined)),
        ),
    );
    await click(rail[0]);
    expect(rail[0].getAttribute("aria-current")).toBe("step");
  });

  it("opens a folder in the outline from the files already stored with it", async () => {
    const f = fixture();
    Object.assign(f.data.materials[0], {
      title: "講義資料",
      module: "folder",
      url: "https://lms.s.isct.ac.jp/2025/mod/folder/view.php?id=9",
      bodyHtml:
        '<ul><li><a href="https://lms.s.isct.ac.jp/2025/pluginfile.php/1/mod_folder/content/0/20260928.zip?forcedownload=1">20260928.zip</a></li></ul>',
    });
    await act(async () =>
      root.render(
        <LmsWorkspace
          {...f}
          entry={{ courseId: f.course.id, request: 96 }}
          active
          schoolWebAvailable={false}
          close={() => {}}
        />,
      ),
    );
    await act(async () =>
      (
        host.querySelector(
          '.lmsFolder > [aria-expanded="false"]',
        ) as HTMLButtonElement
      ).click(),
    );
    await vi.waitFor(() =>
      expect(host.querySelector(".lmsFolderFile")?.textContent).toContain(
        "20260928.zip",
      ),
    );
    expect(host.querySelector(".lmsViewer")).toBeNull();
  });

  it("marks a lesson for the coming class on double-click, and a mark sets where the course is", async () => {
    vi.useFakeTimers({
      now: new Date("2026-10-08T00:00:00Z"),
      toFake: ["Date"],
    });
    const f = fixture();
    f.data.academicProfile.years = [
      {
        academicYear: 2026,
        quarterStarts: {
          "1Q": "2026-04-06",
          "2Q": "2026-06-10",
          "3Q": "2026-10-01",
          "4Q": "2026-12-08",
        },
      },
    ];
    f.course.meetings = [
      {
        days: [4],
        periods: [3, 4],
        startTime: "10:45",
        endTime: "12:25",
        room: "M-B07",
      },
    ];
    f.data.memberships = [
      {
        id: "mem",
        kind: "membership",
        rev: 1,
        updatedAt: "2026-10-05T00:00:00Z",
        updatedBy: "test",
        termId: "2026-3Q",
        courseId: f.course.id,
        status: "lms-member",
        evidence: { provider: "lms", externalId: "42" },
      },
    ];
    f.record.sections = [1, 2, 3].map((number) => ({
      id: 10 + number,
      number,
      title: `第${number}回`,
      visible: true,
    }));
    f.data.materials = [];
    f.course.overrides = {
      lessonLog: [{ occurrence: "2026-10-08T10:45:00+09:00", sectionId: 12 }],
    };
    core.timeline = {
      lastStarted: "2026-10-01T10:45:00+09:00",
      next: "2026-10-08T10:45:00+09:00",
      classAt: "2026-10-08T10:45:00+09:00",
    };
    const markLesson = vi.fn();
    await act(async () =>
      root.render(
        <LmsWorkspace
          {...f}
          entry={{ courseId: f.course.id, request: 95 }}
          active
          schoolWebAvailable={false}
          close={() => {}}
          markLesson={markLesson}
        />,
      ),
    );
    const rail = [
      ...host.querySelectorAll(".lmsLessonRail button:not(.lmsRailTab)"),
    ];
    // Nothing is posted, yet the mark for today's 10:45 class puts the course at 第2回.
    expect(rail[1].getAttribute("aria-current")).toBe("step");
    expect(rail.map((b) => b.classList.contains("held"))).toEqual([
      true,
      true,
      false,
    ]);
    await act(async () =>
      rail[2].dispatchEvent(new MouseEvent("dblclick", { bubbles: true })),
    );
    expect(markLesson).toHaveBeenCalledWith(
      13,
      "2026-10-08T10:45:00+09:00",
      "Lesson 3",
    );
  });

  it("after today's class has ended, a mark is for that class and the lesson stays current", async () => {
    // 14:00 in Japan, after the 10:45–12:25 class.
    vi.useFakeTimers({
      now: new Date("2026-10-08T05:00:00Z"),
      toFake: ["Date"],
    });
    const f = fixture();
    f.data.academicProfile.years = [
      {
        academicYear: 2026,
        quarterStarts: {
          "1Q": "2026-04-06",
          "2Q": "2026-06-10",
          "3Q": "2026-10-01",
          "4Q": "2026-12-08",
        },
      },
    ];
    f.course.meetings = [
      {
        days: [4],
        periods: [3, 4],
        startTime: "10:45",
        endTime: "12:25",
        room: "M-B07",
      },
    ];
    f.data.memberships = [
      {
        id: "mem",
        kind: "membership",
        rev: 1,
        updatedAt: "2026-10-05T00:00:00Z",
        updatedBy: "test",
        termId: "2026-3Q",
        courseId: f.course.id,
        status: "lms-member",
        evidence: { provider: "lms", externalId: "42" },
      },
    ];
    f.record.sections = [1, 2, 3, 4].map((number) => ({
      id: 10 + number,
      number,
      title: `第${number}回`,
      visible: true,
    }));
    f.data.materials = [];
    f.course.overrides = {
      lessonLog: [{ occurrence: "2026-10-08T10:45:00+09:00", sectionId: 14 }],
    };
    core.timeline = {
      lastStarted: "2026-10-08T10:45:00+09:00",
      next: "2026-10-15T10:45:00+09:00",
      classAt: "2026-10-08T10:45:00+09:00",
    };
    const markLesson = vi.fn();
    await act(async () =>
      root.render(
        <LmsWorkspace
          {...f}
          entry={{ courseId: f.course.id, request: 94 }}
          active
          schoolWebAvailable={false}
          close={() => {}}
          markLesson={markLesson}
        />,
      ),
    );
    const rail = [
      ...host.querySelectorAll(".lmsLessonRail button:not(.lmsRailTab)"),
    ];
    expect(rail[3].getAttribute("aria-current")).toBe("step");
    await act(async () =>
      rail[2].dispatchEvent(new MouseEvent("dblclick", { bubbles: true })),
    );
    expect(markLesson).toHaveBeenCalledWith(
      13,
      "2026-10-08T10:45:00+09:00",
      "Lesson 3",
    );
  });

  it("shows a label's own text without the name Moodle derives from it", async () => {
    const f = fixture();
    Object.assign(f.data.materials[0], {
      module: "label",
      title: "日付 授業回目 内容…",
      bodyHtml:
        "<table><tr><td>日付</td><td>授業回目</td><td>内容</td></tr></table>",
    });
    await act(async () =>
      root.render(
        <LmsWorkspace
          {...f}
          entry={{ courseId: f.course.id, request: 98 }}
          active
          schoolWebAvailable={false}
          close={() => {}}
        />,
      ),
    );
    const label = host.querySelector(".lmsSectionBody .lmsInlineActivity")!;
    expect(label.querySelector("table")).not.toBeNull();
    expect(label.querySelector("h4")).toBeNull();
  });

  it("filters bilingual LMS text by the school content language, not the UI language", () => {
    const f = fixture();
    f.data.settings.locale = "en";
    f.data.settings.contentLanguage = "ja";
    f.data.settings.lmsBilingualFilter = true;
    f.record.sections![0].bodyHtml =
      '<p lang="en">English summary</p><p lang="ja">日本語の概要</p>';
    act(() =>
      root.render(
        <LmsWorkspace
          {...f}
          entry={{ courseId: f.course.id, request: 700 }}
          active
          schoolWebAvailable={false}
          close={() => {}}
        />,
      ),
    );
    const body = host.querySelector(".lmsSectionBody")?.textContent ?? "";
    expect(body).toContain("日本語の概要");
    expect(body).not.toContain("English summary");
  });

  it("opens an English announcement card in the right reader without expanding its full body on the left", () => {
    const f = fixture();
    f.data.settings.locale = "en";
    f.data.materials[0].module = "forum";
    f.data.materials[0].bodyHtml =
      '<article id="announcement-p42" data-source-url="https://lms.s.isct.ac.jp/2025/mod/forum/discuss.php?d=42"><h5>Classroom change</h5><time>1 October</time><p>Please meet in W3-707. Bring your notes.</p></article>';
    act(() =>
      root.render(
        <LmsWorkspace
          {...f}
          entry={{ courseId: f.course.id, request: 600 }}
          active
          schoolWebAvailable={false}
          close={() => {}}
        />,
      ),
    );
    expect(host.querySelector(".lmsViewer")).toBeNull();
    const card = host.querySelector(".lmsAnnouncementCard")!;
    expect(card.textContent).toContain("Classroom change");
    click(card);
    expect(
      host.querySelector(".lmsViewer .lmsRichContent")?.textContent,
    ).toContain("Bring your notes.");
    expect(
      host.querySelector(".lmsViewerClose")?.getAttribute("aria-label"),
    ).toContain("Classroom change");
  });

  it("shows an announcement forum without posts as empty in the outline, with its official page", () => {
    const f = fixture();
    f.data.settings.locale = "en";
    Object.assign(f.data.materials[0], {
      title: "アナウンスメント",
      module: "forum",
      bodyHtml: "",
      url: "https://lms.s.isct.ac.jp/2025/mod/forum/view.php?id=7",
    });
    act(() =>
      root.render(
        <LmsWorkspace
          {...f}
          entry={{ courseId: f.course.id, request: 610 }}
          active
          schoolWebAvailable={false}
          close={() => {}}
        />,
      ),
    );
    // Nothing to read, so the outline says so in place instead of opening the reader.
    const forum = [...host.querySelectorAll(".lmsInlineActivity")].find(
      (item) => item.textContent?.includes("アナウンスメント"),
    )!;
    expect(forum.textContent).toContain("No announcements yet.");
    expect(host.querySelector(".lmsViewer")).toBeNull();
    click(button("Open in LMS"));
    expect(host.querySelector(".lmsSourceBar")?.textContent).toContain(
      "Official LMS",
    );
  });

  it("hides the redundant syllabus entry and opens its dedicated right panel", () => {
    const f = fixture();
    f.course.offeringId = "20260001";
    f.data.materials.push({
      ...f.data.materials[0],
      id: "syllabus-item",
      title: "シラバス",
      module: "url",
    });
    act(() =>
      root.render(
        <LmsWorkspace
          {...f}
          entry={{ courseId: f.course.id, request: 502 }}
          active
          schoolWebAvailable={false}
          close={() => {}}
        />,
      ),
    );
    expect(
      [...host.querySelectorAll(".lmsItemRow")].some((row) =>
        row.textContent?.includes("シラバス"),
      ),
    ).toBe(false);
    click(
      [...host.querySelectorAll("button")].find(
        (button) => button.textContent?.trim() === "Syllabus",
      )!,
    );
    expect(
      host.querySelector(".lmsViewerClose")?.getAttribute("aria-label"),
    ).toContain("Syllabus");
  });

  it("keeps unpreviewable attachments in the outline as a popover card", async () => {
    const f = fixture();
    const url =
      "https://lms.s.isct.ac.jp/2025/pluginfile.php/42/mod_assign/introattachment/0/report.zip?forcedownload=1";
    f.record.sections![0].bodyHtml = `<p><a href="${url}">Report ZIP</a></p>`;
    const popup = vi.spyOn(window, "open").mockReturnValue(null);
    act(() =>
      root.render(
        <LmsWorkspace
          {...f}
          entry={{ courseId: f.course.id, request: 601 }}
          active
          schoolWebAvailable={false}
          close={() => {}}
        />,
      ),
    );
    click(
      [...host.querySelectorAll("a")].find(
        (link) =>
          link.querySelector(".lmsAttachmentName")?.textContent ===
          "Report ZIP",
      )!,
    );
    await act(async () => {
      await vi.dynamicImportSettled();
    });
    expect(popup).not.toHaveBeenCalled();
    expect(host.querySelector(".lmsViewer")).toBeNull();
    expect(
      host.querySelector(".lmsOutline .lmsFilePopover .lmsFileCard h3")
        ?.textContent,
    ).toBe("Report ZIP");
    act(() =>
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })),
    );
    expect(host.querySelector(".lmsFilePopover")).toBeNull();
  });

  it("puts consecutive image-only labels in one grid", () => {
    const f = fixture();
    const label = (id: string, name: string) => ({
      ...f.data.materials[0],
      id,
      title: name,
      module: "label",
      resourceUrl: undefined,
      bodyHtml: `<div><p><a href="https://lms.s.isct.ac.jp/2025/pluginfile.php/1/mod_label/intro/${name}.png"><img src="https://lms.s.isct.ac.jp/2025/pluginfile.php/1/mod_label/intro/s_${name}.png" alt=""></a></p></div>`,
    });
    f.data.materials = [
      label("l1", "schedule"),
      label("l2", "discord"),
      ...f.data.materials,
    ];
    act(() =>
      root.render(
        <LmsWorkspace
          {...f}
          entry={{ courseId: f.course.id, request: 602 }}
          active
          schoolWebAvailable={false}
          close={() => {}}
        />,
      ),
    );
    const grids = host.querySelectorAll(".lmsImageGrid");
    expect(grids).toHaveLength(1);
    expect(grids[0].querySelectorAll(".lmsImageCard")).toHaveLength(2);
  });

  it("shows files from an existing cache with string section IDs returned by Moodle", () => {
    const f = fixture();
    f.record.sections![0].id = "91" as unknown as number;
    act(() =>
      root.render(
        <LmsWorkspace
          {...f}
          entry={{ courseId: f.course.id, request: 501 }}
          active
          schoolWebAvailable={false}
          close={() => {}}
        />,
      ),
    );
    expect(
      host.querySelector(
        '.lmsSection[data-reading-section="section:91"] .lmsItemRow',
      ),
    ).not.toBeNull();
  });

  it("preserves empty/text-only sections and original order; searches full collapsed bodies", () => {
    const f = fixture();
    act(() =>
      root.render(
        <LmsWorkspace
          {...f}
          entry={{ courseId: f.course.id, request: 1, date: "2026-10-05" }}
          active
          schoolWebAvailable={false}
          close={() => {}}
        />,
      ),
    );
    expect(
      [...host.querySelectorAll(".lmsSection")].map((el) =>
        el.getAttribute("data-reading-section"),
      ),
    ).toEqual(["section:91", "section:81", "section:71"]);
    expect(host.querySelectorAll(".lmsSectionBody")).toHaveLength(3);
    expect(host.querySelector(".lmsViewer")).toBeNull();
    click(button("Collapse all"));
    expect(host.querySelector(".lmsSectionBody")).toBeNull();
    const search = host.querySelector("input")!;
    act(() => {
      Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value",
      )!.set!.call(search, "searchable tail");
      search.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(host.querySelectorAll(".lmsSection")).toHaveLength(1);
    expect(host.querySelector(".lmsSectionBody")?.textContent).toContain(
      "searchable tail",
    );
  });
  it("shows an assignment as submitted when Core says so, whatever field the LMS used", () => {
    const f = fixture();
    // LMS reported only `submissionStatus`; Core's TaskView owns the rule.
    f.data.assignments[0].submissionStatus = "submitted";
    core.tasks = {
      submittedIds: ["a1"],
      deadlines: {
        a1: {
          at: "2026-10-10T09:00:00+09:00",
          label: "Due 10 Oct, 09:00",
          tone: "normal",
          countdownLabel: "2d",
          countdownTone: "soon",
          overdue: false,
        },
      },
      nearestByCourse: {},
      byTerm: {},
    };
    act(() =>
      root.render(
        <LmsWorkspace
          {...f}
          entry={{ courseId: f.course.id, request: 3, assignmentId: "a1" }}
          active
          schoolWebAvailable={false}
          close={() => {}}
        />,
      ),
    );
    const card = host.querySelector(".lmsAssignmentCard")!;
    expect(card.className).toContain("done");
    expect(card.querySelector(".lmsAssignmentState")?.textContent).toBe(
      "Submitted",
    );
    expect(card.querySelector(".dueChip")).toBeNull();
  });
  it("opens exact assignment requirements first, and official LMS only after Submit", () => {
    const f = fixture();
    act(() =>
      root.render(
        <LmsWorkspace
          {...f}
          entry={{ courseId: f.course.id, request: 2, assignmentId: "a1" }}
          active
          schoolWebAvailable={false}
          close={() => {}}
        />,
      ),
    );
    expect(host.querySelector(".lmsAssignmentCard")?.textContent).toContain(
      "Requirement body",
    );
    expect(host.querySelector(".lmsViewer")).toBeNull();
    expect(host.querySelector(".lmsOfficialHost")).toBeNull();
    click(button("Submit"));
    expect(host.querySelector(".lmsOfficialHost")?.textContent).toContain(
      "Official LMS",
    );
    expect(host.querySelectorAll('[role="tab"]')).toHaveLength(0);
    expect(host.querySelectorAll(".lmsViewerClose")).toHaveLength(1);
    click(host.querySelector(".lmsViewerClose")!);
    expect(host.querySelector(".lmsOfficialHost")).toBeNull();
  });
  it("opens a resource in one click, reuses its tab, and restores its expansion after returning", () => {
    const f = fixture();
    const render = (request: number) =>
      act(() =>
        root.render(
          <LmsWorkspace
            {...f}
            entry={{ courseId: f.course.id, request }}
            active
            schoolWebAvailable={false}
            close={() => {}}
          />,
        ),
      );
    render(3);
    click(button("Lecture file"));
    click(button("Lecture file"));
    expect(host.querySelectorAll('[role="tab"]')).toHaveLength(0);
    expect(host.querySelectorAll(".lmsViewerClose")).toHaveLength(1);
    expect(host.querySelector(".lmsDocument")?.textContent).toContain(
      "Readable file body",
    );
    act(() => root.render(null));
    render(4);
    expect(
      host
        .querySelector(".lmsSection h3 button")
        ?.getAttribute("aria-expanded"),
    ).toBe("true");
    expect(host.querySelector(".lmsDocument")?.textContent).toContain(
      "Readable file body",
    );
  });
  it("restores the centered section-relative position after content height changes", () => {
    const f = fixture();
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      frames.push(callback);
      return frames.length;
    });
    vi.stubGlobal("cancelAnimationFrame", () => {});
    const render = (request: number) =>
      act(() =>
        root.render(
          <LmsWorkspace
            {...f}
            entry={{ courseId: f.course.id, request }}
            active
            schoolWebAvailable={false}
            close={() => {}}
          />,
        ),
      );
    const rect = (top: number, height: number) => ({
      top,
      bottom: top + height,
      height,
      left: 0,
      right: 300,
      width: 300,
      x: 0,
      y: top,
      toJSON() {},
    });
    render(10);
    const scroller = host.querySelector<HTMLDivElement>(".lmsOutlineScroll")!;
    vi.spyOn(scroller, "getBoundingClientRect").mockReturnValue(rect(0, 400));
    [...host.querySelectorAll<HTMLElement>(".lmsSection")].forEach(
      (section, index) =>
        vi
          .spyOn(section, "getBoundingClientRect")
          .mockReturnValue(
            rect(index ? 600 + index * 100 : -150, index ? 100 : 500),
          ),
    );
    scroller.scrollTop = 150;
    act(() => scroller.dispatchEvent(new Event("scroll", { bubbles: true })));
    act(() => root.render(null));
    render(11);
    const reopened = host.querySelector<HTMLDivElement>(".lmsOutlineScroll")!;
    vi.spyOn(reopened, "getBoundingClientRect").mockReturnValue(rect(0, 400));
    vi.spyOn(
      host.querySelector<HTMLElement>(".lmsSection")!,
      "getBoundingClientRect",
    ).mockReturnValue(rect(0, 1000));
    act(() => frames.at(-1)!(0));
    expect(reopened.scrollTop).toBe(500);
  });
  it("releases hidden file previews and bounds opened document tabs", async () => {
    const f = fixture();
    f.data.materials = Array.from({ length: 6 }, (_, index) => ({
      ...f.data.materials[0],
      id: `file-${index}`,
      title: `File ${index}`,
      bodyHtml: undefined,
      resourceUrl: `https://lms.s.isct.ac.jp/2025/pluginfile.php/42/file${index}.pdf`,
    }));
    const render = (active: boolean) =>
      act(() =>
        root.render(
          <LmsWorkspace
            {...f}
            entry={{ courseId: f.course.id, request: 20 }}
            active={active}
            schoolWebAvailable={false}
            close={() => {}}
          />,
        ),
      );
    render(true);
    for (let i = 0; i < 6; i++)
      click(
        [...host.querySelectorAll(".lmsItemRow")].find((b) =>
          b.textContent?.includes(`File ${i}`),
        )!,
      );
    await act(async () => {
      await vi.dynamicImportSettled();
    });
    expect(host.querySelectorAll('[role="tab"]')).toHaveLength(0);
    expect(host.querySelectorAll(".lmsViewerClose")).toHaveLength(1);
    expect(host.querySelectorAll("canvas")).toHaveLength(1);
    render(false);
    expect(host.querySelectorAll("canvas")).toHaveLength(0);
    render(true);
    expect(host.querySelectorAll("canvas")).toHaveLength(1);
  });
});

it("keeps Today and two recent course workspaces alive, evicting the oldest", () => {
  let mounted = 0;
  let disposed = 0;
  function RetainedCourse({ id, active }: { id: string; active: boolean }) {
    const [count, setCount] = React.useState(0);
    React.useEffect(() => {
      mounted++;
      return () => {
        disposed++;
      };
    }, []);
    return (
      <button
        data-course={id}
        data-active={active}
        onClick={() => setCount((value) => value + 1)}
      >
        {id}:{count}
      </button>
    );
  }
  const render = (id?: string) =>
    act(() =>
      root.render(
        <TodayWorkbench
          entry={id ? { courseId: id, request: Date.now() } : null}
          renderWorkspace={(entry, active) => (
            <RetainedCourse id={entry.courseId} active={active} />
          )}
        >
          {(compact) => (
            <input
              aria-label="Today retained state"
              data-compact={compact}
              defaultValue="Today"
            />
          )}
        </TodayWorkbench>,
      ),
    );
  render("A");
  click(host.querySelector('[data-course="A"]')!);
  const today = host.querySelector("input")!;
  today.value = "Retained";
  expect(
    host.querySelector(".todayRetained")?.classList.contains("compact"),
  ).toBe(true);
  render();
  expect(host.querySelector("input")).toBe(today);
  expect(
    host.querySelector(".todayRetained")?.classList.contains("compact"),
  ).toBe(false);
  expect(host.querySelector<HTMLInputElement>("input")!.value).toBe("Retained");
  render("A");
  expect(host.querySelector('[data-course="A"]')!.textContent).toBe("A:1");
  render("B");
  render("C");
  expect(host.querySelectorAll(".todayCourseSlot")).toHaveLength(2);
  expect(host.querySelector('[data-course="A"]')).toBeNull();
  expect(mounted).toBe(3);
  expect(disposed).toBe(1);
});

it("releases idle course DOMs after 45 seconds while keeping Today mounted", () => {
  vi.useFakeTimers();
  const render = (active: boolean) =>
    act(() =>
      root.render(
        <TodayWorkbench
          active={active}
          entry={{ courseId: "idle-course", request: 1 }}
          renderWorkspace={(entry) => (
            <div data-course={entry.courseId}>Reader</div>
          )}
        >
          {() => <input defaultValue="Today" />}
        </TodayWorkbench>,
      ),
    );
  render(true);
  const today = host.querySelector("input");
  expect(host.querySelectorAll("[data-course]")).toHaveLength(1);
  render(false);
  act(() => vi.advanceTimersByTime(45000));
  expect(host.querySelectorAll("[data-course]")).toHaveLength(0);
  expect(host.querySelector("input")).toBe(today);
  render(true);
  expect(host.querySelectorAll("[data-course]")).toHaveLength(1);
});

describe("inert course HTML", () => {
  it("keeps tables and valid links while removing active content and unsafe URLs", () => {
    const clean = safeContent(
      '<script>steal()</script><form>submit</form><p onclick="steal()">Text<a href="javascript:steal()">bad</a><a href="https://lms.s.isct.ac.jp/2025/">good</a></p><table><tr><td colspan="2">Date</td></tr></table>',
    );
    expect(clean).not.toMatch(/script|onclick|javascript|form|steal/);
    expect(clean).toContain('<td colspan="2">Date</td>');
    expect(clean).toContain('href="https://lms.s.isct.ac.jp/2025/"');
  });
  it("filters only complete paired labels and retains mixed continuation/code", () => {
    const paired =
      "<p>授業計画：NPDA<br>課題：状態遷移<br>Course schedule: NPDA<br>Objectives: transitions</p>";
    expect(safeContent(paired, "en")).not.toContain("授業計画");
    expect(safeContent(paired, "en")).toContain("Objectives");
    const mixed = paired.replace(
      "課題：状態遷移",
      "課題：状態遷移<br>Important unique notice",
    );
    expect(safeContent(mixed, "en")).toContain("授業計画");
    expect(
      safeContent(
        "<pre><code>授業計画：NPDA<br>Course schedule: NPDA</code></pre>",
        "en",
      ),
    ).toContain("授業計画");
    expect(
      safeContent('<p lang="ja">日本語</p><p lang="en">English</p>', "en"),
    ).toContain("English");
    expect(
      safeContent('<p lang="ja">日本語</p><p lang="en">English</p>', "en"),
    ).not.toContain("日本語");
  });
});

it("falls back to the available school language when the preferred blocks are absent", () => {
  expect(
    safeContent('<p lang="en">English-only announcement</p>', "ja"),
  ).toContain("English-only announcement");
  expect(safeContent('<p lang="ja">日本語のみ</p>', "en")).toContain(
    "日本語のみ",
  );
});
