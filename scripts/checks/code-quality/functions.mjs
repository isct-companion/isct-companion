import { execFileSync } from "node:child_process";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { functionRow, languages, metricLabels } from "./config.mjs";

export function walk(node, visit, parent = null) {
  if (!node || typeof node.type !== "string") return;
  if (visit(node, parent) === false) return;
  for (const value of Object.values(node)) {
    if (Array.isArray(value))
      for (const child of value) walk(child, visit, node);
    else walk(value, visit, node);
  }
}

const tsFunctions = new Set([
  "FunctionDeclaration",
  "FunctionExpression",
  "ArrowFunctionExpression",
  "ObjectMethod",
  "ClassMethod",
  "ClassPrivateMethod",
]);
const tsControls = new Set([
  "IfStatement",
  "ConditionalExpression",
  "ForStatement",
  "ForInStatement",
  "ForOfStatement",
  "WhileStatement",
  "DoWhileStatement",
  "CatchClause",
]);

// The name a declaration is called by, or null for anything that is not a
// function and for inline callbacks, which count toward their caller.
function tsName(node, parent) {
  if (!tsFunctions.has(node.type)) return null;
  return (
    node.id?.name ??
    node.key?.name ??
    node.key?.id?.name ??
    node.key?.value ??
    (parent?.type === "VariableDeclarator" ? parent.id.name : null) ??
    (["ObjectProperty", "ClassProperty", "ClassPrivateProperty"].includes(
      parent?.type,
    )
      ? (parent.key.name ?? parent.key.id?.name ?? parent.key.value)
      : null) ??
    (parent?.type === "ExportDefaultDeclaration" ? "default" : null)
  );
}

// `else if` continues its chain at the same depth, like a switch arm. Every
// parser below applies the same rule.
const isElseIf = (node, parent) =>
  node.type === "IfStatement" &&
  parent?.type === "IfStatement" &&
  parent.alternate === node;

function tsMetrics(node, name, file) {
  const depths = new WeakMap();
  let branches = 0;
  let nesting = 0;
  walk(node.body, (child, parent) => {
    if (tsName(child, parent) != null) return false;
    const control = tsControls.has(child.type);
    if (control || child.type === "SwitchCase") branches++;
    const nests =
      child.type === "SwitchStatement" || (control && !isElseIf(child, parent));
    const depth = (depths.get(parent) ?? 0) + Number(nests);
    depths.set(child, depth);
    nesting = Math.max(nesting, depth);
  });
  const params = node.params.filter(
    (param) => !(param.type === "Identifier" && param.name === "this"),
  );
  const inputs = params.reduce((count, param) => {
    const pattern = param.type === "AssignmentPattern" ? param.left : param;
    if (pattern.type === "ObjectPattern")
      return count + pattern.properties.length;
    if (pattern.type === "ArrayPattern")
      return count + pattern.elements.filter(Boolean).length;
    return count + 1;
  }, 0);
  return functionRow({
    file,
    name,
    language: "TS/TSX",
    line: node.loc.start.line,
    endLine: node.loc.end.line,
    parameters: params.length,
    inputs,
    branches,
    nesting,
  });
}

export function tsFunctionRows(ast, file) {
  const rows = [];
  walk(ast, (node, parent) => {
    const name = tsName(node, parent);
    if (name != null) rows.push(tsMetrics(node, String(name), file));
  });
  return rows;
}

const tool = (name) => fileURLToPath(new URL(name, import.meta.url));

// Rust and Swift go through Tree-sitter in Python, Kotlin through the Kotlin
// compiler's PSI in Java; neither has a maintained parser for Node. Both print
// one JSON object per line: a function row, `{file, error}` for a file that
// did not parse, or `{unavailable}` when the parser itself is not installed.
function runParser(program, args, input, setup) {
  let output;
  try {
    output = execFileSync(program, args, {
      encoding: "utf8",
      input,
      stdio: "pipe",
      maxBuffer: 32 * 1024 * 1024,
    });
  } catch (error) {
    if (error.code === "ENOENT")
      return { unavailable: `${program} not found. ${setup}` };
    const detail = String(error.stderr ?? "")
      .trim()
      .split("\n")
      .at(-1);
    return { failure: `${program} exited with ${error.status}: ${detail}` };
  }
  return {
    records: output
      .split("\n")
      .filter(Boolean)
      .map((line) => JSON.parse(line)),
  };
}

