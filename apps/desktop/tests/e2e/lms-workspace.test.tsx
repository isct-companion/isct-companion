// @vitest-environment jsdom
import { Activity, act, useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  advance,
  buttonByText,
  click,
  chooseOption,
  courseFor,
  emptySchoolRecords,
  installFakeCore,
  mount,
  mountApp,
  offering,
  overviewFor,
  scheduleViewFor,
  typeInto,
  until,
  workspaceFixture,
  type FakeCore,
  type FakeCoreOptions,
  type Mounted,
  type MountedApp,
} from "@tests/harness";
import type { Assignment, Course, Material, Workspace } from "@app/lib/model";
import type {
  LmsCourseRecord,
  LmsSection,
} from "@app/platform/desktop-bridge";
import type {
  CourseContent,
  LessonItemKind,
} from "@app/generated/persistence";
import type { CourseEntry } from "@app/features/lms/LmsWorkspace";

/**
 * The LMS reading workspace and the readers it opens, over the scripted host.
 *
 * The reading behaviours mount the real `LmsWorkspace` with the harness: the
 * legacy suite drove that component with props, and every guarantee here is
 * about what the workspace renders, not about which route reached it. The two
 * behaviours that are about the shell itself — an opened document surviving a
 * move to another lecture, and the Today workbench's retained readers — mount
 * the real shell or the real workbench.
 *
 * `pdfjs-dist` has no headless mode (it needs a worker and a canvas) and the
 * Milkdown/Crepe editor is driven through its real ProseMirror transactions, so
 * only `pdfjs-dist` is mocked; `PdfDocument` itself stays real.
 */

const pdfjs = vi.hoisted(() => ({
  load: vi.fn(),
  destroy: vi.fn(),
  cancel: vi.fn(),
  render: vi.fn(),
}));

vi.mock("pdfjs-dist/legacy/build/pdf.mjs", () => ({
  GlobalWorkerOptions: {},
  getDocument: pdfjs.load,
  TextLayer: class {
    render() {
      return Promise.resolve();
    }
    cancel() {}
  },
}));
vi.mock("pdfjs-dist/legacy/build/pdf.worker.min.mjs?url", () => ({
  default: "worker.mjs",
}));

const LIMIT = 20 * 1024 * 1024;
const CONTEXT = { term: "2026-3Q", course: "TEST", section: "section-91" };
const SOURCE = { provider: "lms" as const, externalId: "42" };

let app: MountedApp | undefined;
let views: Mounted[] = [];
let deviceCounter = 0;

afterEach(async () => {
  await app?.unmount().catch(() => undefined);
  app = undefined;
  for (const view of views.splice(0)) {
    await view.unmount().catch(() => undefined);
  }
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  localStorage.clear();
  sessionStorage.clear();
});

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
});


/**
 * Installs the scripted host, then loads the real module. `app/native` captures
 * its bridge at first import, so the fake host has to be in place first —
 * `mountApp` does the same thing for the shell.
 */
async function scriptedHost<T>(
  options: FakeCoreOptions,
  load: () => Promise<T>,
): Promise<{ core: FakeCore; real: T }> {
  const core = installFakeCore(options);
  // A component-level mount never goes through the shell, so the workspace
  // store would keep the first test's `contentRevision` — half of the
  // read-model cache key in `platform/core-query` — and hand a later test the
  // previous host's answer. Publishing this host's snapshot mirrors launch.
  const { workspaceStore } = await import("@app/store/workspace");
  await workspaceStore.reload();
  return { core, real: await load() };
}

/**
 * Waits generously: the gate runs every suite, and this file shares the machine
 * with sibling suites, so a 5s poll is not enough when the box is loaded.
 */
function waitFor<T>(
  check: () => T,
  options?: { timeout?: number },
): Promise<T> {
  return until(check, { timeout: options?.timeout ?? 15_000 });
}

async function release(view: Mounted): Promise<void> {
  const index = views.indexOf(view);
  if (index >= 0) views.splice(index, 1);
  await view.unmount();
}

function material(overrides: Partial<Material> = {}) {
  return {
    rev: 1,
    updatedAt: "2026-10-05T00:00:00Z",
    updatedBy: "test",
    source: SOURCE,
    id: "m1",
    kind: "material",
    courseId: "test-course",
    title: "Lecture file",
    type: "link",
    session: 9,
    sectionNumber: 9,
    sectionTitle: "Duplicate",
    bodyHtml: "<p>Readable file body</p>",
    module: "resource",
    ...overrides,
  } as Material;
}

function assignment(overrides: Partial<Assignment> = {}) {
  return {
    rev: 1,
    updatedAt: "2026-10-05T00:00:00Z",
    updatedBy: "test",
    source: SOURCE,
    id: "a1",
    kind: "assignment",
    courseId: "test-course",
    lmsCmid: 100,
    title: "Report",
    details: "Requirement body",
    dueAt: "2026-10-10T00:00:00Z",
    submission: "open",
    sourceUrl: "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=100",
    ...overrides,
  } as Assignment;
}

function courseRow(overrides: Partial<Course> = {}) {
  return courseFor(offering("202602421", "TEST", "Test course"), {
    id: "test-course",
    code: "TEST",
    title: "Test course",
    instructor: "",
    url: undefined,
    offeringId: undefined,
    source: SOURCE,
    meetings: [],
    ...overrides,
  } as Partial<Course>) as Course;
}

type LectureFixture = {
  workspace: Workspace;
  course: Course;
  record: LmsCourseRecord;
};

/** The legacy suite's course: three ordinary sections, one file, one report. */
function lectureFixture(): LectureFixture {
  const course = courseRow();
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
  const workspace = workspaceFixture({ courses: [course] });
  workspace.deviceId = `device:lms-${++deviceCounter}`;
  workspace.materials = [material()];
  workspace.assignments = [assignment()];
  return { workspace, course, record };
}

const KINDS: Record<string, LessonItemKind> = {
  resource: "file",
  url: "link",
  folder: "folder",
  page: "page",
  label: "label",
  forum: "forum",
};

/**
 * Core's `courseContent` answer for a fixture's records, as the scripted host
 * gives it: each LMS section with the materials and assignments set in it,
 * none of them lessons. Core's own rules (order, the syllabus link, rail
 * labels, schedule marks, pictures-only labels) are tested in
 * `course_content_follows_the_desktop_lecture_desk`; this suite checks what
 * the workspace renders of the answer.
 */
function contentFor(f: LectureFixture, imageOnly: string[] = []): CourseContent {
  const materials = f.workspace.materials.filter(
    (m) => m.courseId === f.course.id,
  );
  const inSection = (m: Material, section: LmsSection) =>
    m.sectionId != null
      ? Number(m.sectionId) === Number(section.id)
      : m.sectionNumber === section.number;
  const placed = (a: Assignment, section?: LmsSection) =>
    materials.some(
      (m) =>
        a.lmsCmid != null &&
        m.cmid === a.lmsCmid &&
        (!section || inSection(m, section)),
    );
  return {
    courseId: f.course.id,
    lessons: (f.record.sections ?? []).map((section) => {
      const items = materials.filter(
        (m) =>
          inSection(m, section) && !["assign", "quiz"].includes(m.module ?? ""),
      );
      return {
        section: section.number,
        sectionId: Number(section.id),
        title: section.title,
        number: null,
        label: section.title,
        summaryHtml: section.bodyHtml ?? "",
        summary: "",
        items: items.map((m) => ({
          materialId: m.id,
          kind: KINDS[m.module ?? "resource"] ?? "other",
          title: m.title,
          bodyHtml: ["page", "label", "forum"].includes(m.module ?? "")
            ? (m.bodyHtml ?? null)
            : null,
          source: m.url ?? m.resourceUrl ?? null,
          imageOnly: imageOnly.includes(m.id),
        })),
        assignmentIds: f.workspace.assignments
          .filter((a) => placed(a, section))
          .map((a) => a.id),
        posted: items.length > 0,
      };
    }),
    otherAssignmentIds: f.workspace.assignments
      .filter((a) => !placed(a))
      .map((a) => a.id),
    currentSection: null,
    classAt: null,
  };
}

/**
 * The fixture's answer with Core's lesson rail on it: rail labels and lesson
 * numbers by section id, where the course is and the class a mark is for.
 */
function lessonContent(
  f: LectureFixture,
  rail: Record<number, [label: string, number: number]>,
  currentSection: number,
  classAt: string | null = null,
): CourseContent {
  const content = contentFor(f);
  for (const lesson of content.lessons) {
    const [label, number] = rail[lesson.sectionId!] ?? [lesson.label, null];
    Object.assign(lesson, { label, number });
  }
  return { ...content, currentSection, classAt };
}

type WorkspaceOptions = {
  entry?: Partial<CourseEntry>;
  active?: boolean;
  markLesson?: (sectionId: number, occurrence: string, label: string) => void;
  /** Core's `courseContent` answer; by default the fixture's (`contentFor`). */
  content?: CourseContent;
  tasks?: unknown;
  /** Core's `courseDetail` answer, which carries the course's syllabus link. */
  detail?: unknown;
  handle?: FakeCoreOptions["handle"];
};

