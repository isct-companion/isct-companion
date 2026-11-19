/**
 * The read-only Markdown page the Android and iOS apps show a past exam's
 * Markdown in (`resources/markdown-viewer`, built by `yarn markdown-viewer`).
 *
 * It prepares the text with the same rules as the desktop reader
 * (`readableMarkdown`: `\(…\)`, doubled backslashes, page scripts) and renders
 * it with micromark (CommonMark + GFM) and KaTeX, offline: the page loads
 * nothing over the network but images the document itself links. Raw HTML in
 * the document is shown as text, never run.
 *
 * The app loads the page from its bundle and calls
 * `isctMarkdown.show(text, look)` once it has loaded.
 */
import "katex/dist/katex.min.css";
import { micromark } from "micromark";
import { gfm, gfmHtml } from "micromark-extension-gfm";
import { math, mathHtml } from "micromark-extension-math";
import { readableMarkdown } from "../lms/markdown-math";
import "./viewer.css";

/** Colours and size from the app's theme, as CSS values. */
export type MarkdownLook = {
  background?: string;
  text?: string;
  muted?: string;
  line?: string;
  accent?: string;
  surface?: string;
  /** Body text size in CSS pixels. */
  fontSize?: number;
  dark?: boolean;
};

export function renderMarkdown(text: string): string {
  return micromark(readableMarkdown(text), {
    extensions: [gfm(), math()],
    htmlExtensions: [
      gfmHtml(),
      mathHtml({ throwOnError: false, strict: "ignore", trust: false }),
    ],
  });
}

export function show(text: string, look: MarkdownLook = {}) {
  const root = document.documentElement;
  const vars: [string, string | undefined][] = [
    ["--md-background", look.background],
    ["--md-text", look.text],
    ["--md-muted", look.muted],
    ["--md-line", look.line],
    ["--md-accent", look.accent],
    ["--md-surface", look.surface],
    ["--md-size", look.fontSize ? `${look.fontSize}px` : undefined],
  ];
  for (const [name, value] of vars)
    if (value) root.style.setProperty(name, value);
    else root.style.removeProperty(name);
  root.style.colorScheme = look.dark ? "dark" : "light";
  const article = document.getElementById("document")!;
  article.innerHTML = renderMarkdown(text);
  // Wide tables scroll on their own instead of widening the page.
  for (const table of article.querySelectorAll("table")) {
    const frame = document.createElement("div");
    frame.className = "md-table";
    table.replaceWith(frame);
    frame.append(table);
  }
  for (const link of article.querySelectorAll("a[href]"))
    link.setAttribute("rel", "noopener noreferrer");
}

declare global {
  interface Window {
    isctMarkdown: { show: typeof show; renderMarkdown: typeof renderMarkdown };
  }
}

window.isctMarkdown = { show, renderMarkdown };
