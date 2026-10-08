import { beforeEach, describe, expect, it, vi } from "vitest";
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("../../../app/native", () => ({ desktopBridge: {} }));
vi.mock("@tauri-apps/api/core", () => ({ invoke, convertFileSrc: (path: string) => `asset://localhost${path}` }));
import {
  downloadLimit,
  folderComponent,
  lmsFileUrl,
  moveNote,
  previewKind,
  readNote,
  retentionDays,
  saveNote,
} from "./files";
const context = {
  term: "2026-3Q",
  course: "CSC.T251",
  section: "section-91",
};
beforeEach(() => invoke.mockReset());
describe("local course files", () => {
  it("builds the platform-specific LMS file address in one helper", () => {
    const source = "https://lms.s.isct.ac.jp/2025/mod/resource/view.php?id=42";
    expect(lmsFileUrl(source, context, { native: false })).toBe(source);
    expect(
      lmsFileUrl(source, context, { native: true, platform: "MacIntel" }),
    ).toBe(
      "lmsfile://localhost/get?u=https%3A%2F%2Flms.s.isct.ac.jp%2F2025%2Fmod%2Fresource%2Fview.php%3Fid%3D42&term=2026-3Q&course=CSC.T251",
    );
    expect(
      lmsFileUrl(source, context, { native: true, platform: "Windows" }),
    ).toMatch(/^http:\/\/lmsfile\.localhost\/get\?/);
    expect(
      lmsFileUrl(source, context, {
        native: true,
        platform: "Android",
        download: true,
      }),
    ).toContain("download=1");
    expect(
      lmsFileUrl(source, context, {
        native: true,
        platform: "MacIntel",
        foreground: true,
      }),
    ).toContain("foreground=1");
  });
  it("recognizes source files without executing them and bounds path components", () => {
    for (const extension of ["tex", "java", "py", "rs", "json", "txt"])
      expect(
        previewKind(
          `https://lms.s.isct.ac.jp/pluginfile.php/1/file.${extension}?forcedownload=1`,
        ),
      ).toBe("text");
    expect(previewKind("notes.md")).toBe("markdown");
    expect(previewKind("archive.zip")).toBeUndefined();
    expect(folderComponent("..")).toBe("course");
    expect(folderComponent("x/y\\z")).toBe("x_y_z");
  });
  it("serializes note writes and waits for pending save before reopening", async () => {
    let finish!: () => void;
    invoke
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finish = () => resolve({ id: "note" });
          }),
      )
      .mockResolvedValue({ id: "note", text: "latest" });
    const first = saveNote(context, "ordered-note", "old");
    const second = saveNote(context, "ordered-note", "latest");
    const reopened = readNote(context, "ordered-note");
    await Promise.resolve();
    await Promise.resolve();
    expect(invoke).toHaveBeenCalledTimes(1);
    finish();
    await Promise.all([first, second, reopened]);
    expect(invoke.mock.calls.map((call) => call[0])).toEqual([
      "save_course_note",
      "save_course_note",
      "read_course_note",
    ]);
    expect(invoke.mock.calls[1][1].text).toBe("latest");
  });
  it("rejects oversized drafts before sending native commands", async () => {
    await expect(
      saveNote(context, "large-note", "あ".repeat(400000)),
    ).rejects.toThrow("1 MB");
    expect(invoke).not.toHaveBeenCalled();
  });
  it("bounds imported JSON policy values before native commands", () => {
    expect(downloadLimit({})).toBe(20 * 1024 * 1024);
    expect(downloadLimit({ autoDownloadLimitMb: 0 })).toBe(1024 * 1024);
    expect(downloadLimit({ autoDownloadLimitMb: 2048 })).toBe(
      1024 * 1024 * 1024,
    );
    expect(retentionDays({})).toBe(30);
    expect(retentionDays({ autoCleanDays: 1.6 })).toBe(2);
    expect(retentionDays({ autoCleanDays: Number.NaN })).toBe(30);
  });
});

it("moves a note only after its pending autosave, then delegates undo to restore", async () => {
  let finish!: () => void;
  invoke
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = () => resolve({ id: "delete-ordered" });
        }),
    )
    .mockResolvedValue(true);
  const saving = saveNote(context, "delete-ordered", "latest draft");
  const moving = moveNote(context, "delete-ordered");
  await Promise.resolve();
  await Promise.resolve();
  expect(invoke).toHaveBeenCalledTimes(1);
  finish();
  await Promise.all([saving, moving]);
  expect(invoke.mock.calls.map((call) => call[0])).toEqual([
    "save_course_note",
    "move_course_note",
  ]);
  await moveNote(context, "delete-ordered", true);
  expect(invoke).toHaveBeenLastCalledWith("move_course_note", {
    context,
    id: "delete-ordered",
    restore: true,
  });
});