/** Mounts the real reading workspace over the scripted host. */
async function openWorkspace(
  fixture: LectureFixture,
  options: WorkspaceOptions = {},
) {
  const { core, real } = await scriptedHost(
    {
      workspace: fixture.workspace,
      schoolRecords: {
        ...emptySchoolRecords(),
        lmsCourses: [fixture.record],
      },
      queries: {
        courseContent: () => options.content ?? contentFor(fixture),
        taskView: () => options.tasks ?? null,
        courseDetail: () => options.detail ?? null,
      },
      handle: (command, args) => {
        if (command === "list_course_notes") return [];
        // The harness also offers the fake store; this suite's handlers ignore it.
        const custom = options.handle as
          | ((command: string, args: Record<string, unknown>) => unknown)
          | undefined;
        return custom?.(command, args) ?? undefined;
      },
    },
    // The shell renders the in-app confirmations beside every page; load them
    // with the workspace so both share one confirmation store.
    async () => ({
      ...(await import("@app/features/lms/LmsWorkspace")),
      ...(await import("@app/components/confirm")),
    }),
  );
  const props = {
    course: fixture.course,
    data: fixture.workspace,
    entry: { courseId: fixture.course.id, request: 1, ...options.entry },
    active: options.active ?? true,
    schoolWebAvailable: false,
    close: () => {},
    markLesson: options.markLesson,
  };
  const view = await mount(
    <>
      <real.LmsWorkspaceBody {...props} />
      <real.ConfirmDialogs />
    </>,
  );
  views.push(view);
  await waitFor(() =>
    expect(view.host.querySelector(".lmsWorkspace")).not.toBeNull(),
  );
  await advance(25);
  return {
    core,
    view,
    host: view.host,
    render: async (overrides: Partial<typeof props> = {}) => {
      await act(async () =>
        view.root.render(
          <>
            <real.LmsWorkspaceBody {...props} {...overrides} />
            <real.ConfirmDialogs />
          </>,
        ),
      );
    },
  };
}

function scheduleQueryNow(core: FakeCore): string | undefined {
  for (const call of [...core.commands("schedule_query")].reverse()) {
    const queries = call.args.queries as
      { kind?: string; now?: string }[] | undefined;
    const found = queries?.find((query) => query.kind === "courseContent");
    if (found) return found.now;
  }
  return undefined;
}

function itemRow(host: HTMLElement, text: string): HTMLElement {
  const row = [...host.querySelectorAll<HTMLElement>(".lmsItemRow")].find(
    (item) => item.textContent?.includes(text),
  );
  if (!row) throw new Error(`no outline row for ${text}`);
  return row;
}

const lectureRoute = (courseId: string) =>
  `/lectures/${encodeURIComponent(courseId)}`;

