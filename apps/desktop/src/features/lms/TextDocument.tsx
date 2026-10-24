import { useEffect, useRef, useState } from "react";
import { EditorState } from "@codemirror/state";
import { EditorView, lineNumbers, highlightActiveLineGutter, keymap, type Panel } from "@codemirror/view";
import {
  closeSearchPanel, findNext, findPrevious, getSearchQuery, openSearchPanel, search, searchKeymap,
  searchPanelOpen, SearchQuery, setSearchQuery,
} from "@codemirror/search";
import { defaultHighlightStyle, syntaxHighlighting, StreamLanguage } from "@codemirror/language";
import { cpp } from "@codemirror/lang-cpp";
import { java } from "@codemirror/lang-java";
import { python } from "@codemirror/lang-python";
import { r } from "@codemirror/legacy-modes/mode/r";
import { t } from "../../i18n";
import { LmsIcon } from "./LmsIcon";

/** Matches counted for the panel; past this the count reads as "999+". */
const COUNT_LIMIT = 999;

function matchCount(view: EditorView) {
  const query = getSearchQuery(view.state);
  if (!query.valid) return 0;
  let count = 0;
  for (const cursor = query.getCursor(view.state); count <= COUNT_LIMIT && !cursor.next().done; ) count++;
  return count;
}

/**
 * CodeMirror's search behind a small box floating at the code's top right:
 * the editor keeps its find commands, ⌘F and match highlighting.
 */
function searchPanel(view: EditorView): Panel {
  const dom = document.createElement("div");
  dom.className = "lmsCodeSearch";
  dom.setAttribute("role", "search");
  const input = document.createElement("input");
  input.setAttribute("main-field", "true");
  input.setAttribute("aria-label", t("Find in document"));
  input.placeholder = t("Find in document");
  input.value = getSearchQuery(view.state).search;
  const count = document.createElement("span");
  count.setAttribute("role", "status");
  const button = (label: string, icon: string, run: () => void) => {
    const element = document.createElement("button");
    element.type = "button";
    element.title = label;
    element.setAttribute("aria-label", label);
    element.innerHTML = icon;
    element.addEventListener("click", run);
    return element;
  };
  const arrow = (flip: boolean) =>
    `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"${flip ? ' style="transform:scaleY(-1)"' : ""}><path d="m6 15 6-6 6 6"/></svg>`;
  const showCount = () => {
    const total = input.value ? matchCount(view) : 0;
    count.textContent = !input.value ? "" : total === 0 ? t("No matches") : total > COUNT_LIMIT ? `${COUNT_LIMIT}+` : String(total);
  };
  input.addEventListener("input", () => {
    view.dispatch({ effects: setSearchQuery.of(new SearchQuery({ search: input.value })) });
    showCount();
  });
  input.addEventListener("keydown", event => {
    if (event.key === "Enter") {
      event.preventDefault();
      (event.shiftKey ? findPrevious : findNext)(view);
    } else if (event.key === "Escape") {
      event.preventDefault();
      closeSearchPanel(view);
    }
  });
  dom.append(
    input, count,
    button(t("Find previous"), arrow(false), () => findPrevious(view)),
    button(t("Find next"), arrow(true), () => findNext(view)),
    button(t("Close search"), "×", () => closeSearchPanel(view)),
  );
  return { dom, top: true, mount: () => { input.select(); showCount(); } };
}

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
  const editor = useRef<EditorView | null>(null);
  const [searching, setSearching] = useState(false);
  const language = textLanguage(url);
  const position = useRef({ top: 0, left: 0 });
  useEffect(() => {
    const view = new EditorView({ parent: host.current!, state: EditorState.create({ doc: value, extensions: [
      EditorState.readOnly.of(true), EditorView.editable.of(false), lineNumbers(), highlightActiveLineGutter(),
      search({ top: true, createPanel: searchPanel }), keymap.of(searchKeymap),
      EditorView.updateListener.of(update => setSearching(searchPanelOpen(update.state))),
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
    editor.current = view;
    return () => {
      position.current = { top: view.scrollDOM.scrollTop, left: view.scrollDOM.scrollLeft };
      editor.current = null;
      setSearching(false);
      view.destroy();
    };
  }, [value, url]);
  return <div className="lmsTextDocument">
    <div className="lmsTextMeta"><span>{t(language.name)}</span><span>{t("{{count}} lines", { count: value.split("\n").length })} · UTF-8</span></div>
    <div className="lmsTextCode">
      {!searching && <button type="button" className="lmsCodeSearchOpen" title={t("Find in document")} aria-label={t("Find in document")}
        onClick={() => editor.current && openSearchPanel(editor.current)}><LmsIcon name="search" size={14} /></button>}
      <div ref={host} />
    </div>
  </div>;
}
