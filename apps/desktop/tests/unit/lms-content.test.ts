// @vitest-environment jsdom
/**
 * The LMS reading surface, grouped by behaviour: how a course file is
 * classified, how its address is rewritten for the desktop shell, how notes
 * survive an autosave, how lessons are numbered and marked on the schedule
 * table, what an assignment workspace is told, how the outline settles, and
 * what the Matrix scanner sends to the native recognizer.
 *
 * jsdom is needed for three real things: `DOMParser` (lesson summaries,
 * schedule tables, assignment briefs), the `window` the scripted Tauri host
 * installs itself on, and the `File`/`Blob`-based OCR input path.
 */
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import {
  installFakeCore,
  workspaceFixture,
  type FakeCore,
  type FakeCoreOptions,
} from "@tests/harness";
import {
  extension,
  fileFamily,
  previewKind,
} from "@app/features/lms/file-types";
import { textLanguage } from "@app/features/lms/TextDocument";
import {
  courseLessons,
  frontierLesson,
  gravityTarget,
  lessonNumber,
  lessonSummary,
  markedLesson,
  markSchedule,
  railLabels,
} from "@app/features/lms/lessons";
import { springStep, stiffnessFor } from "@app/features/lms/spring-scroll";
import {
  MATRIX_SIZE,
  matrixConfidence,
  normalizeMatrixText,
  parseMatrixText,
  prepareMatrixImage,
  recognizeMatrixImage,
} from "@app/lib/matrix-ocr";

type FilesModule = typeof import("@app/features/lms/files");
type AssignmentsModule =
  typeof import("@app/features/lms/assignment-workspace");

/**
 * `app/native.ts` captures `getDesktopBridge()` when it is first imported, so
 * the scripted host has to exist before the modules that reach it. Everything
 * above is bridge-free; these two are imported after `beforeAll` installs it.
 * The bridge delegates to whichever fake is installed at call time, so each
 * test still gets a clean, recorded host.
 */
let files: FilesModule;
let assignments: AssignmentsModule;

beforeAll(async () => {
  installFakeCore({ workspace: workspaceFixture() });
  [files, assignments] = await Promise.all([
    import("@app/features/lms/files"),
    import("@app/features/lms/assignment-workspace"),
  ]);
});

const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

/** The platform fact `@tauri-apps/api/core`'s `isTauri()` reads. */
function pretendDesktopApp(active: boolean) {
  (globalThis as unknown as Record<string, unknown>).isTauri = active;
  (window as unknown as Record<string, unknown>).isTauri = active;
}

type ChannelCallbacks = Map<number, (raw: unknown) => void>;

/**
 * `installFakeCore` records the callbacks a `Channel` registers but does not
 * expose them (the harness has no "deliver a channel message" hook yet).
 * Wrapping `transformCallback` immediately after installing therefore lets a
 * suite drive the real `Channel`, envelope and all, instead of reaching for the
 * production handler directly.
 */
function installHostWithChannels(
  options: Omit<FakeCoreOptions, "workspace"> = {},
): { core: FakeCore; channels: ChannelCallbacks } {
  const core = installFakeCore({ workspace: workspaceFixture(), ...options });
  const internals = (window as unknown as Record<string, unknown>)
    .__TAURI_INTERNALS__ as {
    transformCallback(callback: (raw: unknown) => void, once?: boolean): number;
  };
  const register = internals.transformCallback.bind(internals);
  const channels: ChannelCallbacks = new Map();
  internals.transformCallback = (callback, once) => {
    const id = register(callback, once);
    channels.set(id, callback);
    return id;
  };
  return { core, channels };
}

afterEach(() => {
  pretendDesktopApp(false);
  codec.heicTo = null;
});

// ---------------------------------------------------------------------------
// File classification
// ---------------------------------------------------------------------------

const lms = (name: string) =>
  `https://lms.s.isct.ac.jp/2025/pluginfile.php/1/mod_resource/content/1/${name}?forcedownload=1`;

describe("LMS file classification", () => {
  it("decodes the extension of an encoded LMS file name", () => {
    expect(extension(lms("%E8%AC%9B%E7%BE%A9.PDF"))).toBe("pdf");
    expect(extension(lms("README"))).toBe("");
  });

  it("lower-cases the extension and ignores a download query suffix", () => {
    expect(extension(lms("lecture.HEIC"))).toBe("heic");
    expect(extension(lms("talk.M4A"))).toBe("m4a");
    // A name that cannot be parsed as a URL is still read as a path.
    expect(extension("notes.MD")).toBe("md");
  });

  it("stops reading a suffix longer than eight characters as an extension", () => {
    expect(
      extension("https://lms.s.isct.ac.jp/files/document.verylongextension"),
    ).toBe("");
  });

  it("previews images, media, documents and source text in the app", () => {
    expect(previewKind(lms("photo.HEIC"))).toBe("image");
    expect(previewKind(lms("lecture.mp4"))).toBe("video");
    expect(previewKind(lms("talk.m4a"))).toBe("audio");
    expect(previewKind(lms("main.cpp"))).toBe("text");
    expect(previewKind(lms("slides.pdf"))).toBe("pdf");
    expect(previewKind("notes.md")).toBe("markdown");
  });

  it("downloads formats it cannot preview rather than showing them as text", () => {
    expect(previewKind(lms("slides.pptx"))).toBeUndefined();
    expect(previewKind(lms("notebook.ipynb"))).toBeUndefined();
    expect(previewKind(lms("page.html"))).toBeUndefined();
    expect(previewKind(lms("code.zip"))).toBeUndefined();
  });

  it("recognizes school code attachments and names their language", () => {
    for (const [ext, name] of [
      ["c", "C / C++"],
      ["cpp", "C / C++"],
      ["java", "Java"],
      ["py", "Python"],
      ["R", "R"],
      ["txt", "Plain text"],
    ] as const) {
      const url = `https://lms.s.isct.ac.jp/2025/pluginfile.php/1/lecture.${ext}?forcedownload=1`;
      expect(previewKind(url), ext).toBe("text");
      expect(textLanguage(url).name, ext).toBe(name);
    }
    for (const ext of ["tex", "rs", "json", "ts", "sh", "sql", "yaml", "go"]) {
      expect(previewKind(lms(`file.${ext}`)), ext).toBe("text");
    }
    expect(previewKind("https://lms.s.isct.ac.jp/1/notes.md")).toBe("markdown");
    expect(textLanguage("https://lms.s.isct.ac.jp/1/notes.md").name).toBe(
      "Plain text",
    );
  });

  it("gives every row a family, falling back to the Moodle module", () => {
    expect(fileFamily(lms("week1.pptx"))).toBe("slides");
    expect(fileFamily(lms("scores.xlsx"))).toBe("sheet");
    expect(fileFamily(lms("handout.docx"))).toBe("doc");
    expect(fileFamily(lms("code.zip"))).toBe("archive");
    expect(fileFamily("", "folder")).toBe("folder");
    expect(fileFamily("", "url")).toBe("link");
    expect(fileFamily("", "forum")).toBe("forum");
    expect(fileFamily("", "page")).toBe("page");
    expect(fileFamily("", "label")).toBe("page");
    expect(fileFamily("", "resource")).toBe("file");
  });
});