describe("the LMS reading workspace", () => {
  it("restores a saved reader when the workspace pane starts narrow", async () => {
    const fixture = lectureFixture();
    sessionStorage.setItem(
      `lms-reading:${fixture.workspace.deviceId}:${fixture.course.id}`,
      JSON.stringify({
        version: 3,
        expanded: [],
        scroll: 0,
        opened: { id: "m1", title: "Lecture file", kind: "material" },
      }),
    );
    // jsdom has no layout; report the narrow width the outline and viewer
    // switch at, as a Windows snap or a side panel leaves the pane.
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      callback(0);
      return 1;
    });
    vi.stubGlobal(
      "ResizeObserver",
      class implements ResizeObserver {
        constructor(private callback: ResizeObserverCallback) {}
        observe(target: Element) {
          if (!target.matches(".lmsWorkspaceBody")) return;
          queueMicrotask(() =>
            this.callback(
              [
                {
                  target,
                  contentRect: new DOMRect(0, 0, 600, 700),
                  borderBoxSize: [{ inlineSize: 600, blockSize: 700 }],
                } as unknown as ResizeObserverEntry,
              ],
              this,
            ),
          );
        }
        unobserve() {}
        disconnect() {}
      },
    );

    const { host } = await openWorkspace(fixture);
    const workspace = () => host.querySelector(".lmsWorkspace");
    await waitFor(() =>
      expect(workspace()?.classList.contains("narrowReader")).toBe(true),
    );
    expect(workspace()?.classList.contains("viewing")).toBe(true);
    expect(host.querySelector(".lmsViewerBody")?.textContent).toContain(
      "Readable file body",
    );
  }, 30_000);

  it("lays out every lesson as a page and opens the outline where the course is", async () => {
    const f = lectureFixture();
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
        title: "演習第1回",
        visible: true,
        bodyHtml: "<p>授業計画: 演習</p>",
      },
    ];
    f.workspace.materials = [11, 12].map((sectionId) =>
      material({
        id: `m${sectionId}`,
        title: `File ${sectionId}`,
        sectionId,
        sectionNumber: sectionId - 10,
        sectionTitle: `第${sectionId - 10}回`,
        bodyHtml: undefined,
      }),
    );
    const { host } = await openWorkspace(f, {
      entry: { request: 97 },
      content: lessonContent(
        f,
        { 11: ["1", 1], 12: ["2", 2], 13: ["演習 1", 1] },
        2,
      ),
    });

    const rail = () => [
      ...host.querySelectorAll<HTMLElement>(
        ".lmsLessonRail button:not(.lmsRailTab)",
      ),
    ];
    // Core's labels, as they are: a recitation keeps its own word.
    await waitFor(() =>
      expect(rail().map((button) => button.textContent)).toEqual([
        "1",
        "2",
        "演習 1",
      ]),
    );
    // The outline opens where Core puts the course.
    expect(rail()[1].getAttribute("aria-current")).toBe("step");
    // Every lesson is laid out at once, each as a page of the outline.
    expect(
      [...host.querySelectorAll(".lmsLessonPage")].map((page) =>
        page.getAttribute("data-page"),
      ),
    ).toEqual(["11", "12", "13"]);
    expect(host.querySelector('[data-page="11"]')?.textContent).toContain(
      "File 11",
    );
    expect(host.querySelector('[data-page="12"]')?.textContent).toContain(
      "File 12",
    );
    // 一般 has no number, and the report belongs to no lesson: both are tabs.
    expect(
      [...host.querySelectorAll(".lmsRailTab")].map((tab) => tab.textContent),
    ).toEqual(["一般", "Activities"]);

    await click(rail()[0]);
    expect(rail()[0].getAttribute("aria-current")).toBe("step");
  }, 30_000);

  it("searches the full collapsed body of a lesson and collapses every section", async () => {
    const f = lectureFixture();
    const { host } = await openWorkspace(f, {
      entry: { request: 1, date: "2026-10-05" },
    });

    expect(
      [...host.querySelectorAll(".lmsSection")].map((section) =>
        section.getAttribute("data-reading-section"),
      ),
    ).toEqual(["section:91", "section:81", "section:71"]);
    expect(host.querySelectorAll(".lmsSectionBody")).toHaveLength(3);
    expect(host.querySelector(".lmsViewer")).toBeNull();

    await click(buttonByText(host, "Collapse all"));
    expect(host.querySelector(".lmsSectionBody")).toBeNull();

    const search = host.querySelector<HTMLInputElement>("input")!;
    await typeInto(search, "searchable tail");
    await waitFor(() =>
      expect(host.querySelectorAll(".lmsSection")).toHaveLength(1),
    );
    expect(host.querySelector(".lmsSectionBody")?.textContent).toContain(
      "searchable tail",
    );
  }, 30_000);

  it("opens a folder in the outline from the files already stored with it", async () => {
    const f = lectureFixture();
    Object.assign(f.workspace.materials[0], {
      title: "講義資料",
      module: "folder",
      url: "https://lms.s.isct.ac.jp/2025/mod/folder/view.php?id=9",
      bodyHtml:
        '<ul><li><a href="https://lms.s.isct.ac.jp/2025/pluginfile.php/1/mod_folder/content/0/20260928.zip?forcedownload=1">20260928.zip</a></li></ul>',
    });
    const { host } = await openWorkspace(f, { entry: { request: 96 } });

    const row = host.querySelector<HTMLButtonElement>(
      '.lmsFolder > [aria-expanded="false"]',
    )!;
    await click(row);
    await waitFor(() =>
      expect(host.querySelector(".lmsFolderFile")?.textContent).toContain(
        "20260928.zip",
      ),
    );
    expect(host.querySelector(".lmsViewer")).toBeNull();
  }, 30_000);

  it("puts consecutive image-only labels in one grid", async () => {
    const f = lectureFixture();
    const label = (id: string, name: string) =>
      material({
        id,
        title: name,
        module: "label",
        resourceUrl: undefined,
        bodyHtml: `<div><p><a href="https://lms.s.isct.ac.jp/2025/pluginfile.php/1/mod_label/intro/${name}.png"><img src="https://lms.s.isct.ac.jp/2025/pluginfile.php/1/mod_label/intro/s_${name}.png" alt=""></a></p></div>`,
      });
    f.workspace.materials = [
      label("l1", "schedule"),
      label("l2", "discord"),
      ...f.workspace.materials,
    ];
    const { host } = await openWorkspace(f, {
      entry: { request: 602 },
      content: contentFor(f, ["l1", "l2"]),
    });

    const grids = host.querySelectorAll(".lmsImageGrid");
    expect(grids).toHaveLength(1);
    expect(grids[0].querySelectorAll(".lmsImageCard")).toHaveLength(2);
  }, 30_000);

  it("shows a label's own text without the name Moodle derives from it, keeping Core's schedule marks", async () => {
    const f = lectureFixture();
    Object.assign(f.workspace.materials[0], {
      module: "label",
      title: "日付 授業回目 内容…",
      bodyHtml:
        '<table><tr><td>日付</td><td>授業回目</td><td>内容</td></tr><tr class="lmsRowNow"><td>10/20</td><td>2回目</td><td>行列</td></tr></table>',
    });
    const { host } = await openWorkspace(f, { entry: { request: 98 } });

    const label = host.querySelector(".lmsSectionBody .lmsInlineActivity")!;
    expect(label.querySelector("table")).not.toBeNull();
    expect(label.querySelector("h4")).toBeNull();
    // The workspace's own sanitising keeps the row Core marked as current.
    expect(label.querySelector("tr.lmsRowNow")?.textContent).toContain("2回目");
  }, 30_000);

  it("renders a page activity inline without opening a document panel", async () => {
    const f = lectureFixture();
    f.workspace.materials[0].module = "page";
    const { host } = await openWorkspace(f, { entry: { request: 99 } });

    expect(
      host.querySelector(".lmsSectionBody .lmsInlineActivity")?.textContent,
    ).toContain("Readable file body");
    expect(host.querySelector(".lmsViewer")).toBeNull();
  }, 30_000);

  it("filters bilingual school text by the content language, and shows the original on request", async () => {
    const f = lectureFixture();
    f.workspace.settings.locale = "en";
    f.workspace.settings.contentLanguage = "ja";
    f.workspace.settings.lmsBilingualFilter = true;
    f.record.sections![0].bodyHtml =
      '<p lang="en">English summary</p><p lang="ja">日本語の概要</p>';
    const { host } = await openWorkspace(f, { entry: { request: 700 } });

    const body = () => host.querySelector(".lmsSectionBody")?.textContent ?? "";
    expect(body()).toContain("日本語の概要");
    expect(body()).not.toContain("English summary");

    await click(buttonByText(host, "Show original"));
    await waitFor(() =>
      expect(host.querySelector(".lmsSectionBody")?.textContent).toContain(
        "English summary",
      ),
    );
  }, 30_000);

  it("filters only complete paired labels and keeps mixed continuation and code", async () => {
    const paired =
      "<p>授業計画：NPDA<br>課題：状態遷移<br>Course schedule: NPDA<br>Objectives: transitions</p>";
    const mixed = paired.replace(
      "課題：状態遷移",
      "課題：状態遷移<br>Important unique notice",
    );
    const code =
      "<pre><code>授業計画：NPDA<br>Course schedule: NPDA</code></pre>";

    const english = async (bodyHtml: string) => {
      const f = lectureFixture();
      f.workspace.settings.locale = "en";
      f.workspace.settings.contentLanguage = "en";
      f.workspace.settings.lmsBilingualFilter = true;
      f.record.sections![0].bodyHtml = bodyHtml;
      const { host } = await openWorkspace(f, { entry: { request: 701 } });
      return host.querySelector(".lmsSectionBody")?.textContent ?? "";
    };

    const filtered = await english(paired);
    expect(filtered).not.toContain("授業計画");
    expect(filtered).toContain("Objectives");

    const withContinuation = await english(mixed);
    expect(withContinuation).toContain("授業計画");
    expect(withContinuation).toContain("Important unique notice");

    const withCode = await english(code);
    expect(withCode).toContain("授業計画");
  }, 60_000);

  it("falls back to the school language that is present when the preferred blocks are absent", async () => {
    const only = async (bodyHtml: string, contentLanguage: "ja" | "en") => {
      const f = lectureFixture();
      f.workspace.settings.locale = "en";
      f.workspace.settings.contentLanguage = contentLanguage;
      f.workspace.settings.lmsBilingualFilter = true;
      f.record.sections![0].bodyHtml = bodyHtml;
      const { host } = await openWorkspace(f, { entry: { request: 702 } });
      return host.querySelector(".lmsSectionBody")?.textContent ?? "";
    };

    expect(
      await only('<p lang="en">English-only announcement</p>', "ja"),
    ).toContain("English-only announcement");
    expect(await only('<p lang="ja">日本語のみ</p>', "en")).toContain(
      "日本語のみ",
    );
  }, 60_000);

  it("opens an announcement in the reader without expanding its body in the outline", async () => {
    const f = lectureFixture();
    f.workspace.settings.locale = "en";
    Object.assign(f.workspace.materials[0], {
      module: "forum",
      bodyHtml:
        '<article id="announcement-p42" data-source-url="https://lms.s.isct.ac.jp/2025/mod/forum/discuss.php?d=42"><h5>Classroom change</h5><time>1 October</time><p>Please meet in W3-707. Bring your notes.</p></article>',
    });
    const { host } = await openWorkspace(f, { entry: { request: 600 } });

    expect(
      host
        .querySelector(".lmsOutline")
        ?.contains(host.querySelector('[aria-label="Search course content"]')),
    ).toBe(true);
    expect(
      host.querySelector(".lmsWorkspaceTools")?.querySelector("input"),
    ).toBeNull();
    expect(host.querySelector(".lmsViewer")).toBeNull();
    const card = host.querySelector<HTMLElement>(".lmsAnnouncementCard")!;
    expect(card.textContent).toContain("Classroom change");
    await click(card);
    await waitFor(() =>
      expect(
        host.querySelector(".lmsViewer .lmsRichContent")?.textContent,
      ).toContain("Bring your notes."),
    );
    expect(
      host.querySelector(".lmsViewerClose")?.getAttribute("aria-label"),
    ).toContain("Classroom change");
  }, 30_000);

  it("shows an announcement forum without posts as empty, with its official page", async () => {
    const f = lectureFixture();
    f.workspace.settings.locale = "en";
    Object.assign(f.workspace.materials[0], {
      title: "アナウンスメント",
      module: "forum",
      bodyHtml: "",
      url: "https://lms.s.isct.ac.jp/2025/mod/forum/view.php?id=7",
    });
    const { host, core } = await openWorkspace(f, { entry: { request: 610 } });

    const forum = [...host.querySelectorAll(".lmsInlineActivity")].find(
      (item) => item.textContent?.includes("アナウンスメント"),
    )!;
    expect(forum.textContent).toContain("No announcements yet.");
    expect(host.querySelector(".lmsViewer")).toBeNull();

    await click(buttonByText(host, "Open in LMS"));
    // The reader's tool row carries the page's address, as a browser's does.
    await waitFor(() =>
      expect(host.querySelector(".lmsWebAddress")?.textContent).toContain(
        "lms.s.isct.ac.jp",
      ),
    );
    await act(async () =>
      core.emit("school-web-url", {
        site: "science-lms",
        url: "https://docs.google.com/forms/d/test/viewform",
      }),
    );
    await waitFor(() =>
      expect(host.querySelector(".lmsWebAddress span")?.textContent).toBe(
        "docs.google.com",
      ),
    );
    expect(
      host
        .querySelector('.lmsWebToolbar a[aria-label="Open in browser"]')
        ?.getAttribute("href"),
    ).toBe("https://docs.google.com/forms/d/test/viewform");
  }, 30_000);

  it("opens the syllabus in its own panel", async () => {
    const f = lectureFixture();
    const { host } = await openWorkspace(f, {
      entry: { request: 502 },
      detail: {
        syllabusUrl: "https://syllabus.s.isct.ac.jp/courses/-/-/-/202600001",
      },
    });

    await click(
      [...host.querySelectorAll("button")].find(
        (button) => button.textContent?.trim() === "Syllabus",
      )!,
    );
    await waitFor(() =>
      expect(
        host.querySelector(".lmsViewerClose")?.getAttribute("aria-label"),
      ).toContain("Syllabus"),
    );
  }, 30_000);

  it("keeps an unpreviewable attachment in the outline as a popover card", async () => {
    const f = lectureFixture();
    const url =
      "https://lms.s.isct.ac.jp/2025/pluginfile.php/42/mod_assign/introattachment/0/report.zip?forcedownload=1";
    f.record.sections![0].bodyHtml = `<p><a href="${url}">Report ZIP</a></p>`;
    const popup = vi.spyOn(window, "open").mockReturnValue(null);
    const { host } = await openWorkspace(f, { entry: { request: 601 } });

    await click(
      [...host.querySelectorAll("a")].find(
        (link) =>
          link.querySelector(".lmsAttachmentName")?.textContent ===
          "Report ZIP",
      )!,
    );
    await waitFor(() =>
      expect(
        host.querySelector(".lmsOutline .lmsFilePopover .lmsFileCard h3")
          ?.textContent,
      ).toBe("Report ZIP"),
    );
    expect(popup).not.toHaveBeenCalled();
    expect(host.querySelector(".lmsViewer")).toBeNull();

    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    await waitFor(() =>
      expect(host.querySelector(".lmsFilePopover")).toBeNull(),
    );
  }, 30_000);

  it("marks the lesson a coming class covers when its rail entry is double-clicked", async () => {
    const f = lectureFixture();
    f.record.sections = [1, 2, 3].map((number) => ({
      id: 10 + number,
      number,
      title: `第${number}回`,
      visible: true,
    }));
    f.workspace.materials = [];
    const markLesson = vi.fn();
    const { host } = await openWorkspace(f, {
      entry: { request: 95 },
      markLesson,
      content: lessonContent(
        f,
        { 11: ["1", 1], 12: ["2", 2], 13: ["3", 3] },
        2,
        "2026-10-08T10:45:00+09:00",
      ),
    });

    await waitFor(() =>
      expect(
        host.querySelectorAll(".lmsLessonRail button:not(.lmsRailTab)"),
      ).toHaveLength(3),
    );
    const rail = [
      ...host.querySelectorAll<HTMLElement>(
        ".lmsLessonRail button:not(.lmsRailTab)",
      ),
    ];
    // Core puts the course at 第2回: it and the lessons before it are held.
    expect(rail[1].getAttribute("aria-current")).toBe("step");
    expect(rail.map((button) => button.classList.contains("held"))).toEqual([
      true,
      true,
      false,
    ]);

    await act(async () => {
      rail[2].dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    });
    expect(markLesson).toHaveBeenCalledWith(
      13,
      "2026-10-08T10:45:00+09:00",
      "Lesson 3",
    );
  }, 30_000);

  it("shows an assignment as submitted when Core's task view says so, whatever the LMS field", async () => {
    const f = lectureFixture();
    // The LMS reported only `submissionStatus`; Core's TaskView owns the rule.
    f.workspace.assignments[0].submissionStatus = "submitted";
    const { host } = await openWorkspace(f, {
      entry: { request: 3, assignmentId: "a1" },
      tasks: {
        submittedIds: ["a1"],
        deadlines: {
          a1: {
            at: "2026-10-10T09:00:00+09:00",
            label: "Due 10 Oct, 09:00",
            tone: "normal",
            remainingMilliseconds: 172800000,
            countdownLabel: "2d",
            countdownTone: "soon",
            overdue: false,
          },
        },
        nearestByCourse: {},
        byTerm: {},
      },
    });

    const card = host.querySelector(".lmsAssignmentCard")!;
    expect(card.className).toContain("done");
    expect(card.querySelector(".lmsAssignmentState")?.textContent).toBe(
      "Submitted",
    );
    expect(card.querySelector(".dueChip")).toBeNull();
  }, 30_000);

  it("opens exact assignment requirements first, and the official LMS only after View in LMS", async () => {
    const f = lectureFixture();
    const { host, render } = await openWorkspace(f, {
      entry: { request: 2, assignmentId: "a1" },
    });

    expect(host.querySelector(".lmsAssignmentCard")?.textContent).toContain(
      "Requirement body",
    );
    expect(host.querySelector(".lmsViewer")).toBeNull();
    expect(host.querySelector(".lmsOfficialHost")).toBeNull();

    // Without a workspace there is no draft to prepare.
    await click(buttonByText(host, "View in LMS"));
    await waitFor(() =>
      expect(host.querySelector(".lmsOfficialHost")?.textContent).toContain(
        "opens the signed-in LMS here",
      ),
    );
    expect(host.querySelectorAll('[role="tab"]')).toHaveLength(0);
    expect(host.querySelectorAll(".lmsViewerClose")).toHaveLength(1);
    // A retained page hides and returns with the page open; the viewer must
    // render the same hooks either way (React #311 on the first return).
    await render({ active: false });
    await render({ active: true });
    expect(host.querySelector(".lmsOfficialHost")?.textContent).toContain(
      "opens the signed-in LMS here",
    );

    await click(host.querySelector(".lmsViewerClose"));
    await waitFor(() =>
      expect(host.querySelector(".lmsOfficialHost")).toBeNull(),
    );
  }, 30_000);

  it.each(["ready", "failed", "missing"] as const)(
    "prepares workspace files in the right LMS pane, and stops if preparation fails (%s)",
    async (outcome) => {
      const f = lectureFixture();
      f.workspace.assignments = [assignment()];
      const workspace = {
        path: "/courses/Report",
        name: "Report",
        submitFiles: ["report.pdf"],
        zip: false,
        zipName: "Report.zip",
      };
      let present = true;
      const url =
        "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=100&action=editsubmission";
      const { host, core } = await openWorkspace(f, {
        handle: (command) => {
          if (command === "assignment_workspace")
            return present
              ? { workspace, handIn: "prepareDraft" }
              : { workspace: null, handIn: "openLms" };
          if (command === "prepare_workspace_submission") {
            if (outcome === "failed")
              return Promise.reject({
                code: "hand-in-empty",
                detail: "The submit folder is empty",
              });
            if (outcome === "missing") {
              // Deleted in Finder after the card was shown.
              present = false;
              return Promise.reject({
                code: "workspace-folder-missing",
                detail: "The workspace folder no longer exists",
              });
            }
            return { workspace, url };
          }
          return undefined;
        },
      });
      await waitFor(() =>
        expect(host.querySelector(".lmsWorkspaceCard")).not.toBeNull(),
      );
      await click(buttonByText(host, "Prepare in LMS"));
      expect(
        core.commands("prepare_workspace_submission")[0].args,
      ).toMatchObject({
        path: workspace.path,
        assignment: "a1",
        url: f.workspace.assignments[0].sourceUrl,
      });
      if (outcome === "failed") {
        expect(host.textContent).toContain("The submit folder is empty.");
        expect(host.querySelector(".lmsViewer")).toBeNull();
      } else if (outcome === "missing") {
        // The card gives way to a notice; the page still opens.
        await waitFor(() =>
          expect(host.querySelector(".lmsWorkspaceCard")).toBeNull(),
        );
        expect(host.textContent).toContain(
          "The workspace folder “Report” is gone.",
        );
        expect(host.querySelector(".lmsWorkspaceError")).toBeNull();
        await click(buttonByText(host, "View in LMS"));
        await waitFor(() =>
          expect(
            host.querySelector(".lmsOfficialHost a")?.getAttribute("href"),
          ).toBe(f.workspace.assignments[0].sourceUrl),
        );
      } else {
        await waitFor(() =>
          expect(
            host.querySelector(".lmsOfficialHost a")?.getAttribute("href"),
          ).toBe(url),
        );
        expect(host.querySelectorAll(".lmsViewer")).toHaveLength(1);
      }
    },
  );

  it("deletes a workspace only after the in-app confirmation", async () => {
    // The shell's window.confirm is asynchronous, so a `!confirm()` guard
    // never stopped the delete; the in-app dialog has to answer first.
    const f = lectureFixture();
    f.workspace.assignments = [assignment()];
    const workspace = {
      path: "/courses/Report",
      name: "Report",
      submitFiles: [],
      zip: false,
      zipName: "Report.zip",
    };
    const { host, core } = await openWorkspace(f, {
      handle: (command) =>
        command === "assignment_workspace"
          ? { workspace, handIn: "prepareDraft" }
          : undefined,
    });
    const trash = await waitFor(() => {
      const button = host.querySelector<HTMLButtonElement>(
        '[aria-label="Delete workspace"]',
      );
      if (!button) throw new Error("The workspace card has no delete button.");
      return button;
    });
    const dialog = () => document.querySelector<HTMLElement>('[role="dialog"]');

    await click(trash);
    await waitFor(() =>
      expect(dialog()?.textContent).toContain(
        "Delete the workspace “Report” and everything in it?",
      ),
    );
    await click(buttonByText(dialog()!, "Cancel"));
    await waitFor(() => expect(dialog()).toBeNull());
    expect(core.commands("delete_workspace")).toHaveLength(0);

    await click(trash);
    await waitFor(() => expect(dialog()).not.toBeNull());
    await click(buttonByText(dialog()!, "Delete"));
    await waitFor(() =>
      expect(core.commands("delete_workspace")[0]?.args).toMatchObject({
        path: workspace.path,
      }),
    );
  }, 30_000);

  it("renders course HTML inert: tables and safe links stay, active content and unsafe URLs do not", async () => {
    const f = lectureFixture();
    f.record.sections![0].bodyHtml =
      '<script>steal()</script><form>submit</form><p onclick="steal()">Text<a href="javascript:steal()">bad</a><a href="https://lms.s.isct.ac.jp/2025/">good</a></p><table><tr><td colspan="2">Date</td></tr></table>';
    const { host } = await openWorkspace(f, { entry: { request: 603 } });

    const body = host.querySelector(
      '.lmsSection[data-reading-section="section:91"] .lmsSectionBody',
    )!;
    expect(body.textContent).toContain("Text");
    expect(body.querySelector("script")).toBeNull();
    expect(body.querySelector("form")).toBeNull();
    expect(body.querySelector("[onclick]")).toBeNull();
    expect(body.innerHTML).not.toContain("javascript:");
    expect(body.querySelector("td")?.getAttribute("colspan")).toBe("2");
    expect(
      [...body.querySelectorAll("a")].map((link) => link.getAttribute("href")),
    ).toContain("https://lms.s.isct.ac.jp/2025/");
  }, 30_000);

  it("replaces the current document, pins PDF tools above it, and releases hidden readers", async () => {
    const f = lectureFixture();
    pdfjs.load.mockReset();
    pdfjs.destroy.mockReset();
    pdfjs.cancel.mockReset();
    pdfjs.render.mockReset();
    pdfjs.render.mockReturnValue({
      promise: Promise.resolve(),
      cancel: pdfjs.cancel,
    });
    pdfjs.load.mockReturnValue({
      promise: Promise.resolve({
        numPages: 1,
        getPage: vi.fn(async () => ({
          getViewport: ({ scale }: { scale: number }) => ({
            width: 600 * scale,
            height: 800 * scale,
          }),
          render: pdfjs.render,
          cleanup: vi.fn(),
          streamTextContent: () => ({}),
        })),
      }),
      destroy: pdfjs.destroy,
    });
    f.workspace.materials = Array.from({ length: 6 }, (_, index) =>
      material({
        id: `file-${index}`,
        title: `File ${index}`,
        bodyHtml: undefined,
        resourceUrl: `https://lms.s.isct.ac.jp/2025/pluginfile.php/42/file${index}.pdf`,
      }),
    );
    const { host, view, render } = await openWorkspace(f, {
      entry: { request: 20 },
      handle: (command, args) =>
        command === "open_course_material"
          ? {
              state: "ready",
              file: {
                id: "f",
                name: "file.pdf",
                section: "section-0",
                path: `/course/${String(args.url).split("/").pop()}`,
                bytes: 10,
                cached: true,
              },
            }
          : undefined,
    });

    for (let index = 0; index < 6; index += 1) {
      await click(itemRow(host, `File ${index}`));
      await waitFor(() =>
        expect(host.querySelectorAll("canvas")).toHaveLength(1),
      );
    }
    expect(host.querySelectorAll(".lmsViewerClose")).toHaveLength(1);
    expect(host.querySelectorAll('[role="tab"]')).toHaveLength(0);
    expect(
      host.querySelector('.lmsReaderToolSlot [aria-label="PDF tools"]'),
    ).not.toBeNull();
    expect(
      host.querySelector('.lmsViewerBody [aria-label="PDF tools"]'),
    ).toBeNull();
    expect(
      host.querySelector(".lmsViewerClose")?.getAttribute("aria-label"),
    ).toBe("Close File 5");

    await render({ active: false });
    await waitFor(() =>
      expect(host.querySelectorAll("canvas")).toHaveLength(0),
    );
    await render({ active: true });
    await waitFor(() =>
      expect(host.querySelectorAll("canvas")).toHaveLength(1),
    );

    await click(host.querySelector(".lmsViewerClose"));
    await waitFor(() =>
      expect(host.querySelectorAll("canvas")).toHaveLength(0),
    );
    expect(host.querySelector(".lmsViewer")).toBeNull();

    await release(view);
    const stored = JSON.parse(
      sessionStorage.getItem(
        `lms-reading:${f.workspace.deviceId}:${f.course.id}`,
      ) ?? "null",
    );
    expect(stored.opened).toBeUndefined();
    expect(stored).not.toHaveProperty("tabs");
  }, 60_000);

  it("restores the centered section-relative reading position after content height changes", async () => {
    const f = lectureFixture();
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      frames.push(callback);
      return frames.length;
    });
    vi.stubGlobal("cancelAnimationFrame", () => {});

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

    const first = await openWorkspace(f, { entry: { request: 10 } });
    const scroller =
      first.host.querySelector<HTMLDivElement>(".lmsOutlineScroll")!;
    vi.spyOn(scroller, "getBoundingClientRect").mockReturnValue(rect(0, 400));
    [...first.host.querySelectorAll<HTMLElement>(".lmsSection")].forEach(
      (section, index) =>
        vi
          .spyOn(section, "getBoundingClientRect")
          .mockReturnValue(
            rect(index ? 600 + index * 100 : -150, index ? 100 : 500),
          ),
    );
    scroller.scrollTop = 150;
    await act(async () => {
      scroller.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    await release(first.view);

    const second = await openWorkspace(f, { entry: { request: 11 } });
    const reopened =
      second.host.querySelector<HTMLDivElement>(".lmsOutlineScroll")!;
    vi.spyOn(reopened, "getBoundingClientRect").mockReturnValue(rect(0, 400));
    vi.spyOn(
      second.host.querySelector<HTMLElement>(".lmsSection")!,
      "getBoundingClientRect",
    ).mockReturnValue(rect(0, 1000));
    await act(async () => {
      frames.at(-1)!(0);
    });
    expect(reopened.scrollTop).toBe(500);
  }, 30_000);

  it("refreshes the course content when the retained reader is resumed, and keeps ticking while active", async () => {
    const f = lectureFixture();
    const { core, real } = await scriptedHost(
      {
        workspace: f.workspace,
        schoolRecords: {
          ...emptySchoolRecords(),
          lmsCourses: [f.record],
        },
        handle: (command) => (command === "list_course_notes" ? [] : undefined),
      },
      () => import("@app/features/lms/LmsWorkspace"),
    );
    vi.useFakeTimers({ now: new Date("2026-10-08T00:00:00Z") });

    const node = (active: boolean) => (
      <real.LmsWorkspaceBody
        course={f.course}
        data={f.workspace}
        entry={{ courseId: f.course.id, request: 1 }}
        active={active}
        schoolWebAvailable={false}
        close={() => {}}
      />
    );
    const view = await mount(node(false));
    views.push(view);
    await waitFor(() =>
      expect(scheduleQueryNow(core)).toBe("2026-10-08T00:00:00.000Z"),
    );

    // A hidden reader is idle: two minutes pass without another query.
    await act(async () => {
      vi.advanceTimersByTime(120_000);
    });
    expect(scheduleQueryNow(core)).toBe("2026-10-08T00:00:00.000Z");

    await act(async () => {
      view.root.render(node(true));
    });
    await waitFor(() =>
      expect(scheduleQueryNow(core)).toBe("2026-10-08T00:02:00.000Z"),
    );

    await act(async () => {
      vi.advanceTimersByTime(60_000);
    });
    await waitFor(() =>
      expect(scheduleQueryNow(core)).toBe("2026-10-08T00:03:00.000Z"),
    );
  }, 30_000);
});

