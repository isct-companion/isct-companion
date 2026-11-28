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
  COURSE_FILE_POLICY,
  downloadLimit,
  retentionDays,
} from "@app/lib/course-file-policy";
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
} from "@app/lib/file-types";
import type { MatrixCaptureMetrics } from "@app/generated/persistence";
import { textLanguage } from "@app/components/files/TextDocument";
import {
  gravityTarget,
  springStep,
  stiffnessFor,
} from "@app/features/lms/spring-scroll";
import {
  MATRIX_SIZE,
  normalizeMatrixText,
  prepareMatrixImage,
  recognizeMatrixImage,
} from "@app/lib/matrix-ocr";

type FilesModule = typeof import("@app/features/lms/files");
type AssignmentsModule =
  typeof import("@app/features/lms/assignment-workspace");

/**
 * `platform/native.ts` captures `getDesktopBridge()` when it is first imported, so
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
  it("reads the extension from an LMS file address", () => {
    expect(extension(lms("%E8%AC%9B%E7%BE%A9.PDF"))).toBe("pdf");
    expect(extension(lms("README"))).toBe("");
    expect(extension(lms("lecture.HEIC"))).toBe("heic");
    expect(extension(lms("talk.M4A"))).toBe("m4a");
    // A name that cannot be parsed as a URL is still read as a path.
    expect(extension("notes.MD")).toBe("md");
    // A suffix longer than eight characters is not an extension.
    expect(
      extension("https://lms.s.isct.ac.jp/files/document.verylongextension"),
    ).toBe("");
  });

  it("previews images, media, documents and source text, and downloads the rest", () => {
    expect(previewKind(lms("photo.HEIC"))).toBe("image");
    expect(previewKind(lms("lecture.mp4"))).toBe("video");
    expect(previewKind(lms("talk.m4a"))).toBe("audio");
    expect(previewKind(lms("main.cpp"))).toBe("text");
    expect(previewKind(lms("slides.pdf"))).toBe("pdf");
    expect(previewKind("notes.md")).toBe("markdown");
    expect(previewKind(lms("code.zip"))).toBe("archive");
    expect(previewKind(lms("slides.pptx"))).toBeUndefined();
    expect(previewKind(lms("notebook.ipynb"))).toBeUndefined();
    expect(previewKind(lms("page.html"))).toBeUndefined();
    // Core reads ZIP only; other archives download.
    expect(previewKind(lms("code.7z"))).toBeUndefined();
  });

  it("recognizes school code attachments and highlights their language", () => {
    const plain = textLanguage("notes-1a2b3c4d.unknown").name;
    for (const ext of ["c", "cpp", "java", "py", "R", "txt"]) {
      const url = `https://lms.s.isct.ac.jp/2025/pluginfile.php/1/lecture.${ext}?forcedownload=1`;
      expect(previewKind(url), ext).toBe("text");
    }
    for (const ext of ["tex", "rs", "json", "ts", "sh", "sql", "yaml", "go"]) {
      expect(previewKind(lms(`file.${ext}`)), ext).toBe("text");
    }
    // The reader names the language from the downloaded file's name.
    const named = ["c", "java", "py", "R"].map(
      (ext) => textLanguage(`lecture-1a2b3c4d.${ext}`).name,
    );
    expect(new Set([...named, plain]).size).toBe(5);
    expect(textLanguage("lecture-1a2b3c4d.cpp").name).toBe(named[0]);
    expect(textLanguage("lecture-1a2b3c4d.txt").name).toBe(plain);
    expect(textLanguage("notes-1a2b3c4d.md").name).toBe(plain);
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
  const course = "course-1";

  // `lms_protocol.rs` reads exactly these query keys.
  it("puts the source and course on the app scheme, in the form each webview requires", () => {
    expect(files.lmsFileUrl(source, course, { os: "macos" })).toBe(
      "lmsfile://localhost/get?u=https%3A%2F%2Flms.s.isct.ac.jp%2F2025%2Fmod%2Fresource%2Fview.php%3Fid%3D42&course=course-1",
    );
    expect(
      files.lmsFileUrl(source, course, { os: "macos", download: true }),
    ).toContain("download=1");
    expect(
      files.lmsFileUrl(source, course, { os: "macos", foreground: true }),
    ).toContain("foreground=1");
    // Windows needs the http form, read from the injected native OS.
    window.__ISCT_COMPANION_NATIVE_OS__ = "windows";
    try {
      expect(files.lmsFileUrl(source, course)).toMatch(
        /^http:\/\/lmsfile\.localhost\/get\?/,
      );
    } finally {
      delete window.__ISCT_COMPANION_NATIVE_OS__;
    }
  });
});

// ---------------------------------------------------------------------------
// Imported policy values
// ---------------------------------------------------------------------------

describe("course file policy bounds", () => {
  const MB = 1024 * 1024;
  const { autoDownloadLimitMb: download, retentionDays: retention } =
    COURSE_FILE_POLICY;

  it("keeps every limit a usable control with the default inside its range", () => {
    for (const [name, policy] of Object.entries(COURSE_FILE_POLICY)) {
      expect(Number.isInteger(policy.min), name).toBe(true);
      expect(Number.isInteger(policy.default), name).toBe(true);
      expect(Number.isInteger(policy.max), name).toBe(true);
      expect(policy.min, name).toBeGreaterThanOrEqual(1);
      expect(policy.default, name).toBeGreaterThanOrEqual(policy.min);
      expect(policy.default, name).toBeLessThanOrEqual(policy.max);
    }
  });

  it("rounds and clamps imported values into the policy range, defaulting when unset", () => {
    expect(downloadLimit({})).toBe(download.default * MB);
    expect(downloadLimit({ autoDownloadLimitMb: Number.NaN })).toBe(
      download.default * MB,
    );
    expect(downloadLimit({ autoDownloadLimitMb: download.min - 1 })).toBe(
      download.min * MB,
    );
    expect(downloadLimit({ autoDownloadLimitMb: download.max + 1 })).toBe(
      download.max * MB,
    );
    expect(downloadLimit({ autoDownloadLimitMb: download.min + 1.6 })).toBe(
      (download.min + 2) * MB,
    );
    expect(retentionDays({})).toBe(retention.default);
    expect(retentionDays({ autoCleanDays: Number.NaN })).toBe(
      retention.default,
    );
    expect(retentionDays({ autoCleanDays: retention.min - 1 })).toBe(
      retention.min,
    );
    expect(retentionDays({ autoCleanDays: retention.max + 1 })).toBe(
      retention.max,
    );
    expect(retentionDays({ autoCleanDays: retention.min + 0.6 })).toBe(
      retention.min + 1,
    );
  });
});

// ---------------------------------------------------------------------------
// Note persistence
// ---------------------------------------------------------------------------

const noteContext = { courseId: "course-1", section: "section-91" };

/** A move the host performed, with what it had stored at that moment. */
type NoteMove = { id: string; restore: boolean; committed: string | null };