// ---------------------------------------------------------------------------
// File addresses and folder paths
// ---------------------------------------------------------------------------

describe("LMS file addresses", () => {
  const source = "https://lms.s.isct.ac.jp/2025/mod/resource/view.php?id=42";
  const context = {
    term: "2026-3Q",
    course: "CSC.T251",
    section: "section-91",
  };

  it("leaves the source address alone outside the desktop shell", () => {
    expect(files.lmsFileUrl(source, context, { native: false })).toBe(source);
  });

  it("rewrites the address onto the app scheme on macOS", () => {
    expect(
      files.lmsFileUrl(source, context, { native: true, os: "macos" }),
    ).toBe(
      "lmsfile://localhost/get?u=https%3A%2F%2Flms.s.isct.ac.jp%2F2025%2Fmod%2Fresource%2Fview.php%3Fid%3D42&term=2026-3Q&course=CSC.T251",
    );
  });

  it("uses the http form the Windows and Android webviews require", () => {
    for (const os of ["windows", "android"] as const) {
      expect(
        files.lmsFileUrl(source, context, { native: true, os }),
        os,
      ).toMatch(/^http:\/\/lmsfile\.localhost\/get\?/);
    }
  });

  it("carries the download and foreground flags when the caller asks for them", () => {
    expect(
      files.lmsFileUrl(source, context, {
        native: true,
        os: "android",
        download: true,
      }),
    ).toContain("download=1");
    expect(
      files.lmsFileUrl(source, context, {
        native: true,
        os: "macos",
        foreground: true,
      }),
    ).toContain("foreground=1");
  });

  it("takes the platform from the injected native OS when the caller does not name one", () => {
    window.__ISCT_COMPANION_NATIVE_OS__ = "windows";
    try {
      expect(files.lmsFileUrl(source, context, { native: true })).toMatch(
        /^http:\/\/lmsfile\.localhost\/get\?/,
      );
    } finally {
      delete window.__ISCT_COMPANION_NATIVE_OS__;
    }
  });

  it("rewrites through the app protocol by default inside the desktop shell", () => {
    delete window.__ISCT_COMPANION_NATIVE_OS__;
    expect(files.lmsFileUrl(source, context)).toMatch(
      /^lmsfile:\/\/localhost\/get\?/,
    );
  });

  it("bounds a folder path component and refuses traversal names", () => {
    expect(files.folderComponent("..")).toBe("course");
    expect(files.folderComponent(".")).toBe("course");
    expect(files.folderComponent("")).toBe("course");
    expect(files.folderComponent("x/y\\z")).toBe("x_y_z");
    expect(files.folderComponent("a".repeat(200))).toHaveLength(120);
  });
});

// ---------------------------------------------------------------------------
// Imported policy values
// ---------------------------------------------------------------------------

describe("course file policy bounds", () => {
  it("defaults the automatic download limit to 20 MB and clamps imported values", () => {
    expect(files.downloadLimit({})).toBe(20 * 1024 * 1024);
    expect(files.downloadLimit({ autoDownloadLimitMb: 0 })).toBe(1024 * 1024);
    expect(files.downloadLimit({ autoDownloadLimitMb: 2048 })).toBe(
      1024 * 1024 * 1024,
    );
    expect(files.downloadLimit({ autoDownloadLimitMb: Number.NaN })).toBe(
      20 * 1024 * 1024,
    );
    expect(files.downloadLimit({ autoDownloadLimitMb: 2.6 })).toBe(
      3 * 1024 * 1024,
    );
  });

  it("defaults the cleanup retention to 30 days and clamps imported values", () => {
    expect(files.retentionDays({})).toBe(30);
    expect(files.retentionDays({ autoCleanDays: 1.6 })).toBe(2);
    expect(files.retentionDays({ autoCleanDays: Number.NaN })).toBe(30);
    expect(files.retentionDays({ autoCleanDays: 0 })).toBe(1);
    expect(files.retentionDays({ autoCleanDays: 99_999 })).toBe(3650);
  });
});

// ---------------------------------------------------------------------------
// Note persistence
// ---------------------------------------------------------------------------

const noteContext = {
  term: "2026-3Q",
  course: "CSC.T251",
  section: "section-91",
};

type NoteHost = {
  core: FakeCore;
  /** What the host has durably stored. */
  committed(): string | null;
  /** Every draft the host has been asked to write, in arrival order. */
  savedTexts(): string[];
  /** Completes the first save the host is holding. */
  release(): void;
};