describe("the reading workspace on its lecture route", () => {
  it("keeps an opened document when the reader moves to another lecture and back", async () => {
    const f = lectureFixture();
    const other = courseRow({
      id: "other-course",
      code: "OTHER",
      title: "Other course",
      source: { provider: "lms", externalId: "99" },
    });
    f.workspace.courses.push(other);

    app = await mountApp(
      {
        workspace: f.workspace,
        schoolRecords: {
          ...emptySchoolRecords(),
          lmsCourses: [f.record],
        },
        queries: {
          workspaceOverview: (_query, core) => overviewFor(core.workspace),
          scheduleView: (_query, core) => scheduleViewFor(core.workspace),
          courseContent: (query) =>
            (query as { courseId: string }).courseId === f.course.id
              ? contentFor(f)
              : null,
        },
        handle: (command) => (command === "list_course_notes" ? [] : undefined),
      },
      { route: lectureRoute(f.course.id) },
    );
    const { host } = app;

    await waitFor(() =>
      expect(host.querySelector(".lmsWorkspace")).not.toBeNull(),
    );
    const row = itemRow(host, "Lecture file");
    await click(row);
    await click(row);
    await waitFor(() =>
      expect(host.querySelector(".lmsDocument")?.textContent).toContain(
        "Readable file body",
      ),
    );
    expect(host.querySelectorAll(".lmsViewerClose")).toHaveLength(1);
    expect(host.querySelectorAll('[role="tab"]')).toHaveLength(0);

    await app.go(lectureRoute(other.id));
    await waitFor(() =>
      expect(host.querySelectorAll(".lmsWorkspace")).toHaveLength(2),
    );
    // The hidden reader released its document, but kept the open tab.
    await waitFor(() =>
      expect(host.querySelectorAll(".lmsDocument")).toHaveLength(0),
    );
    expect(host.querySelectorAll(".lmsViewerClose")).toHaveLength(1);

    await app.go(lectureRoute(f.course.id));
    await waitFor(() =>
      expect(host.querySelector(".lmsDocument")?.textContent).toContain(
        "Readable file body",
      ),
    );
    expect(host.querySelectorAll(".lmsViewerClose")).toHaveLength(1);
  }, 40_000);
});

