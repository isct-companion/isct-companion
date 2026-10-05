// @vitest-environment jsdom
import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { Editor, editorViewCtx } from "@milkdown/kit/core";
import { MarkdownDocument } from "./MarkdownDocument";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
// jsdom has no layout observers; editor transactions remain real.
vi.stubGlobal(
  "IntersectionObserver",
  class {
    observe() {}
    unobserve() {}
    disconnect() {}
  },
);
vi.stubGlobal(
  "ResizeObserver",
  class {
    observe() {}
    unobserve() {}
    disconnect() {}
  },
);
const instances: Editor[] = [];
const make = Editor.make.bind(Editor);
vi.spyOn(Editor, "make").mockImplementation(() => {
  const editor = make();
  instances.push(editor);
  return editor;
});
let root: Root;
let host: HTMLDivElement;
afterEach(async () => {
  if (root) await act(async () => root.unmount());
  host?.remove();
  await vi.waitFor(() => expect(instances[0]?.status).toBe("Destroyed"));
  instances.splice(0);
});
async function mount(value: string, onChange?: (value: string) => void) {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await act(async () =>
    root.render(<MarkdownDocument value={value} onChange={onChange} />),
  );
  await vi.waitFor(() =>
    expect(host.querySelector(".ProseMirror")).not.toBeNull(),
  );
  await vi.waitFor(() =>
    expect({
      status: instances[0].status,
      error: host.querySelector("[role=alert]")?.textContent,
    }).toEqual({ status: "Created", error: undefined }),
  );
  return instances[0];
}
it("keeps formulas, lists and code through an edit, without saving on load or losing the last keystroke", async () => {
  const source =
    "# Notes\n\nInline $E=mc^2$\n\n$$\nx^2+y^2=z^2\n$$\n\n- First\n- Second\n\n```js\nconst a = 1;\n```\n";
  const change = vi.fn();
  const editor = await mount(source, change);
  expect(change).not.toHaveBeenCalled();
  await act(async () =>
    editor.action((ctx) => {
      const view = ctx.get(editorViewCtx);
      view.dispatch(view.state.tr.insertText(" updated", 7));
    }),
  );
  expect(change).toHaveBeenCalledTimes(1);
  const saved = change.mock.calls[0][0];
  expect(saved).toContain("updated");
  expect(saved).toContain("$E=mc^2$");
  expect(saved).toContain("x^2+y^2=z^2");
  expect(saved).toContain("```js");
  expect(saved).toContain("Second");
  await act(async () =>
    root.render(<MarkdownDocument value={saved} onChange={change} />),
  );
  expect(instances[0]).toBe(editor);
  expect(change).toHaveBeenCalledTimes(1);
  await act(async () => root.unmount());
  expect(change).toHaveBeenCalledTimes(1);
});
it("renders attachments read-only and applies external content without emitting edits", async () => {
  const editor = await mount("# Attachment\n\nRead me");
  expect(editor.action((ctx) => ctx.get(editorViewCtx).editable)).toBe(false);
  await act(async () =>
    root.render(<MarkdownDocument value="# Replacement" />),
  );
  expect(host.textContent).toContain("Replacement");
  expect(instances[0]).toBe(editor);
});