/**
 * A scripted host that holds the first `save_course_note` until `release()`,
 * so the suite can see what the shell does while a write is still in flight.
 */
function noteHost({
  holdFirst = true,
}: { holdFirst?: boolean } = {}): NoteHost {
  const state = {
    holding: holdFirst,
    committed: null as string | null,
    saved: [] as string[],
    pending: [] as (() => void)[],
  };
  const core = installFakeCore({
    workspace: workspaceFixture(),
    handle: (command, args) => {
      if (command === "save_course_note") {
        const text = String(args.text);
        state.saved.push(text);
        const body = {
          id: String(args.id),
          section: noteContext.section,
          name: "Note",
          path: "notes/ordered",
          bytes: text.length,
          cached: false,
          text,
        };
        if (state.holding) {
          state.holding = false;
          return new Promise((resolve) => {
            state.pending.push(() => {
              state.committed = text;
              resolve(body);
            });
          });
        }
        state.committed = text;
        return Promise.resolve(body);
      }
      if (command === "read_course_note")
        return Promise.resolve({
          id: String(args.id),
          section: noteContext.section,
          name: "Note",
          path: "notes/ordered",
          bytes: 0,
          cached: true,
          text: state.committed ?? "",
        });
      if (command === "move_course_note") return Promise.resolve(true);
      return undefined;
    },
  });
  return {
    core,
    committed: () => state.committed,
    savedTexts: () => state.saved,
    release: () => {
      for (const commit of state.pending.splice(0)) commit();
    },
  };
}

describe("note persistence", () => {
  it("writes notes in order, holding a later draft and a reopen until the pending save lands", async () => {
    const host = noteHost();
    const first = files.saveNote(noteContext, "ordered-note", "old");
    const second = files.saveNote(noteContext, "ordered-note", "latest");
    const reopened = files.readNote(noteContext, "ordered-note");

    await flush();
    expect(host.core.calls.map((call) => call.command)).toEqual([
      "save_course_note",
    ]);
    expect(host.savedTexts()).toEqual(["old"]);

    host.release();
    const [firstFile, secondFile, reopenedFile] = await Promise.all([
      first,
      second,
      reopened,
    ]);

    expect(host.core.calls.map((call) => call.command)).toEqual([
      "save_course_note",
      "save_course_note",
      "read_course_note",
    ]);
    expect(host.savedTexts()).toEqual(["old", "latest"]);
    expect(host.committed()).toBe("latest");
    expect(firstFile.text).toBe("old");
    expect(secondFile.text).toBe("latest");
    // The reopen waited for the write, so it reads the newest draft.
    expect(reopenedFile.text).toBe("latest");
  });

  it("rejects an oversized draft before the host is asked to write", async () => {
    const host = noteHost();
    await expect(
      files.saveNote(noteContext, "large-note", "あ".repeat(400_000)),
    ).rejects.toThrowError(/^Notes are limited to 1 MB$/);
    expect(host.core.calls).toEqual([]);
  });

  it("accepts a draft of exactly the byte limit and rejects one byte more", async () => {
    const host = noteHost({ holdFirst: false });
    const atLimit = "a".repeat(1_000_000);
    await expect(
      files.saveNote(noteContext, "limit-note", atLimit),
    ).resolves.toMatchObject({ id: "limit-note", text: atLimit });
    await expect(
      files.saveNote(noteContext, "limit-note", `${atLimit}a`),
    ).rejects.toThrowError(/^Notes are limited to 1 MB$/);
    expect(host.savedTexts()).toEqual([atLimit]);
  });

  it("moves a note only after its pending autosave, and delegates undo to the restore flag", async () => {
    const host = noteHost();
    const saving = files.saveNote(
      noteContext,
      "delete-ordered",
      "latest draft",
    );
    const moving = files.moveNote(noteContext, "delete-ordered");

    await flush();
    expect(host.core.calls.map((call) => call.command)).toEqual([
      "save_course_note",
    ]);

    host.release();
    await Promise.all([saving, moving]);
    expect(host.core.calls.map((call) => call.command)).toEqual([
      "save_course_note",
      "move_course_note",
    ]);
    expect(host.core.calls[1].args).toEqual({
      context: noteContext,
      id: "delete-ordered",
      restore: false,
    });

    await files.moveNote(noteContext, "delete-ordered", true);
    expect(host.core.calls.at(-1)?.command).toBe("move_course_note");
    expect(host.core.calls.at(-1)?.args).toEqual({
      context: noteContext,
      id: "delete-ordered",
      restore: true,
    });
  });
});

// ---------------------------------------------------------------------------
// Lessons
// ---------------------------------------------------------------------------

describe("lesson numbering", () => {
  it("reads the lesson number from the common section titles", () => {
    expect(lessonNumber("第2回")).toBe(2);
    expect(lessonNumber("第 12 回 クラス")).toBe(12);
    expect(lessonNumber("第３回")).toBe(3);
    expect(lessonNumber("Lecture 4")).toBe(4);
    expect(lessonNumber("Week 10: Graphs")).toBe(10);
    expect(lessonNumber("5. Recursion")).toBe(5);
  });

  it("leaves a section without a lesson number alone", () => {
    expect(lessonNumber("一般")).toBeUndefined();
    expect(lessonNumber("連絡先・オフィスアワー")).toBeUndefined();
    expect(lessonNumber("2026年度 講義資料")).toBeUndefined();
  });
});

describe("lesson summary", () => {
  it("summarises a lesson by the first line of its plan, without a short leading label", () => {
    expect(lessonSummary("<p>授業計画：正規文法<br>課題：確認する</p>")).toBe(
      "正規文法",
    );
    expect(
      lessonSummary("<p>Course schedule: Basic syntax</p><p>Objectives: …</p>"),
    ).toBe("Basic syntax");
  });

  it("has no summary for a lesson without a plan", () => {
    expect(lessonSummary(undefined)).toBe("");
    expect(lessonSummary("")).toBe("");
  });
});

