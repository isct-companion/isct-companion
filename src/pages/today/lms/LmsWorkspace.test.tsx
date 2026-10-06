// @vitest-environment jsdom
import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { seedWorkspace, type Course } from "../../../core/model";
import type { LmsCourseRecord } from "../../../platform/desktop-bridge";
import { LmsWorkspace } from "./LmsWorkspace";
import { safeContent } from "./content";
import { TodayWorkbench } from "./TodayWorkbench";

vi.mock("./PdfDocument", () => ({ PdfDocument: () => <canvas aria-label="PDF page" /> }));

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement;
let root: Root;
let fixtureIndex = 0;
beforeEach(() => {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  sessionStorage.clear();
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

  it("opens an English announcement card in the right reader without expanding its full body on the left", () => {
    const f = fixture();
    f.data.settings.locale = "en";
    f.data.materials[0].module = "forum";
    f.data.materials[0].bodyHtml = '<article id="announcement-p42" data-source-url="https://lms.s.isct.ac.jp/2025/mod/forum/discuss.php?d=42"><h5>Classroom change</h5><time>1 October</time><p>Please meet in W3-707. Bring your notes.</p></article>';
    act(() => root.render(<LmsWorkspace {...f} entry={{courseId:f.course.id,request:600}} active schoolWebAvailable={false} close={()=>{}} />));
    expect(host.querySelector(".lmsViewer")).toBeNull();
    const card = host.querySelector(".lmsAnnouncementCard")!;
    expect(card.textContent).toContain("Announcement");
    expect(card.textContent).toContain("Classroom change");
    click(card);
    expect(host.querySelector(".lmsViewer .lmsRichContent")?.textContent).toContain("Bring your notes.");
    expect(host.querySelector('.lmsViewerClose')?.getAttribute("aria-label")).toContain("Classroom change");
  });

  it("hides the redundant syllabus entry and opens its dedicated right panel", () => {
    const f = fixture();
    f.course.offeringId = "20260001";
    f.data.materials.push({ ...f.data.materials[0], id: "syllabus-item", title: "シラバス", module: "url" });
    act(() => root.render(<LmsWorkspace {...f} entry={{courseId: f.course.id, request: 502}} active schoolWebAvailable={false} close={() => {}} />));
    expect([...host.querySelectorAll(".lmsItemRow")].some(row => row.textContent?.includes("シラバス"))).toBe(false);
    click([...host.querySelectorAll("button")].find(button => button.textContent?.includes("Open in syllabus"))!);
    expect(host.querySelector('.lmsViewerClose')?.getAttribute("aria-label")).toContain("Syllabus");
  });

  it("keeps unpreviewable attachments in the outline as a popover card", async () => {
    const f = fixture();
    const url = "https://lms.s.isct.ac.jp/2025/pluginfile.php/42/mod_assign/introattachment/0/report.zip?forcedownload=1";
    f.record.sections![0].bodyHtml = `<p><a href="${url}">Report ZIP</a></p>`;
    const popup = vi.spyOn(window, "open").mockReturnValue(null);
    act(() => root.render(<LmsWorkspace {...f} entry={{courseId:f.course.id,request:601}} active schoolWebAvailable={false} close={()=>{}} />));
    click([...host.querySelectorAll("a")].find(link => link.querySelector(".lmsAttachmentName")?.textContent === "Report ZIP")!);
    await act(async () => { await vi.dynamicImportSettled(); });
    expect(popup).not.toHaveBeenCalled();
    expect(host.querySelector(".lmsViewer")).toBeNull();
    expect(host.querySelector(".lmsOutline .lmsFilePopover .lmsFileCard h3")?.textContent).toBe("Report ZIP");
    act(() => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
    expect(host.querySelector(".lmsFilePopover")).toBeNull();
  });

  it("puts consecutive image-only labels in one grid", () => {
    const f = fixture();
    const label = (id: string, name: string) => ({ ...f.data.materials[0], id, title: name, module: "label", resourceUrl: undefined,
      bodyHtml: `<div><p><a href="https://lms.s.isct.ac.jp/2025/pluginfile.php/1/mod_label/intro/${name}.png"><img src="https://lms.s.isct.ac.jp/2025/pluginfile.php/1/mod_label/intro/s_${name}.png" alt=""></a></p></div>` });
    f.data.materials = [label("l1", "schedule"), label("l2", "discord"), ...f.data.materials];
    act(() => root.render(<LmsWorkspace {...f} entry={{courseId:f.course.id,request:602}} active schoolWebAvailable={false} close={()=>{}} />));
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
    expect(host.querySelector(".lmsInlineActivity")?.textContent).toContain(
      "Requirement body",
    );
    expect(host.querySelector(".lmsViewer")).toBeNull();
    expect(host.querySelector(".lmsOfficialHost")).toBeNull();
    click(button("Submit in LMS"));
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
