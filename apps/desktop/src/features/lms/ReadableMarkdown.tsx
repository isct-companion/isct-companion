import { useMemo } from "react";
import { MarkdownDocument } from "./MarkdownDocument";
import { readableMarkdown } from "./markdown-math";

/** A Markdown file, read-only, prepared for reading (its math as KaTeX reads
 * it, no page scripts): course files and past exams alike. Loaded lazily
 * with the viewer and KaTeX. */
export function ReadableMarkdown({ text }: { text: string }) {
  const value = useMemo(() => readableMarkdown(text), [text]);
  return <MarkdownDocument value={value} />;
}