describe("course lessons", () => {
  const sections = [
    { id: 1, title: "一般" },
    { id: 2, title: "第1回" },
    { id: 3, title: "第2回" },
    { id: 4, title: "第3回" },
  ];

  it("keeps the numbered sections in order, and stops before the last without material", () => {
    const lessons = courseLessons(sections, (id) => id <= 3);
    expect(lessons.map((lesson) => lesson.number)).toEqual([1, 2, 3]);
    expect(lessons.map((lesson) => lesson.posted)).toEqual([true, true, false]);
  });

  it("is at the last lesson with posted material, or the first before anything is posted", () => {
    expect(
      frontierLesson(courseLessons(sections, (id) => id <= 3))?.number,
    ).toBe(2);
    expect(frontierLesson(courseLessons(sections, () => false))?.number).toBe(
      1,
    );
  });

  it("is not a lesson course when fewer than two sections are numbered", () => {
    expect(courseLessons(sections.slice(0, 2), () => true)).toEqual([]);
  });
});

describe("lesson schedule marking", () => {
  const html =
    "<table><tr><th>日付</th><th>授業回目</th></tr><tr><td>10/2</td><td>1回目</td></tr><tr><td>10/16</td><td>休講</td></tr><tr><td>10/20</td><td>2回目</td></tr><tr><td>10/23</td><td>3回目</td></tr></table>";
  const rowClasses = (marked: string) =>
    [
      ...new DOMParser()
        .parseFromString(marked, "text/html")
        .querySelectorAll("tr"),
    ].map((row) => row.className);

  it("marks the rows before the current lesson, the current row and a cancelled one", () => {
    expect(rowClasses(markSchedule(html, 2))).toEqual([
      "",
      "lmsRowPast",
      "lmsRowOff lmsRowPast",
      "lmsRowNow",
      "",
    ]);
  });

  it("leaves a page without a qualifying lesson table untouched", () => {
    expect(markSchedule("<p>no table</p>", 2)).toBe("<p>no table</p>");
    const single =
      "<table><tr><td>1回目</td><td>intro</td></tr><tr><td>reading</td><td>—</td></tr></table>";
    expect(markSchedule(single, 2)).toBe(single);
    expect(markSchedule(html)).toBe(html);
  });
});

describe("lesson rail labels", () => {
  const at = (numbers: number[]) =>
    numbers.map((number, id) => ({ id, number, posted: true }));

  it("uses the lessons' own numbers when they count up by one", () => {
    expect(railLabels(at([0, 1, 2]))).toEqual(["0", "1", "2"]);
    expect(railLabels(at([1, 2, 3]))).toEqual(["1", "2", "3"]);
  });

  it("falls back to the order when the titles skip or repeat a number", () => {
    expect(railLabels(at([7, 9, 8, 11, 11]))).toEqual([
      "1",
      "2",
      "3",
      "4",
      "5",
    ]);
  });
});

describe("marked lesson expiry", () => {
  const lessons = [1, 2, 3].map((number) => ({
    id: 10 + number,
    number,
    posted: true,
  }));
  const log = [
    { occurrence: "2026-10-06T15:25:00+09:00", sectionId: 11 },
    { occurrence: "2026-10-08T10:45:00+09:00", sectionId: 12 },
  ];

  it("stays with the marked lesson while it is the class that has started", () => {
    expect(
      markedLesson(lessons, log, "2026-10-08T10:45:00+09:00")?.number,
    ).toBe(2);
  });

  it("moves to the next lesson once a later class has started", () => {
    expect(
      markedLesson(lessons, log, "2026-10-15T10:45:00+09:00")?.number,
    ).toBe(3);
  });

  it("uses the latest mark even when the log arrives out of order", () => {
    const shuffled = [...log].reverse();
    expect(
      markedLesson(lessons, shuffled, "2026-10-08T10:45:00+09:00")?.number,
    ).toBe(2);
  });

  it("has no marked lesson without a mark for one of this course's sections", () => {
    expect(
      markedLesson(lessons, null, "2026-10-15T10:45:00+09:00"),
    ).toBeUndefined();
    expect(
      markedLesson(lessons, [], "2026-10-15T10:45:00+09:00"),
    ).toBeUndefined();
    expect(
      markedLesson(
        lessons,
        [{ occurrence: "2026-10-01T10:45:00+09:00", sectionId: 99 }],
        "2026-10-15T10:45:00+09:00",
      ),
    ).toBeUndefined();
  });
});

describe("outline scroll gravity", () => {
  const height = 500;
  const boundaries = [1000, 1600];

  it("brings a section boundary in the top band to the top of the outline", () => {
    // Just above the outline is drawn back down; just below it is pulled up.
    expect(gravityTarget(940, height, boundaries)).toBe(1000);
    expect(gravityTarget(1050, height, boundaries)).toBe(1000);
  });

  it("brings a section end in the bottom band back so the old section ends at the bottom", () => {
    expect(gravityTarget(560, height, boundaries)).toBe(500);
  });

  it("leaves the middle of a section and an outline already resting on a boundary alone", () => {
    expect(gravityTarget(1250, height, boundaries)).toBeUndefined();
    expect(gravityTarget(750, height, boundaries)).toBeUndefined();
    expect(gravityTarget(1000, height, boundaries)).toBeUndefined();
  });

  it("prefers a start near the top over the next start near the bottom", () => {
    expect(gravityTarget(940, height, [1000, 1400])).toBe(1000);
  });

  it("keeps the settled position inside the scrollable range", () => {
    expect(gravityTarget(940, height, boundaries, 1300)).toBe(800);
  });
});

// ---------------------------------------------------------------------------
// Assignment workspace
// ---------------------------------------------------------------------------

