import { useMemo } from "react";
import { MarkdownDocument } from "../../features/lms/MarkdownDocument";
import { readableMarkdown } from "../../features/lms/markdown-math";

/** A past exam's Markdown, read-only, prepared for reading (its math as
 * KaTeX reads it, no page scripts). Loaded lazily with the viewer and KaTeX. */
export function ExamMarkdown({ text }: { text: string }) {
  const value = useMemo(() => readableMarkdown(text), [text]);
  return <MarkdownDocument value={value} />;
}
