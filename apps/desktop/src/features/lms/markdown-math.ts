/**
 * Rewrites the LaTeX math delimiters that Markdown readers don't parse into the
 * `$…$` / `$$…$$` forms the Markdown viewer renders with KaTeX.
 *
 * Shared documents (past exams above all) are often written or exported with
 * `\(…\)` and `\[…\]`, which CommonMark reads as escaped brackets, so the
 * formula showed as raw source. A display formula on lines of its own, whether
 * `\[…\]`, `$$…$$` on one line, or a `$$` fence inside a paragraph, becomes a
 * fenced `$$` block set apart by blank lines, so it is set as a centred block
 * instead of inline text. Fenced code, indented code, inline code and existing
 * inline math are left as written.
 *
 * Meant for documents shown read-only: an editor would save the rewritten
 * delimiters back into the user's file.
 */
export function normalizeMathDelimiters(markdown: string): string {
  if (!/\\[([]|\$\$/.test(markdown)) return markdown;
  const lines = markdown.split(/\r?\n/);
  const out: string[] = [];
  let chunk: string[] = [];
  let spaceAfter = false;
  const push = (line: string) => {
    if (spaceAfter && line.trim() !== "") out.push("");
    spaceAfter = false;
    out.push(line);
  };
  const flush = () => {
    if (chunk.length)
      for (const line of rewriteInline(chunk.join("\n")).split("\n"))
        push(line);
    chunk = [];
  };
  const block = (indent: string, body: string[]) => {
    flush();
    if (out.length && out[out.length - 1].trim() !== "") out.push("");
    out.push(
      `${indent}$$`,
      ...body.map((line) => indent + line.trim()),
      `${indent}$$`,
    );
    spaceAfter = true;
  };
  let fence: { char: string; size: number } | null = null;
  for (let index = 0; index < lines.length; index++) {
    const line = lines[index];
    if (fence) {
      push(line);
      const close = /^\s*(`{3,}|~{3,})\s*$/.exec(line);
      if (close && close[1][0] === fence.char && close[1].length >= fence.size)
        fence = null;
      continue;
    }
    const code = /^\s*(?:(?:[-*+]|\d+[.)])\s+)?(`{3,}|~{3,})/.exec(line);
    if (code) {
      flush();
      fence = { char: code[1][0], size: code[1].length };
      push(line);
      continue;
    }
    if (line.trim() === "" || (chunk.length === 0 && /^( {4}|\t)/.test(line))) {
      // A blank line ends a paragraph; an indented line outside one is code.
      flush();
      push(line);
      continue;
    }
    const indent = /^\s*/.exec(line)![0];
    const text = line.trim();
    // `$$` alone (or with text after it) opens a math fence: keep its body
    // as written up to the line that closes it.
    if (/^\$\$[^$]*$/.test(text)) {
      const end = lines.findIndex((l, i) => i > index && /\$\$\s*$/.test(l));
      if (end > index) {
        const body = [text.slice(2), ...lines.slice(index + 1, end)];
        body.push(lines[end].replace(/\$\$\s*$/, ""));
        block(
          indent,
          body.filter(
            (l, i) => l.trim() !== "" || (i > 0 && i < body.length - 1),
          ),
        );
        index = end;
        continue;
      }
    }
    // `$$…$$` filling its line is a display formula, not inline math.
    const single = /^\$\$((?:(?!\$\$).)+)\$\$$/.exec(text);
    if (single) {
      block(indent, [single[1]]);
      continue;
    }
    // `\[` opening a line, closed by `\]` ending a line.
    if (text.startsWith("\\[")) {
      const end = lines.findIndex(
        (l, i) =>
          i >= index &&
          /\\\]\s*$/.test(i === index ? l.trimStart().slice(2) : l),
      );
      const between = end > index ? lines.slice(index + 1, end) : [];
      const body =
        end === index
          ? [text.slice(2).replace(/\\\]\s*$/, "")]
          : [
              text.slice(2),
              ...between,
              (lines[end] ?? "").replace(/\\\]\s*$/, ""),
            ];
      if (
        end >= index &&
        !between.some((l) => l.trim() === "") &&
        !body.some((l) => l.includes("\\]"))
      ) {
        block(
          indent,
          body.filter((l) => l.trim() !== ""),
        );
        index = end;
        continue;
      }
    }
    chunk.push(line);
  }
  flush();
  return out.join("\n");
}

/** `\(…\)` → `$…$` and `\[…\]` → `$$…$$` inside running text. */
function rewriteInline(text: string): string {
  let result = "";
  let i = 0;
  while (i < text.length) {
    const char = text[i];
    if (char === "\\") {
      const next = text[i + 1];
      if (next === "(" || next === "[") {
        const end = text.indexOf(next === "(" ? "\\)" : "\\]", i + 2);
        const body = end < 0 ? "" : text.slice(i + 2, end).trim();
        if (body) {
          result += next === "(" ? `$${body}$` : `$$${body}$$`;
          i = end + 2;
          continue;
        }
      }
      // Any other escape, an escaped backslash included, stays as written.
      result += text.slice(i, i + 2);
      i += 2;
      continue;
    }
    if (char === "`" || char === "$") {
      // Inline code and existing math pass through to their matching closer.
      let run = 1;
      while (text[i + run] === char) run++;
      const marker = char.repeat(run);
      const end = text.indexOf(marker, i + run);
      const stop = end > 0 ? end + run : i + run;
      result += text.slice(i, stop);
      i = stop;
      continue;
    }
    result += char;
    i++;
  }
  return result;
}