const assignmentHtml = `<p>Implement the classes.</p><ul><li>Use Java 21</li><li>Write tests</li></ul>
<a href="https://lms.s.isct.ac.jp/2026/pluginfile.php/1/mod_assign/intro/OOP-2026-1B-problem.zip">problem</a>
<a href="https://lms.s.isct.ac.jp/2026/pluginfile.php/1/mod_assign/intro/OOP-2026-1B-problem.zip">again</a>
<a href="https://example.com/pluginfile.php/evil.zip">elsewhere</a>`;

describe("assignment attachments", () => {
  it("collects each LMS attachment once, named by the file it points at", () => {
    expect(assignments.assignmentAttachments(assignmentHtml)).toEqual([
      {
        url: "https://lms.s.isct.ac.jp/2026/pluginfile.php/1/mod_assign/intro/OOP-2026-1B-problem.zip",
        name: "OOP-2026-1B-problem.zip",
      },
    ]);
  });

  it("keeps only https links on the school's LMS host", () => {
    const other = `<a href="http://lms.s.isct.ac.jp/pluginfile.php/1/x.zip">plain</a>
<a href="https://example.com/pluginfile.php/evil.zip">elsewhere</a>
<a href="/pluginfile.php/relative.zip">relative</a>`;
    expect(assignments.assignmentAttachments(other)).toEqual([]);
  });

  it("decodes the file name and falls back to the link text", () => {
    const names = `<a href="https://lms.s.isct.ac.jp/pluginfile.php/1/%E8%AA%B2%E9%A1%8C.pdf">課題</a>
<a href="https://lms.s.isct.ac.jp/pluginfile.php/1/mod_assign/intro/">instructions</a>`;
    expect(assignments.assignmentAttachments(names)).toEqual([
      {
        url: "https://lms.s.isct.ac.jp/pluginfile.php/1/%E8%AA%B2%E9%A1%8C.pdf",
        name: "課題.pdf",
      },
      {
        url: "https://lms.s.isct.ac.jp/pluginfile.php/1/mod_assign/intro/",
        name: "instructions",
      },
    ]);
  });
});

describe("assignment brief", () => {
  it("writes the brief with the facts, the description and the attachments", () => {
    const brief = assignments.assignmentBrief({
      title: "1B",
      course: "OOP",
      due: "2026/10/20 23:59 JST",
      html: assignmentHtml,
      attachments: ["OOP-2026-1B-problem.zip"],
    });
    expect(brief).toContain(
      "# 1B\n\n- Course: OOP\n- Due: 2026/10/20 23:59 JST\n",
    );
    expect(brief).toContain("- Hand in: put the files to submit in `submit/`");
    expect(brief).toContain(
      "Implement the classes.\n- Use Java 21\n- Write tests",
    );
    expect(brief).toContain("## Attachments\n\n- OOP-2026-1B-problem.zip\n");
  });

  it("includes the LMS address only when one is given", () => {
    const withUrl = assignments.assignmentBrief({
      title: "1B",
      course: "OOP",
      url: "https://lms.s.isct.ac.jp/mod/assign/view.php?id=1",
      html: "<p>Do it.</p>",
      attachments: [],
    });
    expect(withUrl).toContain(
      "- LMS: https://lms.s.isct.ac.jp/mod/assign/view.php?id=1",
    );
    const without = assignments.assignmentBrief({
      title: "1B",
      course: "OOP",
      html: "<p>Do it.</p>",
      attachments: [],
    });
    expect(without).not.toContain("- Due:");
    expect(without).not.toContain("- LMS:");
    expect(without).not.toContain("## Attachments");
  });

  it("strips scripts and keeps paragraph and list structure in the description", () => {
    const brief = assignments.assignmentBrief({
      title: "Lab",
      course: "OOP",
      html: "<script>alert(1)</script><p>First</p><ul><li>One</li><li>Two</li></ul>",
      attachments: [],
    });
    expect(brief).not.toContain("alert");
    expect(brief).toContain("First\n- One\n- Two");
  });
});

// ---------------------------------------------------------------------------
// Spring scroll
// ---------------------------------------------------------------------------

describe("spring scroll", () => {
  it("arrives within about a third of a second without overshooting visibly", () => {
    let state = { position: 0, velocity: 0 };
    let arrivedAt: number | undefined;
    let furthest = 0;
    for (let frame = 1; frame <= 120; frame++) {
      state = springStep(
        state.position,
        state.velocity,
        400,
        1 / 60,
        stiffnessFor(400),
      );
      furthest = Math.max(furthest, state.position);
      if (
        arrivedAt == null &&
        Math.abs(state.position - 400) < 0.5 &&
        Math.abs(state.velocity) < 8
      )
        arrivedAt = frame / 60;
    }
    expect(arrivedAt).toBeGreaterThan(0.2);
    expect(arrivedAt).toBeLessThan(0.6);
    // At most a hair past the target.
    expect(furthest - 400).toBeLessThan(2);
  });

  it("is softer for long jumps", () => {
    expect(stiffnessFor(3000)).toBeLessThan(stiffnessFor(300));
    expect(stiffnessFor(100)).toBe(14);
  });
});

// ---------------------------------------------------------------------------
// Matrix OCR
// ---------------------------------------------------------------------------

/**
 * `heic-to/csp` decodes through a `Worker`, `URL.createObjectURL` and a
 * `<canvas>`; jsdom provides none of them, so its decode has no headless mode
 * (the README's "third-party renderer" exception). The mock delegates to the
 * real codec by default — so magic-byte inspection is real — and only the
 * tests that need decoded JPEG bytes replace `heicTo`. This never stands in
 * for the module under test.
 */
type HeicToArgs = { blob: Blob; type: string; quality?: number };

const codec = vi.hoisted(() => ({
  heicTo: null as ((args: HeicToArgs) => Promise<Blob>) | null,
}));