const nativeParsers = [
  {
    languages: { ".rs": "Rust", ".swift": "Swift" },
    run: (files, { python }) =>
      runParser(
        python,
        ["-I", "-B", tool("native-functions.py")],
        JSON.stringify(files),
        "Pass --python or install requirements.txt.",
      ),
  },
  {
    languages: { ".kt": "Kotlin" },
    run: (files, { kotlinLib }) => {
      const lib =
        kotlinLib ??
        (process.env.KOTLIN_HOME && join(process.env.KOTLIN_HOME, "lib"));
      if (!lib) return { unavailable: "Pass --kotlin-lib or set KOTLIN_HOME." };
      return runParser(
        "java",
        ["-cp", join(resolve(lib), "*"), tool("KotlinFunctions.java")],
        `${files.join("\n")}\n`,
        "Install Java 17 or newer.",
      );
    },
  },
];

function nativeFunctionRows(files, repo, options) {
  const rows = [];
  const errors = [];
  const skipped = [];
  for (const parser of nativeParsers) {
    const language = (file) => parser.languages[/\.\w+$/.exec(file)?.[0] ?? ""];
    const selected = files.filter(language);
    if (selected.length === 0) continue;
    const names = [...new Set(selected.map(language))];
    const result = parser.run(
      selected.map((file) => resolve(repo, file)),
      options,
    );
    const unavailable =
      result.unavailable ??
      result.records?.find((record) => record.unavailable)?.unavailable;
    if (unavailable) {
      skipped.push({ languages: names, reason: unavailable });
      continue;
    }
    if (result.failure) {
      errors.push({
        message: `${names.join(" and ")} metrics: ${result.failure}`,
      });
      continue;
    }
    for (const { file: path, error, ...metrics } of result.records) {
      const file = path.slice(repo.length + 1);
      if (error) errors.push({ file, message: error });
      else
        rows.push(functionRow({ ...metrics, file, language: language(file) }));
    }
  }
  return { rows, errors, skipped };
}

function functionSummary(rows) {
  return languages
    .map((language) => {
      const own = rows.filter((row) => row.language === language);
      const metrics = Object.fromEntries(
        Object.keys(metricLabels).map((metric) => {
          const sorted = own.map((row) => row[metric]).sort((a, b) => a - b);
          const at = (percentile) =>
            sorted[Math.ceil(percentile * sorted.length) - 1] ?? null;
          return [
            metric,
            { p50: at(0.5), p90: at(0.9), max: sorted.at(-1) ?? null },
          ];
        }),
      );
      return { language, count: own.length, metrics };
    })
    .filter((group) => group.count > 0);
}

export function createFunctionSection({ repo, sort, python, kotlinLib }) {
  const rows = [];
  const native = [];
  return {
    accepts: (file) => /\.(?:tsx?|rs|swift|kt)$/.test(file),
    visit({ file, ast }) {
      if (/\.tsx?$/.test(file)) rows.push(...tsFunctionRows(ast(), file));
      else native.push(file);
    },
    finish() {
      const {
        rows: nativeRows,
        errors,
        skipped,
      } = nativeFunctionRows(native, repo, { python, kotlinLib });
      rows.push(...nativeRows);
      rows.sort(
        (a, b) =>
          b[sort] - a[sort] ||
          b.lines - a.lines ||
          a.file.localeCompare(b.file) ||
          a.line - b.line,
      );
      return {
        data: { rows, summary: functionSummary(rows) },
        errors,
        skipped,
      };
    },
  };
}
