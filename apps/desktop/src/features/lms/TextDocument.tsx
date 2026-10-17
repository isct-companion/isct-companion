import { useEffect, useRef } from "react";
import { EditorState } from "@codemirror/state";
import { EditorView, lineNumbers, highlightActiveLineGutter } from "@codemirror/view";
import { defaultHighlightStyle, syntaxHighlighting, StreamLanguage } from "@codemirror/language";
import { cpp } from "@codemirror/lang-cpp";
import { java } from "@codemirror/lang-java";
import { python } from "@codemirror/lang-python";
import { r } from "@codemirror/legacy-modes/mode/r";
import { t } from "../../i18n";
export function textLanguage(url: string) {
  const path = new URL(url).pathname.toLowerCase();
  if (/\.(c|h|cpp|cc|cxx|hpp)$/.test(path)) return { name: "C / C++", extension: cpp() };
  if (/\.java$/.test(path)) return { name: "Java", extension: java() };
  if (/\.py$/.test(path)) return { name: "Python", extension: python() };
  if (/\.r$/.test(path)) return { name: "R", extension: StreamLanguage.define(r) };
  return { name: "Plain text", extension: [] };
}
export function TextDocument({ value, url }: { value: string; url: string }) {
  const host = useRef<HTMLDivElement>(null);
  const language = textLanguage(url);
  const position = useRef({ top: 0, left: 0 });
  useEffect(() => {
    const view = new EditorView({ parent: host.current!, state: EditorState.create({ doc: value, extensions: [
      EditorState.readOnly.of(true), EditorView.editable.of(false), lineNumbers(), highlightActiveLineGutter(),
      language.extension, syntaxHighlighting(defaultHighlightStyle),
      EditorView.contentAttributes.of({ "aria-label": `${language.name} document`, tabindex: "0" }),
      EditorView.theme({ "&": { backgroundColor: "transparent", color: "var(--ink)", fontSize: "13px", padding: "16px 0" },
        ".cm-content": { fontFamily: '"SFMono-Regular", Consolas, "Liberation Mono", monospace', padding: "4px 0" },
        ".cm-line": { padding: "0 24px 0 16px" }, ".cm-gutters": { backgroundColor: "transparent", border: "none", color: "var(--muted)", padding: "0 8px" },
        ".cm-activeLineGutter": { backgroundColor: "transparent" }, ".cm-scroller": { overflow: "auto", fontFamily: '"SFMono-Regular", Consolas, "Liberation Mono", monospace', lineHeight: "1.85" },
        ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": { backgroundColor: "var(--accent-soft)" },
      }),
    ] }) });
    view.scrollDOM.scrollTop = position.current.top;
    view.scrollDOM.scrollLeft = position.current.left;
    return () => {
      position.current = { top: view.scrollDOM.scrollTop, left: view.scrollDOM.scrollLeft };
      view.destroy();
    };
  }, [value, url]);
  return <div className="lmsTextDocument"><div className="lmsTextMeta"><span>{t(language.name)}</span><span>{t("{{count}} lines", { count: value.split("\n").length })} · UTF-8</span></div><div ref={host} /></div>;
}