vi.mock("heic-to/csp", async (importOriginal) => {
  const actual = await importOriginal<typeof import("heic-to/csp")>();
  const heicTo = ((args: HeicToArgs) =>
    codec.heicTo
      ? codec.heicTo(args)
      : actual.heicTo(args as never)) as unknown as typeof actual.heicTo;
  return { ...actual, isHeic: actual.isHeic, heicTo };
});

/** A file whose HEIC brand the real codec's magic-byte check recognizes. */
function heicFile(name: string): File {
  const header = new Uint8Array([
    0, 0, 0, 0x18, 0x66, 0x74, 0x79, 0x70, 0x68, 0x65, 0x69, 0x63, 1, 2, 3, 4,
  ]);
  return new File([header], name, { type: "image/heic" });
}

describe("matrix confidence and text", () => {
  it("keeps the review bands on their thresholds", () => {
    expect(matrixConfidence("A", 0.95)).toBe("high");
    expect(matrixConfidence("A", 0.9)).toBe("high");
    expect(matrixConfidence("A", 0.89)).toBe("medium");
    expect(matrixConfidence("A", 0.75)).toBe("medium");
    expect(matrixConfidence("I", 0.74)).toBe("low");
  });

  it("treats a missing, non-finite or empty reading as low confidence", () => {
    expect(matrixConfidence("", 0.99)).toBe("low");
    expect(matrixConfidence("A", Number.NaN)).toBe("low");
    expect(matrixConfidence("A", Number.POSITIVE_INFINITY)).toBe("low");
  });

  it("normalizes a manual paste to the letters it contains, capped at the Matrix size", () => {
    expect(normalizeMatrixText("a, b 0 1 8")).toBe("AB");
    expect(normalizeMatrixText("a".repeat(80))).toHaveLength(MATRIX_SIZE);
    expect(normalizeMatrixText("a".repeat(80))).toBe("A".repeat(MATRIX_SIZE));
  });

  it("reads whitespace-formatted Matrix text in visual row order", () => {
    const rowMajor = Array.from({ length: MATRIX_SIZE }, (_, index) =>
      String.fromCharCode(65 + (index % 26)),
    );
    const rows = Array.from({ length: 7 }, (_, row) =>
      rowMajor.slice(row * 10, row * 10 + 10).join(" "),
    ).join("\n");
    expect(parseMatrixText(rows)).toEqual(rowMajor);
    expect(parseMatrixText(rowMajor.join("").toLowerCase())).toEqual(rowMajor);
    expect(parseMatrixText(rowMajor.join("\t"))).toEqual(rowMajor);
  });

  it.each([
    ["69 letters", Array.from({ length: 69 }, () => "A").join("")],
    ["71 letters", Array.from({ length: 71 }, () => "A").join("")],
    ["punctuation", `${Array.from({ length: 69 }, () => "A").join("")}-`],
  ])("rejects Matrix text with %s", (_name, text) => {
    expect(() => parseMatrixText(text)).toThrowError(
      /^Paste exactly 70 letters in row order/,
    );
  });
});

describe("matrix image preparation", () => {
  it("passes supported image bytes through without touching the codec's decoder", async () => {
    await expect(
      prepareMatrixImage(new File(["png"], "photo.png")),
    ).resolves.toBe("cG5n");
  });

  it("converts HEIC by contents to shell-provided JPEG bytes", async () => {
    // The codec echoes the format it was asked for, so the assertion reads the
    // production function's output rather than the codec's arguments.
    codec.heicTo = async (args) =>
      new Blob([`jpeg:${args.type}:${args.quality}`], { type: args.type });
    const converted = await prepareMatrixImage(heicFile("photo.heic"));
    expect(window.atob(converted)).toBe("jpeg:image/jpeg:0.95");
  });

  it("rejects empty and oversized input before reading the codec", async () => {
    await expect(
      prepareMatrixImage(new File([], "empty.png")),
    ).rejects.toThrowError(/^Choose a valid image smaller than 20 MB\.$/);
    await expect(
      prepareMatrixImage(
        new File([new Uint8Array(20 * 1024 * 1024 + 1)], "big.png"),
      ),
    ).rejects.toThrowError(/^Choose a valid image smaller than 20 MB\.$/);
  });

  it("reports a HEIC decode failure without codec details", async () => {
    await expect(
      prepareMatrixImage(heicFile("photo.heic")),
    ).rejects.toThrowError(
      /^Cannot decode this HEIC image\. Try exporting it as PNG or JPEG\.$/,
    );
  });

  it("reports a decoded HEIC that grew past the size limit", async () => {
    codec.heicTo = async () => new Blob([new Uint8Array(20 * 1024 * 1024 + 1)]);
    await expect(
      prepareMatrixImage(heicFile("photo.heic")),
    ).rejects.toThrowError(
      /^Decoded HEIC image exceeds 20 MB\. Try exporting it as JPEG\.$/,
    );
  });

  it("reports an image whose bytes cannot be inspected without codec details", async () => {
    const unreadable = Object.assign(new File(["bad"], "photo.bin"), {
      arrayBuffer: () => Promise.reject(new Error("parser detail")),
    });
    await expect(prepareMatrixImage(unreadable)).rejects.toThrowError(
      /^Cannot read this image\. Choose a valid PNG, JPEG, WebP or HEIC photo\.$/,
    );
  });
});

