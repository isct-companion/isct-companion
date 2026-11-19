/**
 * Rewrites a Markdown document's math into the `$…$` / `$$…$$` forms the
 * Markdown viewer renders with KaTeX.
 *
 * Shared documents (past exams above all) are often written for a page that
 * runs Markdown first and MathJax after it, so they use `\(…\)` and `\[…\]`,
 * or double every backslash (`\\(`, `\\rightarrow`) for Markdown to halve.
 * Read as standard Markdown, the first show as raw source and the second set
 * `\\` as a line break and the command name as italic letters. Both are
 * rewritten here: the delimiters become `$` math, and a formula whose every
 * command is written with a doubled backslash gets its backslashes halved.
 *
 * A display formula on lines of its own, whether `\[…\]`, `$$…$$` on one line,
 * or a `$$` fence inside a paragraph, becomes a fenced `$$` block set apart by
 * blank lines, so it is set as a centred block instead of inline text. Fenced
 * code, indented code and inline code are left as written.
 *
 * Meant for documents shown read-only: an editor would save the rewritten
 * math back into the user's file.
 */
export function normalizeMathDelimiters(markdown: string): string {
  if (!/\\[([]|\$/.test(markdown)) return markdown;
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
    const formula = undoubleBackslashes(body.map((l) => l.trim()).join("\n"));
    out.push(
      `${indent}$$`,
      ...formula.split("\n").map((line) => indent + line),
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
    // `$$` alone (or with text after it) opens a math fence that runs to the
    // line ending in `$$`.
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
    // `\[` (or `\\[`) opening a line, closed by `\]` (`\\]`) ending a line.
    const opener = /^(\\{1,2})\[/.exec(text);
    if (opener) {
      const slashes = opener[1];
      const closer = new RegExp(
        `(?<!\\\\)${slashes.replace(/\\/g, "\\\\")}\\]\\s*$`,
      );
      const start = text.slice(opener[0].length);
      const end = lines.findIndex(
        (l, i) => i >= index && closer.test(i === index ? start : l),
      );
      const between = end > index ? lines.slice(index + 1, end) : [];
      const body =
        end === index
          ? [start.replace(closer, "")]
          : [start, ...between, (lines[end] ?? "").replace(closer, "")];
      if (
        end >= index &&
        !between.some((l) => l.trim() === "") &&
        !body.some((l) => l.includes(`${slashes}]`))
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

/**
 * Halves the backslashes of a formula written for Markdown to unescape:
 * every command has an even run of backslashes (`\\alpha`) and at least one
 * has exactly two. A formula with any single-backslash command is standard
 * LaTeX and stays as written, its `\\` line breaks included.
 */
export function undoubleBackslashes(formula: string): string {
  const commands = formula.match(/\\+(?=[A-Za-z])/g);
  if (
    !commands?.some((run) => run.length === 2) ||
    commands.some((run) => run.length % 2 === 1)
  )
    return formula;
  return formula.replace(/\\+/g, (run) =>
    "\\".repeat(Math.ceil(run.length / 2)),
  );
}

/** `\(…\)` → `$…$` and `\[…\]` → `$$…$$` inside running text (also with
 * doubled backslashes); existing `$` math has its backslashes undoubled. */
function rewriteInline(text: string): string {
  let result = "";
  let i = 0;
  while (i < text.length) {
    const char = text[i];
    if (char === "\\") {
      let run = 1;
      while (text[i + run] === "\\") run++;
      const next = text[i + run];
      if (run <= 2 && (next === "(" || next === "[")) {
        const slashes = "\\".repeat(run);
        const closer = slashes + (next === "(" ? ")" : "]");
        const end = text.indexOf(closer, i + run + 1);
        const body =
          end < 0
            ? ""
            : undoubleBackslashes(text.slice(i + run + 1, end).trim());
        if (body) {
          result += next === "(" ? `$${body}$` : `$$${body}$$`;
          i = end + closer.length;
          continue;
        }
      }
      // An odd run escapes the next character; both stay as written.
      const stop = i + run + (run % 2 === 1 && next !== undefined ? 1 : 0);
      result += text.slice(i, stop);
      i = stop;
      continue;
    }
    if (char === "`" || char === "$") {
      // Inline code passes through to its matching closer; math is undoubled.
      let run = 1;
      while (text[i + run] === char) run++;
      const marker = char.repeat(run);
      const end = text.indexOf(marker, i + run);
      if (end > 0) {
        const inner = text.slice(i + run, end);
        result +=
          marker + (char === "$" ? undoubleBackslashes(inner) : inner) + marker;
        i = end + run;
      } else {
        result += marker;
        i += run;
      }
      continue;
    }
    result += char;
    i++;
  }
  return result;
}
