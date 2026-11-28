// @vitest-environment jsdom
import { Activity, act, useState, type ReactElement } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  advance,
  buttonByText,
  click,
  installComponentHost,
  mount,
  typeInto,
  until,
  workspaceFixture,
  type Mounted,
} from "@tests/harness";

/**
 * The LMS reading workspace's own paths, over the scripted host.
 *
 * Each case mounts the production component that owns one path — the
 * assignment footer, the file preview, the reader's LMS view, the note
 * editor, the PDF viewer, the course HTML renderer — and checks where the
 * person's data and the host end up: the folder Core reports, the note on
 * disk, the page Core is asked to open. None of them needs an LMS course: the
 * outline itself is Core's `courseContent` over a real enrolment, and what it
 * draws is checked by hand (`COURSE-LESSON-SERIES-LABELS`,
 * `DESKTOP-PDF-TOOLBAR` in docs/verification/manual-checks.yaml).
 *
 * `pdfjs-dist` has no headless mode (it needs a worker and a canvas), so it
 * alone is mocked; `PdfDocument` stays real. The Milkdown editor is driven
 * through its real ProseMirror transactions.
 */

const pdfjs = vi.hoisted(() => ({ load: vi.fn(), render: vi.fn() }));

vi.mock("pdfjs-dist/legacy/build/pdf.mjs", () => ({
  GlobalWorkerOptions: {},
  PasswordResponses: { NEED_PASSWORD: 1, INCORRECT_PASSWORD: 2 },
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
const COURSE_ID = "test-course";
const CONTEXT = { courseId: COURSE_ID, section: "section-91" };

let views: Mounted[] = [];

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
});

afterEach(async () => {
  for (const mounted of views.splice(0)) {
    await mounted.unmount().catch(() => undefined);
  }
  vi.restoreAllMocks();
  localStorage.clear();
  sessionStorage.clear();
});

/**
 * Waits generously: the gate runs every suite, and this file shares the machine
 * with sibling suites, so a 5s poll is not enough when the box is loaded.
 */
function waitFor<T>(check: () => T): Promise<T> {
  return until(check, { timeout: 15_000 });
}

/** The element, or an error naming what has not appeared yet. */
function required<T>(value: T | null | undefined, what: string): T {
  if (value == null) throw new Error(`${what} is not there yet.`);
  return value;
}

async function view(element: ReactElement): Promise<Mounted> {
  const mounted = await mount(element);
  views.push(mounted);
  return mounted;
}

async function release(mounted: Mounted): Promise<void> {
  const index = views.indexOf(mounted);
  if (index >= 0) views.splice(index, 1);
  await mounted.unmount();
}

/** A button whose label starts with `label` (the rest is a size). */
function buttonStarting(host: ParentNode, label: string) {
  return [...host.querySelectorAll("button")].find((button) =>
    (button.textContent ?? "").trim().startsWith(label),
  );
}

function itemRow(host: HTMLElement, text: string): HTMLElement {
  const row = [...host.querySelectorAll<HTMLElement>(".lmsItemRow")].find(
    (item) => item.textContent?.includes(text),
  );
  if (!row) throw new Error(`no row for ${text}`);
  return row;
}

const lmsPage = (url: string) => ({ site: "science-lms", url });

/** A file Core has in the course folder. */
const storedFile = (path: string, viewer: string, text?: string) => ({
  id: path,
  name: path.split("/").pop(),
  section: "",
  path,
  bytes: 10,
  cached: true,
  viewer,
  ...(text === undefined ? {} : { text }),
});

/** One PDF page as PDF.js describes it. */
const sheet = () => ({
  getViewport: ({ scale }: { scale: number }) => ({
    width: 600 * scale,
    height: 800 * scale,
  }),
  render: pdfjs.render,
  cleanup: () => {},
  streamTextContent: () => ({}),
});

describe("an assignment's workspace folder", () => {
  it("hands its files to the LMS draft, survives the folder vanishing, and is deleted only once confirmed", async () => {
    const report = {
      path: "/courses/Report",
      name: "Report",
      submitFiles: ["report.pdf"],
      zip: false,
      zipName: "Report.zip",
    };
    const draft =
      "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=100&action=editsubmission";
    // The person's folder on disk, as Core reports it.
    let folder: typeof report | null = report;
    let submitEmpty = false;
    await installComponentHost({
      workspace: workspaceFixture(),
      handle: (command, args) => {
        switch (command) {
          case "assignment_workspace":
            return folder
              ? { workspace: folder, handIn: "prepareDraft" }
              : { workspace: null, handIn: "openLms" };
          case "prepare_workspace_submission":
            if (args.path !== folder?.path)
              return Promise.reject({
                code: "workspace-folder-missing",
                detail: "The workspace folder no longer exists",
              });
            if (submitEmpty)
              return Promise.reject({
                code: "hand-in-empty",
                detail: "The submit folder is empty",
              });
            return { workspace: folder, url: draft };
          case "create_assignment_workspace":
            folder = report;
            return { workspace: folder, missing: [] };
          case "delete_workspace":
            if (args.path === folder?.path) folder = null;
            return null;
        }
        return undefined;
      },
    });
    const { AssignmentWorkspaceFooter } =
      await import("@app/features/lms/AssignmentWorkspaceCard");
    const { ConfirmDialogs } = await import("@app/components/confirm");
    // What the shell is asked to open: the prepared draft, or the plain page.
    const lms: (string | undefined)[] = [];
    const { host } = await view(
      <>
        <AssignmentWorkspaceFooter
          assignment={{ id: "a1" }}
          state={undefined}
          submit={(url) => lms.push(url)}
        />
        <ConfirmDialogs />
      </>,
    );
    const button = (label: string) =>
      waitFor(() => required(buttonByText(host, label), label));
    const trash = () =>
      waitFor(() =>
        required(
          host.querySelector<HTMLButtonElement>(
            '[aria-label="Delete workspace"]',
          ),
          "the workspace card",
        ),
      );
    const dialog = () => document.querySelector<HTMLElement>('[role="dialog"]');
    const answer = (label: string) =>
      waitFor(() =>
        required(
          dialog() && buttonByText(dialog()!, label),
          "the confirmation",
        ),
      );

    // The files go to the requested assignment's draft, which the shell opens.
    await click(await button("Upload to LMS draft"));
    await waitFor(() => expect(lms).toEqual([draft]));

    // Core refuses an empty submit folder: nothing opens.
    submitEmpty = true;
    await click(await button("Upload to LMS draft"));
    await waitFor(() =>
      expect(host.querySelector('[role="alert"]')).not.toBeNull(),
    );
    expect(lms).toEqual([draft]);

    // Deleted in Finder: the next upload finds the folder gone, Core offers a
    // new one, and the LMS page still opens without it.
    submitEmpty = false;
    folder = null;
    await click(await button("Upload to LMS draft"));
    const create = await button("Create workspace");
    await click(await button("View in LMS"));
    expect(lms).toEqual([draft, undefined]);

    // Made again, the folder goes only after the in-app confirmation.
    await click(create);
    await click(await trash());
    await click(await answer("Cancel"));
    await waitFor(() => expect(dialog()).toBeNull());
    expect(folder).toBe(report);
    await click(await trash());
    await click(await answer("Delete"));
    await waitFor(() => expect(folder).toBeNull());
  }, 30_000);
});

describe("a course file", () => {
  async function openPreview(
    url: string,
    handle: (command: string, args: Record<string, unknown>) => unknown,
    props: {
      openPage?: (page: unknown) => void;
      openWeb?: (url: string) => void;
      openFile?: (url: string, name: string) => void;
    } = {},
  ) {
    const core = await installComponentHost({
      workspace: workspaceFixture(),
      handle: (command, args) => handle(command, args),
    });
    const { FilePreview } = await import("@app/features/lms/FilePreview");
    const mounted = await view(
      <FilePreview
        url={url}
        title="File"
        courseId={COURSE_ID}
        limit={LIMIT}
        openPage={props.openPage ?? (() => undefined)}
        openWeb={props.openWeb ?? (() => undefined)}
        openFile={props.openFile}
      />,
    );
    return { core, mounted, host: mounted.host };
  }

  it("previews what Core prepared: past the download limit only on request, a ZIP's files each on its own, a protected PDF once unlocked", async () => {
    // Core reports a file over the request's limit by its size; only a
    // request without one prepares it.
    const big = "https://lms.s.isct.ac.jp/2025/pluginfile.php/1/big.txt";
    const text = await openPreview(big, (command, args) =>
      command !== "open_course_material"
        ? undefined
        : args.limit != null
          ? { state: "tooLarge", bytes: 45 * 1024 * 1024, page: lmsPage(big) }
          : {
              state: "ready",
              file: storedFile("/course/big.txt", "text", "Chapter 1"),
              page: lmsPage(big),
            },
    );
    await click(
      await waitFor(() =>
        required(buttonStarting(text.host, "Preview"), "the size card"),
      ),
    );
    // Prepared on request, the copy Core stored is the one revealed.
    await click(
      await waitFor(() =>
        required(buttonByText(text.host, "Show in folder"), "the preview"),
      ),
    );
    await waitFor(() =>
      expect(text.core.commands("open_course_file").at(-1)?.args).toMatchObject(
        { path: "/course/big.txt", reveal: true },
      ),
    );
    await release(text.mounted);

    // A ZIP lists its files; each opens in the reader as a course file.
    const zip =
      "https://lms.s.isct.ac.jp/2025/pluginfile.php/1/OOP-2A-problem.zip";
    const makefile = `${zip}#entry=OOP-2A/Makefile`;
    const entries: [string, string][] = [];
    const archive = await openPreview(
      zip,
      (command) =>
        command === "open_course_material"
          ? {
              state: "archive",
              file: storedFile(
                "/course/OOP-2A-problem-0011aabb.zip",
                "external",
              ),
              files: [
                {
                  name: "OOP-2A/README.md",
                  url: `${zip}#entry=OOP-2A/README.md`,
                  bytes: 2048,
                },
                { name: "OOP-2A/Makefile", url: makefile, bytes: 4 },
              ],
              page: lmsPage(zip),
            }
          : undefined,
      { openFile: (url, name) => entries.push([url, name]) },
    );
    await click(await waitFor(() => itemRow(archive.host, "Makefile")));
    expect(entries).toEqual([[makefile, "OOP-2A/Makefile"]]);
    await release(archive.mounted);

    // A protected PDF asks for its password, again after a wrong one, and
    // opens with the one the person typed.
    pdfjs.load.mockReset();
    pdfjs.render.mockReset();
    pdfjs.render.mockReturnValue({
      promise: Promise.resolve(),
      cancel: () => {},
    });
    let opened: (doc: unknown) => void = () => undefined;
    const task: {
      promise: Promise<unknown>;
      destroy: () => Promise<void>;
      onPassword?: (unlock: (password: string) => void, reason: number) => void;
    } = {
      promise: new Promise((resolve) => (opened = resolve)),
      destroy: async () => undefined,
    };
    pdfjs.load.mockReturnValue(task);
    const exam = "https://lms.s.isct.ac.jp/2025/pluginfile.php/1/exam.pdf";
    const pdf = await openPreview(exam, (command) =>
      command === "open_course_material"
        ? {
            state: "ready",
            file: storedFile("/course/exam-1a2b3c4d.pdf", "pdf"),
            page: lmsPage(exam),
          }
        : undefined,
    );
    await waitFor(() => expect(task.onPassword).toBeTypeOf("function"));
    const unlock = vi.fn();
    const field = () =>
      waitFor(() =>
        required(
          pdf.host.querySelector<HTMLInputElement>(".lmsPdfPassword input"),
          "the password field",
        ),
      );
    await act(async () => task.onPassword!(unlock, 1));
    await typeInto(await field(), "wrong");
    await click(buttonByText(pdf.host, "Open"));
    await act(async () => task.onPassword!(unlock, 2));
    await typeInto(await field(), "secret");
    await click(buttonByText(pdf.host, "Open"));
    expect(unlock).toHaveBeenLastCalledWith("secret");
    await act(async () =>
      opened({ numPages: 1, getPage: async () => sheet() }),
    );
    await waitFor(() =>
      expect(pdf.host.querySelector("canvas")).not.toBeNull(),
    );
  }, 30_000);

  it("sends a file Core cannot show here where Core says it belongs", async () => {
    const source = "https://lms.s.isct.ac.jp/2025/mod/resource/view.php?id=42";
    // A download that failed only this time is asked for again.
    let failures = 1;
    const flaky = await openPreview(source, (command) => {
      if (command !== "open_course_material") return undefined;
      if (failures-- > 0)
        return { state: "failed", reason: "network", page: lmsPage(source) };
      return {
        state: "ready",
        file: storedFile("/course/handout.txt", "text", "Handout"),
        page: lmsPage(source),
      };
    });
    await click(
      await waitFor(() => required(buttonByText(flaky.host, "Retry"), "Retry")),
    );
    await waitFor(() =>
      required(buttonByText(flaky.host, "Show in folder"), "the prepared file"),
    );
    await release(flaky.mounted);

    // Signing in, or a source Core could not resolve, happens on the LMS page
    // Core names.
    for (const [reason, label] of [
      ["login", "Sign in to LMS"],
      ["unresolved", "Open in LMS"],
    ] as const) {
      const page = lmsPage(source);
      const pages: unknown[] = [];
      const failed = await openPreview(
        source,
        (command) =>
          command === "open_course_material"
            ? { state: "failed", reason, page }
            : undefined,
        { openPage: (next) => pages.push(next) },
      );
      await click(
        await waitFor(() => required(buttonByText(failed.host, label), label)),
      );
      expect(pages).toEqual([page]);
      await release(failed.mounted);
    }

    // A page outside the school sites goes to the reader's web view.
    const outside = "https://example.org/lecture-video";
    const web: string[] = [];
    await openPreview(
      outside,
      (command) =>
        command === "open_course_material"
          ? { state: "external", url: outside }
          : undefined,
      { openWeb: (url) => web.push(url) },
    );
    await waitFor(() => expect(web).toEqual([outside]));
  }, 30_000);
});

describe("the LMS view in the reader", () => {
  it("follows Core's page: the newest revision, a return when shown again, and a popup a later reader finds", async () => {
    const forum = "https://lms.s.isct.ac.jp/2025/mod/forum/view.php?id=7";
    const page = {
      site: "science-lms",
      revision: 3,
      url: forum,
      popups: 0,
      back: "no",
      forward: "unknown",
      notice: null,
      access: null,
      signIn: false,
      outside: false,
      otp: null,
    };
    let snapshot: Record<string, unknown> = page;
    const core = await installComponentHost({
      workspace: workspaceFixture(),
      handle: (command, args) => {
        switch (command) {
          case "school_web_page":
            return snapshot;
          case "open_school_site":
            return {
              site: "science-lms",
              url: (args.entry as { url?: string }).url ?? forum,
              phase: "pageLoaded",
            };
          case "hide_school_web":
          case "set_school_web_bounds":
          case "school_web_back":
          case "close_school_web_popup":
          case "navigate_school_web":
            return true;
        }
        return undefined;
      },
    });
    const { OfficialLmsView } =
      await import("@app/features/lms/OfficialLmsView");
    const reader = (active: boolean) => (
      <OfficialLmsView url={forum} active={active} available />
    );
    const opens = () =>
      core.commands("open_school_site").map(({ args }) => args);
    const lastArgs = (command: string) => core.commands(command).at(-1)?.args;

    const first = await view(reader(true));
    const opened = await waitFor(() => required(opens()[0], "the LMS view"));
    expect(opened).toMatchObject({ entry: { kind: "go", url: forum } });
    const surface = opened.surface;

    // Of the snapshot and the events, the newest revision wins: Core now says
    // Back does something, and a late older page does not take that back.
    await waitFor(() =>
      expect(core.listenerCount("school-web-page")).toBeGreaterThan(0),
    );
    await act(async () => {
      core.emit("school-web-page", { ...page, revision: 5, back: "yes" });
      core.emit("school-web-page", { ...page, revision: 4 });
    });
    await click(first.host.querySelector('button[aria-label="Previous page"]'));
    await waitFor(() =>
      expect(lastArgs("school_web_back")).toMatchObject({
        site: "science-lms",
      }),
    );

    // Covered and shown again, the reader asks Core to return to its page.
    await act(async () => first.root.render(reader(false)));
    await act(async () => first.root.render(reader(true)));
    await waitFor(() =>
      expect(opens().at(-1)).toMatchObject({
        surface,
        entry: { kind: "return", url: forum },
      }),
    );
    await release(first);

    // A reader opened after a popup and a failed load reads both from the
    // snapshot; no event tells it.
    snapshot = {
      ...page,
      revision: 9,
      popups: 1,
      back: "yes",
      notice: { url: forum, reason: "unreachable" },
    };
    const later = await view(reader(true));
    await waitFor(() =>
      expect(opens().at(-1)).toMatchObject({
        entry: { kind: "go", url: forum },
      }),
    );
    expect(opens().at(-1)?.surface).not.toBe(surface);
    await click(
      await waitFor(() =>
        required(buttonByText(later.host, "Close window"), "the popup line"),
      ),
    );
    await waitFor(() =>
      expect(lastArgs("close_school_web_popup")).toMatchObject({
        site: "science-lms",
      }),
    );
    await click(
      required(buttonByText(later.host, "Retry"), "the failure line"),
    );
    await waitFor(() =>
      expect(lastArgs("navigate_school_web")).toMatchObject({
        site: "science-lms",
        url: forum,
      }),
    );
  }, 30_000);
});

describe("a course note", () => {
  it("keeps every draft: not saved on open, through a hidden editor, a failed save and a delete", async () => {
    const original =
      "# Notes\n\nInline $E=mc^2$\n\n$$\nx^2+y^2=z^2\n$$\n\n- First\n- Second\n\n```js\nconst a = 1;\n```\n";
    // The note's file on disk and the deleted-notes store, as the host keeps
    // them. A held save lands only when the test releases it.
    const disk = {
      text: original as string | null,
      deleted: null as string | null,
      failing: false,
      holding: false,
    };
    const held: (() => void)[] = [];
    const file = () => ({
      id: "n1",
      section: CONTEXT.section,
      name: "Note",
      path: "note",
      bytes: 7,
      cached: true,
      viewer: "markdown",
    });
    await installComponentHost({
      workspace: workspaceFixture(),
      handle: (command, args) => {
        if (command === "read_course_note")
          return { ...file(), text: disk.text };
        if (command === "save_course_note") {
          const text = String(args.text);
          if (disk.failing)
            return Promise.reject(new Error("Disk unavailable"));
          if (disk.holding)
            return new Promise((resolve) =>
              held.push(() => {
                disk.text = text;
                resolve(file());
              }),
            );
          disk.text = text;
          return file();
        }
        if (command === "move_course_note") {
          if (!args.restore) [disk.deleted, disk.text] = [disk.text, null];
          return null;
        }
        return undefined;
      },
    });
    const { NoteEditor } = await import("@app/features/lms/NoteEditor");
    // The editor the note really renders: drive its ProseMirror transactions.
    const { Editor, editorViewCtx } = await import("@milkdown/kit/core");
    const editors: InstanceType<typeof Editor>[] = [];
    const make = Editor.make.bind(Editor);
    vi.spyOn(Editor, "make").mockImplementation(() => {
      const editor = make();
      editors.push(editor);
      return editor;
    });
    const note = (mode: "visible" | "hidden") => (
      <Activity mode={mode}>
        <NoteEditor
          context={CONTEXT}
          id="n1"
          onSaved={() => {}}
          onDeleted={() => {}}
        />
      </Activity>
    );
    const mounted = await view(note("visible"));
    const { host, root } = mounted;
    // A hidden editor is torn down; each showing builds the next one.
    const editorReady = async (count: number) => {
      await waitFor(() => expect(editors.length).toBeGreaterThanOrEqual(count));
      await waitFor(() =>
        expect(host.querySelector(".ProseMirror")).not.toBeNull(),
      );
    };
    const edit = (text: string) =>
      act(async () => {
        editors.at(-1)!.action((ctx) => {
          const prose = ctx.get(editorViewCtx);
          prose.dispatch(
            prose.state.tr.insertText(text, prose.state.doc.content.size),
          );
        });
      });
    const hideAndShow = async () => {
      await act(async () => root.render(note("hidden")));
      await act(async () => root.render(note("visible")));
    };
    const recovery = () =>
      (
        JSON.parse(localStorage.getItem("lms-note-recovery:n1") ?? "null") as {
          text: string;
        } | null
      )?.text ?? null;

    // Opening is not an edit: hidden and shown again, the file is untouched.
    await editorReady(1);
    await hideAndShow();
    await editorReady(2);
    expect(disk.text).toBe(original);

    // A save still pending when the editor hides does not cost what is typed
    // after it shows again: the note is not read back over the live draft.
    disk.holding = true;
    await edit(" before hiding");
    await hideAndShow();
    await editorReady(3);
    await edit(" and more");
    disk.holding = false;
    await act(async () => {
      for (const land of held.splice(0)) land();
    });
    await click(buttonByText(host, "Save"));
    await waitFor(() => expect(disk.text).toContain("and more"));
    // Formulas, lists and code come back as the Markdown they were.
    for (const kept of [
      "before hiding",
      "$E=mc^2$",
      "x^2+y^2=z^2",
      "```js",
      "Second",
    ])
      expect(disk.text).toContain(kept);

    // A failed save keeps a recovery draft until a retry lands.
    disk.failing = true;
    await edit(" keep me");
    await click(buttonByText(host, "Save"));
    const retry = await waitFor(() =>
      required(buttonByText(host, "Retry save"), "the failed save"),
    );
    expect(disk.text).not.toContain("keep me");
    expect(recovery()).toContain("keep me");
    disk.failing = false;
    await click(retry);
    await waitFor(() => expect(disk.text).toContain("keep me"));
    await waitFor(() => expect(recovery()).toBeNull());

    // Deleting saves the last draft first; closing afterwards writes nothing
    // back.
    await edit(" last draft");
    await click(buttonByText(host, "Delete note"));
    await waitFor(() => expect(disk.deleted).toContain("last draft"));
    expect(disk.deleted).toContain("keep me");
    await release(mounted);
    await advance(50);
    expect(disk.text).toBeNull();
  }, 60_000);
});

describe("a PDF in a retained reader", () => {
  it("closes its PDF.js document while hidden and draws again from a fresh one when shown", async () => {
    pdfjs.load.mockReset();
    pdfjs.render.mockReset();
    // The first page is still drawing when the reader hides.
    pdfjs.render
      .mockReturnValueOnce({ promise: new Promise(() => {}), cancel: () => {} })
      .mockReturnValue({ promise: Promise.resolve(), cancel: () => {} });
    // PDF.js documents the viewer holds open; a destroyed one throws
    // synchronously when its transport is reused.
    let open = 0;
    pdfjs.load.mockImplementation(() => {
      let destroyed = false;
      open += 1;
      return {
        promise: Promise.resolve({
          numPages: 1,
          getPage: () => {
            if (destroyed)
              throw new TypeError(
                "Cannot read properties of null (reading 'sendWithPromise')",
              );
            return Promise.resolve(sheet());
          },
        }),
        destroy: async () => {
          if (!destroyed) open -= 1;
          destroyed = true;
        },
      };
    });
    await installComponentHost({ workspace: workspaceFixture() });
    const { PdfDocument } = await import("@app/components/files/PdfDocument");
    function RetainedPdf() {
      const [visible, setVisible] = useState(true);
      return (
        <>
          <button onClick={() => setVisible(!visible)}>Switch page</button>
          <Activity mode={visible ? "visible" : "hidden"}>
            <PdfDocument src="asset://localhost/a.pdf" title="Lecture" />
          </Activity>
        </>
      );
    }
    const pdf = await view(<RetainedPdf />);
    await waitFor(() => expect(pdfjs.render).toHaveBeenCalled());

    await click(buttonByText(pdf.host, "Switch page"));
    expect(open).toBe(0);

    await click(buttonByText(pdf.host, "Switch page"));
    await waitFor(() =>
      expect(pdf.host.querySelector("canvas")).not.toBeNull(),
    );
    expect(open).toBe(1);

    await release(pdf);
    expect(open).toBe(0);
  }, 30_000);
});

describe("course HTML", () => {
  it("renders inert: active content and unsafe addresses never reach the page", async () => {
    await installComponentHost({ workspace: workspaceFixture() });
    const { RichContent } = await import("@app/features/lms/RichContent");
    const safe = "https://lms.s.isct.ac.jp/2025/";
    const opened: string[] = [];
    const { host } = await view(
      <RichContent
        html={`<script>steal()</script><form>submit</form><iframe src="https://evil.example/"></iframe><img src="x" onerror="steal()"><p onclick="steal()">Text<a href="javascript:steal()">bad</a><a href="${safe}">good</a></p>`}
        files={{ courseId: COURSE_ID }}
        openLink={(url) => opened.push(url)}
      />,
    );

    expect(
      host.querySelector("script, form, iframe, [onclick], [onerror]"),
    ).toBeNull();
    expect(host.innerHTML).not.toContain("javascript:");
    // Every link a person can click asks the shell to open a safe address only.
    for (const link of host.querySelectorAll("a")) await click(link);
    expect(opened).toEqual([safe]);
  }, 30_000);
});