/** A course workspace stand-in that keeps local state, to prove it stays mounted. */
function RetainedCourse({ id, active }: { id: string; active: boolean }) {
  const [count, setCount] = useState(0);
  return (
    <button
      data-course={id}
      data-active={String(active)}
      onClick={() => setCount((value) => value + 1)}
    >
      {id}:{count}
    </button>
  );
}

describe("the Today workbench's retained course workspaces", () => {
  async function mountWorkbench() {
    const { real } = await scriptedHost(
      { workspace: workspaceFixture() },
      () => import("@app/features/lms/TodayWorkbench"),
    );
    return real.TodayWorkbench;
  }

  it("keeps Today and two recent course workspaces, evicting the oldest", async () => {
    const TodayWorkbench = await mountWorkbench();
    const view = await mount(
      <TodayWorkbench
        entry={{ courseId: "A", request: 1 }}
        renderWorkspace={(entry, active) => (
          <RetainedCourse id={entry.courseId} active={active} />
        )}
      >
        {(compact) => (
          <input
            aria-label="Today retained state"
            data-compact={String(compact)}
            defaultValue="Today"
          />
        )}
      </TodayWorkbench>,
    );
    views.push(view);
    const { host } = view;
    const render = async (entry: CourseEntry | null) => {
      await act(async () =>
        view.root.render(
          <TodayWorkbench
            entry={entry}
            renderWorkspace={(item, active) => (
              <RetainedCourse id={item.courseId} active={active} />
            )}
          >
            {(compact) => (
              <input
                aria-label="Today retained state"
                data-compact={String(compact)}
                defaultValue="Today"
              />
            )}
          </TodayWorkbench>,
        ),
      );
    };

    await click(host.querySelector('[data-course="A"]'));
    const today = host.querySelector("input")!;
    today.value = "Retained";
    expect(
      host.querySelector(".todayRetained")?.classList.contains("compact"),
    ).toBe(true);

    await render(null);
    expect(host.querySelector("input")).toBe(today);
    expect(
      host.querySelector(".todayRetained")?.classList.contains("compact"),
    ).toBe(false);
    expect(host.querySelector<HTMLInputElement>("input")!.value).toBe(
      "Retained",
    );

    await render({ courseId: "A", request: 2 });
    // The workspace was retained, not remounted: its local count survived.
    expect(host.querySelector('[data-course="A"]')!.textContent).toBe("A:1");

    await render({ courseId: "B", request: 3 });
    await render({ courseId: "C", request: 4 });
    expect(host.querySelectorAll(".todayCourseSlot")).toHaveLength(2);
    expect(host.querySelector('[data-course="A"]')).toBeNull();
  }, 30_000);

  it("releases idle course DOMs after 45 seconds while keeping Today mounted", async () => {
    vi.useFakeTimers();
    const TodayWorkbench = await mountWorkbench();
    const view = await mount(
      <TodayWorkbench
        active
        entry={{ courseId: "idle-course", request: 1 }}
        renderWorkspace={(entry) => (
          <div data-course={entry.courseId}>Reader</div>
        )}
      >
        {() => <input defaultValue="Today" />}
      </TodayWorkbench>,
    );
    views.push(view);
    const { host } = view;

    expect(host.querySelectorAll("[data-course]")).toHaveLength(1);
    const today = host.querySelector("input");
    await act(async () => {
      view.root.render(
        <TodayWorkbench
          active={false}
          entry={{ courseId: "idle-course", request: 1 }}
          renderWorkspace={(entry) => (
            <div data-course={entry.courseId}>Reader</div>
          )}
        >
          {() => <input defaultValue="Today" />}
        </TodayWorkbench>,
      );
    });
    await act(async () => {
      vi.advanceTimersByTime(45_000);
    });
    expect(host.querySelectorAll("[data-course]")).toHaveLength(0);
    expect(host.querySelector("input")).toBe(today);

    await act(async () => {
      view.root.render(
        <TodayWorkbench
          active
          entry={{ courseId: "idle-course", request: 1 }}
          renderWorkspace={(entry) => (
            <div data-course={entry.courseId}>Reader</div>
          )}
        >
          {() => <input defaultValue="Today" />}
        </TodayWorkbench>,
      );
    });
    expect(host.querySelectorAll("[data-course]")).toHaveLength(1);
  }, 30_000);
});