describe("matrix image recognition", () => {
  it("refuses to scan outside the desktop app", async () => {
    // The fake host installs `isTauri`, because it stands in for the native
    // bootstrap; this case is the browser build, so it turns the flag off again
    // afterwards.
    const core = installFakeCore({ workspace: workspaceFixture() });
    pretendDesktopApp(false);
    await expect(
      recognizeMatrixImage(new File(["raw"], "photo.jpg")),
    ).rejects.toThrowError(/^Scan image is available in the desktop app\.$/);
    expect(core.calls).toEqual([]);
  });

  it("sends one raw image and forwards typed native progress", async () => {
    pretendDesktopApp(true);
    const recognitions = [
      { text: "A", confidence: 0.95 },
      { text: "B", confidence: 0.9 },
    ];
    const progress: [number, number][] = [];
    const { core, channels } = installHostWithChannels({
      handle: (command, args) => {
        if (command !== "recognize_matrix_image") return undefined;
        const channel = args.onProgress as { id: number };
        channels.get(channel.id)?.({
          index: 0,
          message: { completed: 12, total: 70 },
        });
        return recognitions;
      },
    });

    await expect(
      recognizeMatrixImage(new File(["raw"], "photo.jpg"), (completed, total) =>
        progress.push([completed, total]),
      ),
    ).resolves.toEqual(recognitions);

    expect(core.calls[0].command).toBe("recognize_matrix_image");
    expect(core.calls[0].args.image).toBe("cmF3");
    expect(core.calls[0].args.onProgress).toBeDefined();
    expect(progress).toEqual([[12, 70]]);
  });

  it("rethrows a host failure that is not an Error as an Error", async () => {
    pretendDesktopApp(true);
    installFakeCore({
      workspace: workspaceFixture(),
      handle: (command) =>
        command === "recognize_matrix_image"
          ? Promise.reject("shell exploded")
          : undefined,
    });
    await expect(
      recognizeMatrixImage(new File(["raw"], "photo.jpg")),
    ).rejects.toBeInstanceOf(Error);
    await expect(
      recognizeMatrixImage(new File(["raw"], "photo.jpg")),
    ).rejects.toThrowError(/^shell exploded$/);
  });
});

