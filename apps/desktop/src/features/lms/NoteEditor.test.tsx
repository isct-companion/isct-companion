// @vitest-environment jsdom
import React, { Activity, act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
const { saveNote, readNote, moveNote } = vi.hoisted(() => ({
  saveNote: vi.fn(),
  readNote: vi.fn(),
  moveNote: vi.fn(),
}));
vi.mock("./files", () => ({ saveNote, readNote, moveNote }));
vi.mock("./MarkdownDocument", () => ({
  MarkdownDocument: ({
    value,
    onChange,
  }: {
    value: string;
    onChange: (value: string) => void;
  }) => (
    <textarea
      aria-label="Note text"
      value={value}
      onChange={(event) => onChange(event.target.value)}
    />
  ),
}));
import { NoteEditor } from "./NoteEditor";
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement;
let root: Root;
const context = { term: "2026-3Q", course: "TEST", section: "section-91" };
beforeEach(() => {
  localStorage.clear();
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  readNote.mockResolvedValue({ text: "initial" });
  saveNote.mockReset().mockResolvedValue({ id: "n1" });
  moveNote.mockReset().mockResolvedValue(undefined);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.useRealTimers();
});
async function render() {
  await act(async () => {
    root.render(<NoteEditor context={context} id="n1" onSaved={() => {}} />);
    await vi.dynamicImportSettled();
  });
}
function edit(text: string) {
  act(() => {
    const input = host.querySelector("textarea")!;
    Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      "value",
    )!.set!.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
it("flushes the current draft when closed before autosave", async () => {
  await render();
  edit("current draft");
  await act(async () => root.unmount());
  expect(saveNote).toHaveBeenCalledWith(context, "n1", "current draft");
  expect(saveNote).toHaveBeenCalledTimes(1);
});
it("preserves a failed save and clears recovery after a successful retry", async () => {
  await render();
  saveNote.mockRejectedValueOnce(new Error("Disk unavailable"));
  edit("keep me");
  await act(async () =>
    [...host.querySelectorAll("button")]
      .find((button) => button.textContent === "Save")!
      .click(),
  );
  expect(JSON.parse(localStorage.getItem("lms-note-recovery:n1") ?? "null").text).toBe("keep me");
  expect(host.textContent).toContain("Disk unavailable");
  await act(async () =>
    [...host.querySelectorAll("button")]
      .find((button) => button.textContent === "Retry save")!
      .click(),
  );
  expect(localStorage.getItem("lms-note-recovery:n1")).toBeNull();
  expect(host.textContent).toContain("Saved locally");
});

it("preserves the final draft in deleted storage and does not resurrect it on unmount", async () => {
  const deleted = vi.fn();
  await act(async () => { root.render(<NoteEditor context={context} id="n1" onSaved={()=>{}} onDeleted={deleted}/>); await vi.dynamicImportSettled(); });
  edit("last draft before deletion");
  await act(async()=>[...host.querySelectorAll("button")].find(button=>button.textContent==="Delete note")!.click());
  expect(saveNote).toHaveBeenCalledWith(context,"n1","last draft before deletion");
  expect(moveNote).toHaveBeenCalledWith(context,"n1");
  expect(saveNote.mock.invocationCallOrder[0]).toBeLessThan(moveNote.mock.invocationCallOrder[0]);
  expect(deleted).toHaveBeenCalledOnce();
  await act(async()=>root.unmount());
  expect(saveNote).toHaveBeenCalledTimes(1);
});

it("retains the live draft across Activity hiding while the disk save is pending", async () => {
  readNote.mockClear();
  let finishSave!: (value: { id: string }) => void;
  saveNote.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finishSave = resolve;
      }),
  );
  const renderActivity = async (mode: "visible" | "hidden") => {
    await act(async () => {
      root.render(
        <Activity mode={mode}>
          <NoteEditor context={context} id="n1" onSaved={() => {}} />
        </Activity>,
      );
      await vi.dynamicImportSettled();
    });
  };
  await renderActivity("visible");
  edit("last keystroke before hiding");
  await renderActivity("hidden");
  expect(saveNote).toHaveBeenCalledWith(
    context,
    "n1",
    "last keystroke before hiding",
  );
  await renderActivity("visible");
  expect(host.querySelector("textarea")?.value).toBe(
    "last keystroke before hiding",
  );
  expect(readNote).toHaveBeenCalledTimes(1);
  await act(async () => finishSave({ id: "n1" }));
});