describe("the file preview pipeline", () => {
  async function openPreview(
    url: string,
    handle: (command: string, args: Record<string, unknown>) => unknown,
    props: { title?: string; openInApp?: () => void } = {},
  ) {
    const { core, real } = await scriptedHost(
      {
        workspace: workspaceFixture(),
        handle: (command, args) => {
          if (command === "list_course_notes") return [];
          return handle?.(command, args) ?? undefined;
        },
      },
      () => import("@app/features/lms/FilePreview"),
    );
    const view = await mount(
      <real.FilePreview
        url={url}
        title={props.title ?? "Notes"}
        context={CONTEXT}
        limit={LIMIT}
        openInApp={props.openInApp}
      />,
    );
    views.push(view);
    return { core, view, host: view.host };
  }

  it("prepares the file through the host and previews the local copy", async () => {
    const { core, host } = await openPreview(
      "https://lms.s.isct.ac.jp/2025/pluginfile.php/1/a.txt",
      (command) =>
        command === "open_course_material"
          ? {
              state: "ready",
              file: {
                id: "f1",
                name: "a.txt",
                section: "section-91",
                path: "/course/a.txt",
                bytes: 14,
                cached: true,
                text: "Lecture notes",
              },
            }
          : undefined,
      { title: "Notes" },
    );

    await waitFor(() => expect(host.textContent).toContain("Lecture notes"));
    expect(core.commands("open_course_material")[0].args).toMatchObject({
      context: CONTEXT,
      url: expect.stringContaining("a.txt"),
      download: true,
      readText: true,
      limit: LIMIT,
    });

    await click(buttonByText(host, "Show in folder"));
    await waitFor(() =>
      expect(core.commands("open_course_file")).toHaveLength(1),
    );
    expect(core.commands("open_course_material").at(-1)?.args).toMatchObject({
      download: true,
    });
    expect(core.commands("open_course_file")[0].args).toMatchObject({
      path: "/course/a.txt",
      reveal: true,
    });
  }, 30_000);

  it("renders a previewable image from the local file URL", async () => {
    const { host } = await openPreview(
      "https://lms.s.isct.ac.jp/2025/pluginfile.php/1/pic.png",
      (command) =>
        command === "open_course_material"
          ? {
              state: "ready",
              file: {
                id: "f1",
                name: "pic.png",
                section: "section-91",
                path: "/course/pic.png",
                bytes: 10,
                cached: true,
              },
            }
          : undefined,
      { title: "Picture" },
    );

    await waitFor(() =>
      expect(host.querySelector("img.lmsImagePreview")).not.toBeNull(),
    );
    expect(
      host.querySelector("img.lmsImagePreview")?.getAttribute("src"),
    ).toContain("/course/pic.png");
  }, 30_000);

  it("shows an oversized file's size and previews it explicitly on request", async () => {
    let attempts = 0;
    const { core, host } = await openPreview(
      "https://lms.s.isct.ac.jp/2025/pluginfile.php/1/big.txt",
      (command) => {
        if (command !== "open_course_material") return undefined;
        attempts += 1;
        return attempts === 1
          ? { state: "tooLarge", bytes: 45 * 1024 * 1024 }
          : {
              state: "ready",
              file: {
                id: "f1",
                name: "big.txt",
                section: "section-91",
                path: "/course/big.txt",
                bytes: 9,
                cached: true,
                text: "Chapter 1",
              },
            };
      },
      { title: "Big" },
    );

    await waitFor(() => expect(host.textContent).toContain("45 MB"));
    await click(buttonByText(host, "Preview · 45 MB"));
    await waitFor(() => expect(host.textContent).toContain("Chapter 1"));
    expect(
      core.commands("open_course_material").at(-1)?.args.limit,
    ).toBeUndefined();
  }, 30_000);

  it("falls back to the official LMS in the reader when a file cannot be prepared", async () => {
    const f = lectureFixture();
    f.workspace.materials = [
      material({
        bodyHtml: undefined,
        resourceUrl:
          "https://lms.s.isct.ac.jp/2025/pluginfile.php/42/mod_resource/content/1/slides.pdf",
      }),
    ];
    const { host } = await openWorkspace(f, {
      entry: { request: 604 },
      handle: (command) =>
        command === "open_course_material"
          ? { state: "failed", reason: "network" }
          : undefined,
    });

    await click(itemRow(host, "Lecture file"));
    await waitFor(() => expect(buttonByText(host, "Open in LMS")).toBeTruthy());
    await click(buttonByText(host, "Open in LMS"));
    await waitFor(() =>
      expect(host.querySelector(".lmsOfficialHost")?.textContent).toContain(
        "opens the signed-in LMS here",
      ),
    );
  }, 30_000);

  it.each([
    ["login", "Sign in to LMS"],
    ["unresolved", "Open in LMS"],
    ["network", "Open in LMS"],
  ] as const)(
    "cannot prepare a file (%s) and offers “%s” to open it in the external viewer",
    async (reason, label) => {
      const openInApp = vi.fn();
      const { host } = await openPreview(
        "https://lms.s.isct.ac.jp/2025/mod/resource/view.php?id=42",
        (command) =>
          command === "open_course_material"
            ? { state: "failed", reason }
            : undefined,
        { title: "File", openInApp },
      );

      await waitFor(() => expect(buttonByText(host, label)).toBeTruthy());
      expect(host.querySelector('[role="alert"]')?.textContent).toContain(
        reason,
      );
      await click(buttonByText(host, label));
      expect(openInApp).toHaveBeenCalledTimes(1);
    },
    30_000,
  );
});