type NoteHost = {
  core: FakeCore;
  /** What the host has durably stored. */
  committed(): string | null;
  /** Every move the host performed, in arrival order. */
  moves(): NoteMove[];
  /** Completes the first save the host is holding. */
  release(): void;
};

/**
 * A scripted host that holds the first `save_course_note` until `release()`,
 * so the suite can see what the shell does while a write is still in flight.
 */
function noteHost(): NoteHost {
  const state = {
    holding: true,
    committed: null as string | null,
    pending: [] as (() => void)[],
    moves: [] as NoteMove[],
  };
  const core = installFakeCore({
    workspace: workspaceFixture(),
    handle: (command, args) => {
      if (command === "save_course_note") {
        const text = String(args.text);
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
      if (command === "move_course_note") {
        state.moves.push({
          id: String(args.id),
          restore: Boolean(args.restore),
          committed: state.committed,
        });
        return Promise.resolve(true);
      }
      return undefined;
    },
  });
  return {
    core,
    committed: () => state.committed,
    moves: () => state.moves,
    release: () => {
      for (const commit of state.pending.splice(0)) commit();
    },
  };
}

describe("note persistence", () => {
  it("lands notes in order, holding a later draft and a reopen until the pending save lands", async () => {
    const host = noteHost();
    const first = files.saveNote(noteContext, "ordered-note", "old");
    const second = files.saveNote(noteContext, "ordered-note", "latest");
    const reopened = files.readNote(noteContext, "ordered-note");

    await flush();
    host.release();
    const [, , reopenedFile] = await Promise.all([first, second, reopened]);
    // A later draft that overtook the held save would be overwritten by it.
    expect(host.committed()).toBe("latest");
    // The reopen waited for the write, so it reads the newest draft.
    expect(reopenedFile.text).toBe("latest");
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
    expect(host.moves()).toEqual([]);
    host.release();
    await Promise.all([saving, moving]);
    await files.moveNote(noteContext, "delete-ordered", true);
    expect(host.moves()).toEqual([
      { id: "delete-ordered", restore: false, committed: "latest draft" },
      { id: "delete-ordered", restore: true, committed: "latest draft" },
    ]);
  });
});

// ---------------------------------------------------------------------------
// Outline scrolling
// ---------------------------------------------------------------------------

describe("outline scroll gravity", () => {
  it("settles a section boundary at the top or bottom edge and leaves the rest alone", () => {
    const height = 500;
    const boundaries = [1000, 1600];
    // A boundary in the top band comes to the top: just above the outline is
    // drawn back down; just below it is pulled up.
    expect(gravityTarget(940, height, boundaries)).toBe(1000);
    expect(gravityTarget(1050, height, boundaries)).toBe(1000);
    // A section end in the bottom band comes back so the old section ends at the bottom.
    expect(gravityTarget(560, height, boundaries)).toBe(500);
    // The middle of a section, and an outline already on a boundary, stay put.
    expect(gravityTarget(1250, height, boundaries)).toBeUndefined();
    expect(gravityTarget(750, height, boundaries)).toBeUndefined();
    expect(gravityTarget(1000, height, boundaries)).toBeUndefined();
    // A start near the top wins over the next start near the bottom.
    expect(gravityTarget(940, height, [1000, 1400])).toBe(1000);
    // The settled position stays inside the scrollable range.
    expect(gravityTarget(940, height, boundaries, 1300)).toBe(800);
  });
});

// ---------------------------------------------------------------------------
// Spring scroll
// ---------------------------------------------------------------------------

describe("spring scroll", () => {
  it("arrives within about a third of a second without overshooting visibly, softer for long jumps", () => {
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
    // Long jumps are softer.
    expect(stiffnessFor(3000)).toBeLessThan(stiffnessFor(300));
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

describe("matrix text", () => {
  it("normalizes a manual paste to the letters it contains, capped at the Matrix size", () => {
    expect(normalizeMatrixText("a, b 0 1 8")).toBe("AB");
    expect(normalizeMatrixText("a".repeat(80))).toHaveLength(MATRIX_SIZE);
    expect(normalizeMatrixText("a".repeat(80))).toBe("A".repeat(MATRIX_SIZE));
  });
});

describe("matrix image preparation", () => {
  it("passes supported image bytes through and converts HEIC by contents to JPEG", async () => {
    await expect(
      prepareMatrixImage(new File(["png"], "photo.png")),
    ).resolves.toBe("cG5n");
    // The codec echoes the format it was asked for, so the assertion reads the
    // production function's output rather than the codec's arguments.
    codec.heicTo = async (args) =>
      new Blob([`jpeg:${args.type}:${args.quality}`], { type: args.type });
    const converted = await prepareMatrixImage(heicFile("photo.heic"));
    expect(window.atob(converted)).toMatch(/^jpeg:image\/jpeg:/);
  });

  it("rejects empty, oversized, undecodable and unreadable input without codec details", async () => {
    const tooBig = 20 * 1024 * 1024 + 1;
    await expect(
      prepareMatrixImage(new File([], "empty.png")),
    ).rejects.toThrowError();
    await expect(
      prepareMatrixImage(new File([new Uint8Array(tooBig)], "big.png")),
    ).rejects.toThrowError();
    // The real codec cannot decode a bare HEIC header.
    await expect(
      prepareMatrixImage(heicFile("photo.heic")),
    ).rejects.toThrowError();
    codec.heicTo = async () => new Blob([new Uint8Array(tooBig)]);
    await expect(
      prepareMatrixImage(heicFile("photo.heic")),
    ).rejects.toThrowError();
    const unreadable = Object.assign(new File(["bad"], "photo.bin"), {
      arrayBuffer: () => Promise.reject(new Error("parser detail")),
    });
    await expect(prepareMatrixImage(unreadable)).rejects.toThrowError(
      /^(?![\s\S]*parser detail)/,
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
    ).rejects.toThrowError();
    expect(core.calls).toEqual([]);
  });

  it("sends one raw image and forwards typed native progress", async () => {
    pretendDesktopApp(true);
    // What the recognizer found is opaque here; only its hand-off is checked.
    const recognitions: never[] = [];
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

    // The host received the raw bytes, not a re-encoded image.
    expect(core.commands("recognize_matrix_image")[0].args.image).toBe("cmF3");
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
    const draw = vi.fn((source: unknown) =>
      source === video ? currentTime : undefined,
    );
    const stroke = vi.fn();
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
      drawImage: draw,
      getImageData: (_x: number, _y: number, w: number, h: number) => ({
        data: new Uint8ClampedArray(w * h * 4).fill(200),
      }),
      beginPath() {},
      moveTo() {},
      lineTo() {},
      closePath() {},
      stroke,
    } as unknown as CanvasRenderingContext2D);
    const display = document.createElement("canvas");
    // Frames read from the camera, as opposed to repaints of the display.
    const reads = () =>
      draw.mock.calls
        .map((call, i) => [call[0], draw.mock.results[i].value])
        .filter(([source]) => source === video)
        .map(([, time]) => time);
    return {
      video,
      display,
      draw,
      reads,
      stroke,
      advance: (time: number) => (currentTime = time),
    };
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
    const { video, display, reads, stroke, advance } = cameraSurface(
      async () => stream,
    );
    const capturing = {
      phase: "capturing",
      sequence: 1,
      boundsFrameId: 0,
      keyframeFrameId: null as number | null,
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
      cells: [],
    };
    let deliver!: (snapshot: unknown) => void;
    let finish!: (snapshot: unknown) => void;
    const { core, channels } = installHostWithChannels({
      handle: (command) => {
        if (command === "begin_matrix_camera") return "camera-1";
        if (command === "poll_matrix_camera") return capturing;
        if (command === "track_matrix_camera_frame")
          return new Promise((resolve) => {
            deliver = resolve;
          });
        if (command === "capture_matrix_keyframe")
          return { ...capturing, sequence: 3, keyframeFrameId: null };
        if (command === "process_matrix_camera")
          return new Promise((resolve) => {
            finish = resolve;
          });
        return undefined;
      },
    });
    const onSnapshot = vi.fn(),
      onError = vi.fn();
    close = openMatrixCamera(video, display, onSnapshot, onError);
    await vi.waitFor(() =>
      expect(core.commands("track_matrix_camera_frame")).toHaveLength(1),
    );
    advance(2);
    await vi.advanceTimersByTimeAsync(100);
    advance(3);
    await vi.advanceTimersByTimeAsync(100);
    advance(4);
    await vi.advanceTimersByTimeAsync(100);
    expect(reads()).toEqual([1]);
    // Raw 960 x 540 luma and R-G planes; the frame is named in the headers.
    const first = core.commands("track_matrix_camera_frame")[0];
    expect((first.args as unknown as Uint8Array).length).toBe(960 * 540 * 2);
    expect(first.options?.headers).toMatchObject({
      "x-matrix-session": "camera-1",
      "x-matrix-frame": "0",
      "x-matrix-width": "960",
      "x-matrix-height": "540",
    });
    expect(onSnapshot).not.toHaveBeenCalled();
    // The reply's outline belongs to this frame: both are drawn together, and
    // Core's keyframe request sends the same frame at full resolution.
    deliver({ ...capturing, keyframeFrameId: 0 });
    await vi.advanceTimersByTimeAsync(1);
    expect(stroke).toHaveBeenCalledTimes(1);
    const keyframe = core.commands("capture_matrix_keyframe");
    expect(keyframe).toHaveLength(1);
    expect((keyframe[0].args as unknown as Uint8Array).length).toBe(
      1920 * 1080,
    );
    expect(keyframe[0].options?.headers).toMatchObject({
      "x-matrix-frame": "0",
      "x-matrix-width": "1920",
    });
    expect(core.commands("track_matrix_camera_frame")).toHaveLength(2);
    expect(reads()).toEqual([1, 4]);
    // An outline measured on an older frame is never drawn over this one.
    deliver({ ...capturing, sequence: 4, boundsFrameId: 0 });
    await vi.advanceTimersByTimeAsync(100);
    expect(stroke).toHaveBeenCalledTimes(1);
    expect(core.commands("capture_matrix_keyframe")).toHaveLength(1);
    expect(core.commands("track_matrix_camera_frame")).toHaveLength(2);
    expect(core.commands("poll_matrix_camera")).toHaveLength(1); // deadlines advance during a video stall
    advance(5);
    await vi.advanceTimersByTimeAsync(100);
    expect(core.commands("track_matrix_camera_frame")).toHaveLength(3);
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
    // Each reply is paired with its frame and the frame its outline came
    // from; a painted frame shows the picture that outline was fitted on.
    expect(
      (metrics as unknown as MatrixCaptureMetrics).timeline.map((e) => [
        e.frameId,
        e.boundsFrameId,
        e.sentMs >= e.capturedMs && e.respondedMs >= e.sentMs,
        e.paintedMediaMs === e.mediaMs,
      ]),
    ).toEqual([
      [0, 0, true, true],
      [1, 0, true, true],
      [2, 0, true, false],
    ]);
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
    const { video, display } = cameraSurface(acquire);
    let deliver!: (snapshot: unknown) => void;
    let finish!: (snapshot: unknown) => void;
    const core = installFakeCore({
      workspace: workspaceFixture(),
      handle: (command) => {
        if (command === "begin_matrix_camera") return "camera-2";
        if (command === "track_matrix_camera_frame")
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
    close = openMatrixCamera(video, display, onSnapshot, onError);
    await vi.waitFor(() => expect(acquire).toHaveBeenCalled());
    close();
    grant(stream);
    await vi.waitFor(() => expect(track.stop).toHaveBeenCalledTimes(1));
    expect(core.commands("track_matrix_camera_frame")).toHaveLength(0);
    close = openMatrixCamera(video, display, onSnapshot, onError);
    await vi.waitFor(() => expect(acquire).toHaveBeenCalledTimes(2));
    grant(stream);
    await vi.waitFor(() =>
      expect(core.commands("track_matrix_camera_frame")).toHaveLength(1),
    );
    close();
    deliver({ complete: true, cells: [] });
    await flush();
    expect(onSnapshot).not.toHaveBeenCalled();
    expect(onError).not.toHaveBeenCalled();
    expect(core.commands("stop_matrix_camera")).toHaveLength(2);
    close = openMatrixCamera(video, display, onSnapshot, onError);
    await vi.waitFor(() => expect(acquire).toHaveBeenCalledTimes(3));
    grant(stream);
    await vi.waitFor(() =>
      expect(core.commands("track_matrix_camera_frame")).toHaveLength(2),
    );
    deliver({
      phase: "processing",
      bounds: null,
      cells: [],
      readyCells: 70,
      completedSamples: 0,
      totalSamples: 210,
    });
    // Each started session runs its own consumer from the start.
    await vi.waitFor(() =>
      expect(core.commands("process_matrix_camera")).toHaveLength(2),
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
    const { video, display } = cameraSurface(async () => {
      throw new DOMException("denied", "NotAllowedError");
    });
    const core = installFakeCore({
      workspace: workspaceFixture(),
      handle: (command) =>
        command === "begin_matrix_camera" ? "denied" : undefined,
    });
    const onError = vi.fn();
    close = openMatrixCamera(video, display, vi.fn(), onError);
    await vi.waitFor(() => expect(onError).toHaveBeenCalled());
    expect(core.commands("stop_matrix_camera")).toHaveLength(1);
  });
});

describe("math delimiters in a read-only Markdown document", () => {
  async function normalize(lines: string[]) {
    const { readableMarkdown } =
      await import("@app/lib/markdown-math");
    return readableMarkdown(lines.join("\n")).split("\n");
  }

  it("turns \\( \\) into inline math and \\[ \\] into a display block", async () => {
    expect(
      await normalize([
        "The integral is",
        "\\[",
        "\\int_a^b f(x)\\,dx",
        "\\]",
        "so we get \\( F(b)-F(a) \\).",
      ]),
    ).toEqual([
      "The integral is",
      "",
      "$$",
      "\\int_a^b f(x)\\,dx",
      "$$",
      "",
      "so we get $F(b)-F(a)$.",
    ]);
  });

  it("sets a one-line $$…$$ as a block, indented inside a list item", async () => {
    expect(await normalize(["1. Solve", "   $$x^2 = 4$$"])).toEqual([
      "1. Solve",
      "",
      "   $$",
      "   x^2 = 4",
      "   $$",
    ]);
  });

  it("halves the backslashes of a formula written for Markdown to unescape", async () => {
    expect(
      await normalize([
        "$$t \\\\rightarrow t^{\\\\prime} := t$$",
        "",
        "$$",
        "x \\\\rightarrow x^{\\\\prime} \\\\\\\\",
        "v \\\\rightarrow v^{\\\\prime}",
        "$$",
        "",
        "where \\\\(\\\\alpha\\\\) and $\\\\beta_0$ are constants.",
        "",
        "\\\\[",
        "\\\\frac{a}{b}",
        "\\\\]",
      ]),
    ).toEqual([
      "$$",
      "t \\rightarrow t^{\\prime} := t",
      "$$",
      "",
      "$$",
      "\\begin{gathered}",
      "x \\rightarrow x^{\\prime} \\\\",
      "v \\rightarrow v^{\\prime}",
      "\\end{gathered}",
      "$$",
      "",
      "where $\\alpha$ and $\\beta_0$ are constants.",
      "",
      "$$",
      "\\frac{a}{b}",
      "$$",
    ]);
  });

  it("keeps a doubled \\\\ that starts a line as the line break it lays out", async () => {
    expect(
      await normalize([
        "$$",
        "t \\\\rightarrow t^{\\\\prime} := t",
        "\\\\x \\\\rightarrow x^{\\\\prime}(t^{\\\\prime}) := x(t) + x_0",
        "\\\\v \\\\rightarrow v^{\\\\prime}(t^{\\\\prime}) := v(t)",
        "$$",
      ]),
    ).toEqual([
      "$$",
      "\\begin{gathered}",
      "t \\rightarrow t^{\\prime} := t",
      "\\\\ x \\rightarrow x^{\\prime}(t^{\\prime}) := x(t) + x_0",
      "\\\\ v \\rightarrow v^{\\prime}(t^{\\prime}) := v(t)",
      "\\end{gathered}",
      "$$",
    ]);
  });

  it("drops a page's MathJax script and reads its doubled inline math", async () => {
    expect(
      await normalize([
        '<script type="text/javascript" async src="https://example.test/mathjax.js">',
        "</script>",
        '<script type="text/x-mathjax-config">',
        "MathJax.Hub.Config({ tex2jax: { inlineMath: [['$', '$']] } });",
        "</script>",
        "",
        "**Final 2026**",
        "",
        "* (4) is $$\\\\frac{d^2 x}{dt^2}-2a\\\\frac{dx}{dt}=te^{at}$$ now.",
        "* $\\\\bold{F_2}(\\\\bold{r})$ and $U_i$ swapped.",
      ]),
    ).toEqual([
      "",
      "**Final 2026**",
      "",
      "* (4) is $$\\frac{d^2 x}{dt^2}-2a\\frac{dx}{dt}=te^{at}$$ now.",
      "* $\\bold{F_2}(\\bold{r})$ and $U_i$ swapped.",
    ]);
  });

  it("leaves code, inline code, prices and existing math alone", async () => {
    const source = [
      "Inline $a \\(b\\)$ and `\\(code\\)`; it costs $5 and $10.",
      "",
      "```",
      "\\(in a fence\\)",
      "$$x$$",
      "```",
      "",
      "$$",
      "\\begin{aligned} a &= b \\\\[2pt] c &= d \\end{aligned}",
      "$$",
      "",
      "A literal \\\\(not math).",
    ];
    expect(await normalize(source)).toEqual(source);
  });
});

describe("the mobile Markdown page", () => {
  it("renders a shared note's math with KaTeX and shows its raw HTML as text", async () => {
    const { renderMarkdown } =
      await import("@app/features/markdown-viewer/viewer");
    const html = renderMarkdown(
      [
        "<script>alert(1)</script>",
        "",
        "# Note",
        "",
        "Speed \\\\(v\\\\) and $\\\\Delta t$; <b>bold?</b>",
        "",
        "$$",
        "t \\\\rightarrow t^{\\\\prime}",
        "\\\\x \\\\rightarrow x^{\\\\prime}",
        "$$",
        "",
        "| a | b |",
        "| - | - |",
        "| 1 | 2 |",
      ].join("\n"),
    );
    const page = new DOMParser().parseFromString(html, "text/html");
    expect(page.querySelector("script")).toBeNull();
    expect(page.querySelector("b")).toBeNull();
    expect(page.body.textContent).toContain("<b>bold?</b>");
    expect(page.querySelector("h1")?.textContent).toBe("Note");
    expect(page.querySelectorAll(".katex")).toHaveLength(3);
    expect(page.querySelectorAll(".katex-error")).toHaveLength(0);
    expect(
      page.querySelector(".katex-display annotation")?.textContent,
    ).toContain("x \\rightarrow x^{\\prime}");
    expect(page.querySelectorAll("td")).toHaveLength(2);
  });
});
