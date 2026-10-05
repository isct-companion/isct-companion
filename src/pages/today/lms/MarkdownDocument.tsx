import { useEffect, useRef, useState } from "react";
import { LanguageDescription, LanguageSupport, StreamLanguage } from "@codemirror/language";
import { CrepeBuilder } from "@milkdown/crepe/builder";
import { blockEdit } from "@milkdown/crepe/feature/block-edit";
import { codeMirror } from "@milkdown/crepe/feature/code-mirror";
import { latex } from "@milkdown/crepe/feature/latex";
import { listItem } from "@milkdown/crepe/feature/list-item";
import { linkTooltip } from "@milkdown/crepe/feature/link-tooltip";
import { table } from "@milkdown/crepe/feature/table";
import { toolbar } from "@milkdown/crepe/feature/toolbar";
import { editorViewOptionsCtx, serializerCtx } from "@milkdown/kit/core";
import { Plugin } from "@milkdown/kit/prose/state";
import { $prose, replaceAll } from "@milkdown/kit/utils";
import "@milkdown/crepe/theme/common/style.css";
import "@milkdown/crepe/theme/frame.css";
import "katex/dist/katex.min.css";

export function MarkdownDocument({
  value,
  onChange,
}: {
  value: string;
  onChange?: (value: string) => void;
}) {
  const root = useRef<HTMLDivElement>(null);
  const state = useRef({ value, onChange });
  state.current = { value, onChange };
  const current = useRef<{
    editor: CrepeBuilder;
    value: string;
    replacing: boolean;
  }>(null);
  const [error, setError] = useState<string>();
  const editable = Boolean(onChange);
  useEffect(() => {
    // A separate mount prevents a late async destroy from touching a replacement editor.
    const mount = document.createElement("div");
    root.current!.append(mount);
    const editor = new CrepeBuilder({
      root: mount,
      defaultValue: state.current.value,
    });
    const instance = { editor, value: state.current.value, replacing: false };
    let disposed = false;
    setError(undefined);
    editor.addFeature(codeMirror, {
      languages: [
        LanguageDescription.of({name:"C / C++",alias:["c","cpp","c++"],extensions:["c","cpp","h","hpp"],load:()=>import("@codemirror/lang-cpp").then(m=>m.cpp())}),
        LanguageDescription.of({name:"Java",alias:["java"],extensions:["java"],load:()=>import("@codemirror/lang-java").then(m=>m.java())}),
        LanguageDescription.of({name:"Python",alias:["py","python"],extensions:["py"],load:()=>import("@codemirror/lang-python").then(m=>m.python())}),
        LanguageDescription.of({name:"R",alias:["r"],extensions:["r","R"],load:()=>import("@codemirror/legacy-modes/mode/r").then(m=>new LanguageSupport(StreamLanguage.define(m.r)))}),
      ],
      previewOnlyByDefault: true,
    });
    editor.addFeature(latex, {
      katexOptions: { trust: false, strict: "warn" },
    });
    editor.addFeature(listItem).addFeature(table);
    if (editable)
      editor.addFeature(blockEdit).addFeature(toolbar).addFeature(linkTooltip);
    editor.setReadonly(!editable);
    editor.editor
      .config((ctx) => {
        ctx.update(editorViewOptionsCtx, (options) => ({
          ...options,
          attributes: {
            ...options.attributes,
            role: "textbox",
            "aria-label": "Note document",
            "aria-multiline": "true",
          },
        }));
      })
      .use(
        $prose(
          (ctx) =>
            new Plugin({
              view: () => ({
                update: (view, previous) => {
                  if (
                    disposed ||
                    instance.replacing ||
                    previous.doc.eq(view.state.doc) ||
                    !editable
                  )
                    return;
                  // Capture immediately: closing a tab must not lose the last keystroke.
                  const markdown = ctx.get(serializerCtx)(view.state.doc);
                  if (markdown === instance.value) return;
                  instance.value = markdown;
                  state.current.onChange?.(markdown);
                },
              }),
            }),
        ),
      );
    const ready = editor
      .create()
      .then(() => {
        if (!disposed) {
          current.current = instance;
          if (state.current.value !== instance.value) {
            instance.replacing = true;
            instance.value = state.current.value;
            try {
              editor.editor.action(replaceAll(instance.value));
            } finally {
              instance.replacing = false;
            }
          }
        }
      })
      .catch((reason) => {
        if (!disposed) setError(String(reason));
      });
    return () => {
      disposed = true;
      if (current.current === instance) current.current = null;
      mount.remove();
      void ready.then(() => editor.destroy()).catch(console.error);
    };
  }, [editable]);
  useEffect(() => {
    const instance = current.current;
    if (!instance || value === instance.value) return;
    instance.replacing = true;
    instance.value = value;
    try {
      instance.editor.editor.action(replaceAll(value));
    } finally {
      instance.replacing = false;
    }
  }, [value]);
  return (
    <div className="lmsMarkdownDocument" ref={root}>
      {error && <p role="alert">{error}</p>}
    </div>
  );
}