describe("the note editor", () => {
  async function openNote(
    options: {
      save?: (attempt: number) => unknown;
      onDeleted?: () => void;
    } = {},
  ) {
    let attempts = 0;
    const { core, real } = await scriptedHost(
      {
        workspace: workspaceFixture(),
        handle: (command) => {
          if (command === "read_course_note")
            return {
              id: "n1",
              section: "section-91",
              name: "Note",
              path: "note",
              bytes: 7,
              cached: true,
              text: "initial",
            };
          if (command === "save_course_note") {
            attempts += 1;
            return options.save
              ? options.save(attempts)
              : {
                  id: "n1",
                  section: "section-91",
                  name: "Note",
                  path: "note",
                  bytes: 7,
                  cached: false,
                };
          }
          if (command === "move_course_note") return null;
          return undefined;
        },
      },
      () => import("@app/features/lms/NoteEditor"),
    );

    // The editor the note really renders: drive its ProseMirror transactions.
    const { Editor, editorViewCtx } = await import("@milkdown/kit/core");
    const editors: InstanceType<typeof Editor>[] = [];
    const make = Editor.make.bind(Editor);
    vi.spyOn(Editor, "make").mockImplementation(() => {
      const editor = make();
      editors.push(editor);
      return editor;
    });

    const element = (mode: "visible" | "hidden") => (
      <Activity mode={mode}>
        <real.NoteEditor
          context={CONTEXT}
          id="n1"
          onSaved={() => {}}
          onDeleted={options.onDeleted}
        />
      </Activity>
    );
    const view = await mount(element("visible"));
    views.push(view);
    await waitFor(
      () => expect(view.host.querySelector(".ProseMirror")).not.toBeNull(),
      { timeout: 15_000 },
    );
    await waitFor(() => expect(editors.length).toBeGreaterThan(0));

    return {
      core,
      view,
      host: view.host,
      unmount: () => release(view),
      reload: async (mode: "visible" | "hidden") => {
        await act(async () => view.root.render(element(mode)));
      },
      edit: async (text: string) => {
        await act(async () => {
          editors.at(-1)!.action((ctx) => {
            const prose = ctx.get(editorViewCtx);
            prose.dispatch(
              prose.state.tr.insertText(text, prose.state.doc.content.size),
            );
          });
        });
      },
    };
  }

  it("flushes the current draft when the editor closes before autosave", async () => {
    const note = await openNote();
    await note.edit(" and more");
    await note.unmount();

    await waitFor(() =>
      expect(note.core.commands("save_course_note")).toHaveLength(1),
    );
    const save = note.core.commands("save_course_note")[0];
    expect(save.args).toMatchObject({ context: CONTEXT, id: "n1" });
    expect(String(save.args.text)).toContain("and more");
  }, 40_000);

  it("keeps a failed save as a recovery draft and clears it after a successful retry", async () => {
    const note = await openNote({
      save: (attempt) =>
        attempt === 1
          ? Promise.reject(new Error("Disk unavailable"))
          : {
              id: "n1",
              section: "section-91",
              name: "Note",
              path: "note",
              bytes: 7,
              cached: false,
            },
    });
    await note.edit(" keep me");
    await click(buttonByText(note.host, "Save"));
    await waitFor(() =>
      expect(note.host.textContent).toContain("Disk unavailable"),
    );

    const recovery = JSON.parse(
      localStorage.getItem("lms-note-recovery:n1") ?? "null",
    );
    expect(recovery.text).toContain("keep me");

    await click(buttonByText(note.host, "Retry save"));
    await waitFor(() =>
      expect(note.host.textContent).toContain("Saved locally"),
    );
    expect(localStorage.getItem("lms-note-recovery:n1")).toBeNull();
  }, 40_000);

  it("saves the final draft before moving the note to the deleted store", async () => {
    const deleted = vi.fn();
    const note = await openNote({ onDeleted: deleted });
    await note.edit(" last draft");
    await click(buttonByText(note.host, "Delete note"));
    await waitFor(() => expect(deleted).toHaveBeenCalled());

    const saves = note.core.commands("save_course_note");
    const moves = note.core.commands("move_course_note");
    expect(saves).toHaveLength(1);
    expect(String(saves[0].args.text)).toContain("last draft");
    expect(moves[0].args).toMatchObject({ context: CONTEXT, id: "n1" });
    expect(note.core.calls.indexOf(saves[0])).toBeLessThan(
      note.core.calls.indexOf(moves[0]),
    );

    await note.unmount();
    expect(note.core.commands("save_course_note")).toHaveLength(1);
  }, 40_000);

  it("keeps the live draft across an Activity hide and does not read the note again", async () => {
    const note = await openNote();
    await note.edit(" before hiding");
    await note.reload("hidden");

    await waitFor(() =>
      expect(note.core.commands("save_course_note").length).toBeGreaterThan(0),
    );
    expect(
      String(note.core.commands("save_course_note").at(-1)?.args.text),
    ).toContain("before hiding");

    await note.reload("visible");
    await waitFor(
      () => expect(note.host.querySelector(".ProseMirror")).not.toBeNull(),
      { timeout: 15_000 },
    );
    expect(note.host.querySelector(".ProseMirror")?.textContent).toContain(
      "before hiding",
    );
    expect(note.core.commands("read_course_note")).toHaveLength(1);
  }, 40_000);
});