describe("Live Matrix camera ownership", () => {
  const originalMediaDevices = Object.getOwnPropertyDescriptor(
    navigator,
    "mediaDevices",
  );
  let close: (() => void) | undefined;
  afterEach(() => {
    close?.();
    close = undefined;
    vi.useRealTimers();
    vi.restoreAllMocks();
    if (originalMediaDevices)
      Object.defineProperty(navigator, "mediaDevices", originalMediaDevices);
    else Reflect.deleteProperty(navigator, "mediaDevices");
  });

  function cameraSurface(acquire: () => Promise<MediaStream>) {
    // jsdom has no camera, video decoder or canvas renderer; these are the
    // platform ports, while the production capture loop and IPC remain real.
    Object.defineProperty(navigator, "mediaDevices", {
      configurable: true,
      value: { getUserMedia: acquire },
    });
    const video = document.createElement("video");
    let currentTime = 1;
    Object.defineProperties(video, {
      videoWidth: { value: 1920 },
      videoHeight: { value: 1080 },
      readyState: { value: 2 },
      currentTime: { get: () => currentTime },
    });
    vi.spyOn(video, "play").mockResolvedValue();
    const draw = vi.fn(() => currentTime);
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
      drawImage: draw,
    } as unknown as CanvasRenderingContext2D);
    vi.spyOn(HTMLCanvasElement.prototype, "toDataURL").mockReturnValue(
      "data:image/jpeg;base64,Y2FtZXJh",
    );
    return { video, draw, advance: (time: number) => (currentTime = time) };
  }
  function streamPort() {
    const track = Object.assign(new EventTarget(), { stop: vi.fn() });
    return {
      track,
      stream: {
        getTracks: () => [track],
        getVideoTracks: () => [track],
      } as unknown as MediaStream,
    };
  }
  it("drops old frames, releases capture before background processing and ignores late progress", async () => {
    const { openMatrixCamera } = await import("@app/platform/matrix-camera");
    const { track, stream } = streamPort();
    vi.useFakeTimers();
    const { video, draw, advance } = cameraSurface(async () => stream);
    const capturing = {
      phase: "capturing",
      bounds: [
        { x: 0.1, y: 0.2 },
        { x: 0.9, y: 0.2 },
        { x: 0.9, y: 0.8 },
        { x: 0.1, y: 0.8 },
      ],
      readyCells: 0,
      captureElapsedMs: 0,
      timedOut: false,
      completedSamples: 0,
      totalSamples: 0,
      cells: [],
    };
    const processing = {
      ...capturing,
      phase: "processing",
      readyCells: 70,
      totalSamples: 210,
    };
    const finished = {
      ...processing,
      phase: "complete",
      completedSamples: 210,
      cells: [{ text: "A", confidence: 0.99, stable: true }],
    };
    let deliver!: (snapshot: unknown) => void;
    let finish!: (snapshot: unknown) => void;
    const { core, channels } = installHostWithChannels({
      handle: (command) => {
        if (command === "begin_matrix_camera") return "camera-1";
        if (command === "poll_matrix_camera") return capturing;
        if (command === "capture_matrix_camera_frame")
          return new Promise((resolve) => {
            deliver = resolve;
          });
        if (command === "process_matrix_camera")
          return new Promise((resolve) => {
            finish = resolve;
          });
        return undefined;
      },
    });
    const onSnapshot = vi.fn(),
      onError = vi.fn();
    close = openMatrixCamera(video, onSnapshot, onError);
    await vi.waitFor(() =>
      expect(core.commands("capture_matrix_camera_frame")).toHaveLength(1),
    );
    advance(2);
    await vi.advanceTimersByTimeAsync(100);
    advance(3);
    await vi.advanceTimersByTimeAsync(100);
    advance(4);
    await vi.advanceTimersByTimeAsync(100);
    expect(draw).toHaveBeenCalledTimes(1);
    expect(core.commands("capture_matrix_camera_frame")[0].args).toMatchObject({
      sessionId: "camera-1",
      image: "Y2FtZXJh",
    });
    expect(onSnapshot).not.toHaveBeenCalled();
    deliver(capturing);
    await vi.advanceTimersByTimeAsync(1);
    expect(core.commands("capture_matrix_camera_frame")).toHaveLength(2);
    expect(draw.mock.results.map((result) => result.value)).toEqual([1, 4]);
    deliver(capturing);
    await vi.advanceTimersByTimeAsync(100);
    expect(core.commands("capture_matrix_camera_frame")).toHaveLength(2);
    expect(core.commands("poll_matrix_camera")).toHaveLength(1); // deadlines advance during a video stall
    advance(5);
    await vi.advanceTimersByTimeAsync(100);
    expect(core.commands("capture_matrix_camera_frame")).toHaveLength(3);
    deliver(processing);
    await vi.advanceTimersByTimeAsync(1);
    expect(track.stop).toHaveBeenCalledTimes(1);
    expect(video.srcObject).toBeNull();
    expect(onSnapshot).toHaveBeenCalledWith(processing);
    expect(core.commands("process_matrix_camera")).toHaveLength(1);
    expect(core.commands("stop_matrix_camera")).toHaveLength(0); // core still owns background work
    const channel = core.commands("process_matrix_camera")[0].args
      .onProgress as { id: number };
    const progress = { ...processing, completedSamples: 10 };
    channels.get(channel.id)?.({ index: 0, message: progress });
    expect(onSnapshot).toHaveBeenCalledWith(progress);
    vi.spyOn(document, "hidden", "get").mockReturnValue(true);
    document.dispatchEvent(new Event("visibilitychange"));
    expect(core.commands("stop_matrix_camera")).toHaveLength(0); // processing can continue in background
    finish(finished);
    await vi.waitFor(() => expect(onSnapshot).toHaveBeenCalledWith(finished));
    expect(core.commands("stop_matrix_camera")).toHaveLength(1);
    const metrics = core.commands("stop_matrix_camera")[0].args.capture as {
      frames: number;
      frameMs: number[];
      elapsedMs: number;
      trackedMs: number;
    };
    expect(metrics.frames).toBe(3);
    expect(metrics.frameMs).toHaveLength(3);
    expect(metrics.elapsedMs).toBeGreaterThanOrEqual(metrics.trackedMs);
    expect(metrics.trackedMs).toBeGreaterThanOrEqual(0);
    const calls = onSnapshot.mock.calls.length;
    channels.get(channel.id)?.({ index: 1, message: progress });
    expect(onSnapshot).toHaveBeenCalledTimes(calls);
    expect(track.stop).toHaveBeenCalledTimes(1);
    expect(onError).not.toHaveBeenCalled();
  });

  it("releases a camera permission granted after cancellation and rejects late frame results", async () => {
    const { openMatrixCamera } = await import("@app/platform/matrix-camera");
    const { track, stream } = streamPort();
    let grant!: (stream: MediaStream) => void;
    const acquire = vi.fn(
      () =>
        new Promise<MediaStream>((resolve) => {
          grant = resolve;
        }),
    );
    const { video } = cameraSurface(acquire);
    let deliver!: (snapshot: unknown) => void;
    let finish!: (snapshot: unknown) => void;
    const core = installFakeCore({
      workspace: workspaceFixture(),
      handle: (command) => {
        if (command === "begin_matrix_camera") return "camera-2";
        if (command === "capture_matrix_camera_frame")
          return new Promise((resolve) => {
            deliver = resolve;
          });
        if (command === "process_matrix_camera")
          return new Promise((resolve) => {
            finish = resolve;
          });
        return undefined;
      },
    });
    const onSnapshot = vi.fn(),
      onError = vi.fn();
    close = openMatrixCamera(video, onSnapshot, onError);
    await vi.waitFor(() => expect(acquire).toHaveBeenCalled());
    close();
    grant(stream);
    await vi.waitFor(() => expect(track.stop).toHaveBeenCalledTimes(1));
    expect(core.commands("capture_matrix_camera_frame")).toHaveLength(0);
    close = openMatrixCamera(video, onSnapshot, onError);
    await vi.waitFor(() => expect(acquire).toHaveBeenCalledTimes(2));
    grant(stream);
    await vi.waitFor(() =>
      expect(core.commands("capture_matrix_camera_frame")).toHaveLength(1),
    );
    close();
    deliver({ complete: true, cells: [] });
    await flush();
    expect(onSnapshot).not.toHaveBeenCalled();
    expect(onError).not.toHaveBeenCalled();
    expect(core.commands("stop_matrix_camera")).toHaveLength(2);
    close = openMatrixCamera(video, onSnapshot, onError);
    await vi.waitFor(() => expect(acquire).toHaveBeenCalledTimes(3));
    grant(stream);
    await vi.waitFor(() =>
      expect(core.commands("capture_matrix_camera_frame")).toHaveLength(2),
    );
    deliver({
      phase: "processing",
      bounds: null,
      cells: [],
      readyCells: 70,
      completedSamples: 0,
      totalSamples: 210,
    });
    await vi.waitFor(() =>
      expect(core.commands("process_matrix_camera")).toHaveLength(1),
    );
    close();
    const published = onSnapshot.mock.calls.length;
    finish({ phase: "complete", cells: [] });
    await flush();
    expect(onSnapshot).toHaveBeenCalledTimes(published);
    expect(onError).not.toHaveBeenCalled();
    expect(core.commands("stop_matrix_camera")).toHaveLength(3);
  });

  it("reports a denied camera permission and ends the native session", async () => {
    const { openMatrixCamera } = await import("@app/platform/matrix-camera");
    const { video } = cameraSurface(async () => {
      throw new DOMException("denied", "NotAllowedError");
    });
    const core = installFakeCore({
      workspace: workspaceFixture(),
      handle: (command) =>
        command === "begin_matrix_camera" ? "denied" : undefined,
    });
    const onError = vi.fn();
    close = openMatrixCamera(video, vi.fn(), onError);
    await vi.waitFor(() => expect(onError).toHaveBeenCalled());
    expect(onError.mock.calls[0][0].message).toContain(
      "Camera access was denied",
    );
    expect(core.commands("stop_matrix_camera")).toHaveLength(1);
  });
});
