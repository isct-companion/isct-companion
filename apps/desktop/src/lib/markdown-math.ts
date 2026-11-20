import katex from "katex";

/**
 * Prepares a shared Markdown document for reading: its math in the `$…$` /
 * `$$…$$` forms the Markdown viewer renders with KaTeX, and nothing that only
 * a web page would run.
 *
 * Shared documents (past exams above all) are often written for a page that
 * runs Markdown first and MathJax after it. They use `\(…\)` and `\[…\]`,
 * double backslashes for Markdown to halve (`\\rightarrow`), and may start
 * with a `<script>` that loads MathJax. Read as standard Markdown, the
 * delimiters show as raw source, KaTeX sets `\\` as a line break and the
 * command name as italic letters, and the script shows as text. Here the
 * delimiters become `$` math, a formula written with doubled backslashes is
 * undoubled (see `undoubleBackslashes`), and `<script>`/`<style>` blocks are
 * dropped.
 *
 * A display formula on lines of its own, whether `\[…\]`, `$$…$$` on one
 * line, or a `$$` fence inside a paragraph, becomes a fenced `$$` block set
 * apart by blank lines, so it is set as a centred block instead of inline
 * text. Fenced code, indented code and inline code are left as written.
 *
 * Meant for documents shown read-only: an editor would save the rewritten
 * text back into the user's file.
 */
export function readableMarkdown(markdown: string): string {
  if (!/\\[([]|\$|<(?:script|style)\b/i.test(markdown)) return markdown;
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
    const formula = stackLines(
      undoubleBackslashes(body.map((l) => l.trim()).join("\n")),
    );
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
    // A page's own <script> or <style> (MathJax's loader) is not content.
    const tag = /^\s*<(script|style)\b/i.exec(line);
    if (tag) {
      const close = new RegExp(`</${tag[1]}\\s*>`, "i");
      const end = lines.findIndex((l, i) => i >= index && close.test(l));
      if (end >= index) {
        flush();
        index = end;
        spaceAfter = out.length > 0;
        continue;
      }
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
 * Undoubles a formula written for Markdown to unescape: one whose every
 * command has an even run of backslashes (`\\alpha`), at least one of exactly
 * two. A formula with any single-backslash command is standard LaTeX and stays
 * as written.
 *
 * In an undoubled formula `\\name` is the command `\name` when KaTeX knows
 * it. Otherwise, or when a later line starts with it and the name is one
 * letter (`\\v` there is not the caron accent), it is the line break `\\`
 * before `name`, as the author laid the lines out. Other runs are halved,
 * except that `\\` before a space or the end stays a line break.
 */
export function undoubleBackslashes(formula: string): string {
  const commands = formula.match(/\\+(?=[A-Za-z])/g);
  if (
    !commands?.some((run) => run.length === 2) ||
    commands.some((run) => run.length % 2 === 1)
  )
    return formula;
  return formula.replace(
    /(\\+)([A-Za-z]+)?/g,
    (all: string, run: string, name: string | undefined, offset: number) => {
      const half = "\\".repeat(Math.ceil(run.length / 2));
      if (run.length !== 2) return half + (name ?? "");
      if (!name) {
        const after = formula[offset + 2];
        return after === undefined || /\s/.test(after) ? run : half;
      }
      const lineStart =
        offset > 0 && /\n[ \t]*$/.test(formula.slice(0, offset));
      return knownCommand(name) && !(lineStart && name.length === 1)
        ? `\\${name}`
        : `\\\\ ${name}`;
    },
  );
}

/**
 * KaTeX sets a bare `\\` in a display formula but warns that LaTeX would
 * ignore it; lines broken that way are stacked in `gathered`, centred as
 * they were laid out.
 */
function stackLines(formula: string): string {
  if (!/(?<!\\)\\\\(?!\\)/.test(formula) || /\\begin\{/.test(formula))
    return formula;
  return `\\begin{gathered}\n${formula}\n\\end{gathered}`;
}

const commandCache = new Map<string, boolean>();
/** Whether KaTeX defines `\name`; one that only lacks its argument counts. */
function knownCommand(name: string): boolean {
  let known = commandCache.get(name);
  if (known === undefined) {
    try {
      katex.renderToString(`\\${name}`, { throwOnError: true });
      known = true;
    } catch (error) {
      known = !String(error).includes("Undefined control sequence");
    }
    commandCache.set(name, known);
  }
  return known;
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