describe("the PDF viewer", () => {
  async function openPdf() {
    const { real } = await scriptedHost(
      { workspace: workspaceFixture() },
      () => import("@app/features/lms/PdfDocument"),
    );
    const view = await mount(
      <>
        <real.PdfDocument src="asset://localhost/a.pdf" title="Lecture" />
      </>,
    );
    views.push(view);
    return { view, host: view.host };
  }

  it("releases the document worker and cancels its active page when the viewer leaves", async () => {
    pdfjs.load.mockReset();
    pdfjs.destroy.mockReset();
    pdfjs.cancel.mockReset();
    pdfjs.render.mockReset();
    const sheet = {
      getViewport: ({ scale }: { scale: number }) => ({
        width: 600 * scale,
        height: 800 * scale,
      }),
      render: pdfjs.render,
      cleanup: vi.fn(),
    };
    pdfjs.render.mockReturnValue({
      promise: new Promise(() => {}),
      cancel: pdfjs.cancel,
    });
    pdfjs.load.mockReturnValue({
      promise: Promise.resolve({
        numPages: 20,
        getPage: vi.fn().mockResolvedValue(sheet),
      }),
      destroy: pdfjs.destroy,
    });

    const { view, host } = await openPdf();
    // Only the first page renders, off-screen until it is complete.
    await waitFor(() => expect(pdfjs.render).toHaveBeenCalled());
    expect(host.querySelectorAll("canvas")).toHaveLength(0);

    await release(view);
    expect(pdfjs.cancel).toHaveBeenCalledTimes(1);
    expect(pdfjs.destroy).toHaveBeenCalledTimes(1);
  }, 30_000);

  it("finds text in a later page and zooms without mounting every canvas", async () => {
    pdfjs.load.mockReset();
    pdfjs.destroy.mockReset();
    pdfjs.cancel.mockReset();
    pdfjs.render.mockReset();
    pdfjs.render.mockReturnValue({
      promise: Promise.resolve(),
      cancel: pdfjs.cancel,
    });
    pdfjs.load.mockReturnValue({
      promise: Promise.resolve({
        numPages: 3,
        getPage: vi.fn(async (number: number) => ({
          getViewport: ({ scale }: { scale: number }) => ({
            width: 600 * scale,
            height: 800 * scale,
          }),
          render: pdfjs.render,
          cleanup: vi.fn(),
          streamTextContent: () => ({
            getReader: () => {
              let done = false;
              return {
                read: async () =>
                  done
                    ? { done: true }
                    : ((done = true),
                      {
                        done: false,
                        value: {
                          items: [
                            {
                              str:
                                number === 3
                                  ? "Java object inheritance"
                                  : "Introduction",
                            },
                          ],
                        },
                      }),
                releaseLock() {},
                cancel: async () => {},
              };
            },
          }),
        })),
      }),
      destroy: pdfjs.destroy,
    });
    const scroll = vi
      .spyOn(HTMLElement.prototype, "scrollIntoView")
      .mockImplementation(() => {});

    const { view, host } = await openPdf();
    expect(host.querySelector("form")).toBeNull();
    await click(host.querySelector<HTMLButtonElement>(".lmsPdfFindToggle"));

    const search = host.querySelector<HTMLInputElement>(
      'input[aria-label="Find in document"]',
    )!;
    await typeInto(search, "inheritance");
    await act(async () => {
      host
        .querySelector("form")!
        .dispatchEvent(
          new Event("submit", { bubbles: true, cancelable: true }),
        );
    });
    await waitFor(() =>
      expect(
        host.querySelector<HTMLInputElement>('input[aria-label="Page"]')!.value,
      ).toBe("3"),
    );
    expect(scroll).toHaveBeenCalled();
    expect(host.querySelectorAll("canvas")).toHaveLength(1);

    const zoom = host.querySelector<HTMLInputElement>(
      '[role="combobox"][aria-label="Zoom"]',
    )!;
    await chooseOption(zoom, "1.5");
    expect(
      (host.querySelector(".lmsPdfPages") as HTMLElement).style.width,
    ).toBe("150%");
    expect(host.querySelectorAll("canvas")).toHaveLength(1);

    vi.useFakeTimers();
    const viewport = host.querySelector(".lmsPdfDocument")!;
    const gesture = new WheelEvent("wheel", {
      ctrlKey: true,
      deltaY: -20,
      bubbles: true,
      cancelable: true,
    });
    await act(async () => {
      viewport.dispatchEvent(gesture);
    });
    expect(gesture.defaultPrevented).toBe(true);
    expect(
      (host.querySelector(".lmsPdfPages") as HTMLElement).style.transform,
    ).toContain("scale");
    await act(async () => {
      vi.advanceTimersByTime(150);
    });
    expect(
      parseFloat(
        (host.querySelector(".lmsPdfPages") as HTMLElement).style.width,
      ),
    ).toBeGreaterThan(150);
    expect(
      (host.querySelector(".lmsPdfPages") as HTMLElement).style.transform,
    ).toBe("");

    await release(view);
  }, 30_000);

  it("sizes every page from its own viewport in a mixed portrait and landscape deck", async () => {
    pdfjs.load.mockReset();
    pdfjs.destroy.mockReset();
    pdfjs.cancel.mockReset();
    pdfjs.render.mockReset();
    pdfjs.render.mockReturnValue({
      promise: Promise.resolve(),
      cancel: pdfjs.cancel,
    });
    const sizes = [
      [600, 800],
      [800, 450],
      [600, 800],
    ];
    pdfjs.load.mockReturnValue({
      promise: Promise.resolve({
        numPages: 3,
        getPage: vi.fn(async (number: number) => ({
          getViewport: ({ scale }: { scale: number }) => ({
            width: sizes[number - 1][0] * scale,
            height: sizes[number - 1][1] * scale,
          }),
          render: pdfjs.render,
          cleanup: vi.fn(),
          streamTextContent: () => ({}),
        })),
      }),
      destroy: pdfjs.destroy,
    });

    const { view, host } = await openPdf();
    // jsdom does not expose `aspectRatio` on CSSStyleDeclaration; read the
    // serialized style the way a person would inspect the page.
    const aspects = () =>
      [...host.querySelectorAll<HTMLElement>("[data-pdf-page]")].map((page) =>
        Number(
          page.getAttribute("style")?.match(/aspect-ratio:\s*([\d.]+)/)?.[1],
        ).toFixed(3),
      );
    await waitFor(() => expect(aspects()).toEqual(["0.750", "1.778", "0.750"]));
    // The rendered first page has been swapped in.
    expect(host.querySelectorAll("canvas")).toHaveLength(1);

    await release(view);
  }, 30_000);
});

describe("the markdown editor", () => {
  async function openMarkdown(
    value: string,
    onChange?: (value: string) => void,
  ) {
    await scriptedHost(
      { workspace: workspaceFixture() },
      async () => undefined,
    );
    const { Editor, editorViewCtx } = await import("@milkdown/kit/core");
    const { MarkdownDocument } =
      await import("@app/features/lms/MarkdownDocument");
    const editors: InstanceType<typeof Editor>[] = [];
    const make = Editor.make.bind(Editor);
    vi.spyOn(Editor, "make").mockImplementation(() => {
      const editor = make();
      editors.push(editor);
      return editor;
    });
    const view = await mount(
      <MarkdownDocument value={value} onChange={onChange} />,
    );
    views.push(view);
    await waitFor(
      () => expect(view.host.querySelector(".ProseMirror")).not.toBeNull(),
      { timeout: 15_000 },
    );
    await waitFor(() => expect(editors.length).toBeGreaterThan(0));
    return {
      view,
      host: view.host,
      editors,
      rerender: async (next: string, onChange?: (value: string) => void) => {
        await act(async () =>
          view.root.render(
            <MarkdownDocument value={next} onChange={onChange} />,
          ),
        );
      },
      editable: () =>
        editors.at(-1)!.action((ctx) => ctx.get(editorViewCtx).editable),
      edit: async (text: string) => {
        await act(async () => {
          editors.at(-1)!.action((ctx) => {
            const prose = ctx.get(editorViewCtx);
            prose.dispatch(
              prose.state.tr.insertText(text, prose.state.doc.content.size),
            );
          });
        });
      },
    };
  }

  it("keeps formulas, lists and code through an edit, without saving on load", async () => {
    const source =
      "# Notes\n\nInline $E=mc^2$\n\n$$\nx^2+y^2=z^2\n$$\n\n- First\n- Second\n\n```js\nconst a = 1;\n```\n";
    const change = vi.fn();
    const editor = await openMarkdown(source, change);

    // Loading a saved document is not an edit.
    expect(change).not.toHaveBeenCalled();
    expect(editor.host.querySelector(".ProseMirror")?.textContent).toContain(
      "Notes",
    );
    expect(editor.host.querySelectorAll(".ProseMirror li")).toHaveLength(2);

    await editor.edit(" updated");
    await waitFor(() => expect(change).toHaveBeenCalledTimes(1));
    const saved = change.mock.calls[0][0] as string;
    expect(saved).toContain("updated");
    expect(saved).toContain("$E=mc^2$");
    expect(saved).toContain("x^2+y^2=z^2");
    expect(saved).toContain("```js");
    expect(saved).toContain("Second");

    const prose = editor.host.querySelector(".ProseMirror");
    await editor.rerender(saved, change);
    // The same editor takes the controlled value back; no extra edit is emitted.
    expect(editor.host.querySelector(".ProseMirror")).toBe(prose);
    expect(change).toHaveBeenCalledTimes(1);
  }, 40_000);

  it("applies an external value to a live editor without emitting an edit", async () => {
    const change = vi.fn();
    const editor = await openMarkdown("# Attachment\n\nRead me", change);
    await editor.rerender("# Replacement", change);
    await waitFor(() =>
      expect(editor.host.textContent).toContain("Replacement"),
    );
    expect(change).not.toHaveBeenCalled();
  }, 40_000);

  it("formats the selected text from the Markdown tool panel", async () => {
    const change = vi.fn();
    const editor = await openMarkdown("Plain", change);
    const { editorViewCtx } = await import("@milkdown/kit/core");
    const { TextSelection } = await import("@milkdown/kit/prose/state");
    await act(async () =>
      editor.editors.at(-1)!.action((ctx) => {
        const view = ctx.get(editorViewCtx);
        view.dispatch(
          view.state.tr.setSelection(
            TextSelection.create(view.state.doc, 1, 6),
          ),
        );
      }),
    );
    await click(editor.host.querySelector('[aria-label="Bold"]'));
    await waitFor(() =>
      expect(change.mock.calls.at(-1)?.[0]).toContain("**Plain**"),
    );
  }, 40_000);

  it("renders an attachment read-only", async () => {
    const editor = await openMarkdown("# Attachment\n\nRead me");
    expect(editor.editable()).toBe(false);
    expect(
      editor.host.querySelector('[aria-label="Markdown tools"]'),
    ).toBeNull();
    expect(
      editor.host
        .querySelector(".ProseMirror")
        ?.getAttribute("contenteditable"),
    ).toBe("false");
  }, 40_000);
});
