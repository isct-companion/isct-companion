import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, posix, resolve } from "node:path";
import { runJscpd } from "jscpd";
import { platformFor, sourceRoots } from "./config.mjs";
import { walk } from "./functions.mjs";

// Leads for the coupling and duplication checks in docs/code-quality.md §3:
// how the desktop modules depend on one another, code repeated across files
// and shells, and files that change together. They point at places to read;
// whether two pieces are one concept is decided by reading them (§4).

// The desktop source is organised in layers. An import against this order,
// or between two siblings of one layer, is a lead. Each rule says why.
const desktop = "apps/desktop/src/";
export const isDesktopModule = (file) =>
  file.startsWith(desktop) && /\.tsx?$/.test(file);
const desktopRules = [
  {
    name: "shared UI imports a page",
    from: /^components\//,
    to: /^pages\//,
    why: "components/ are shared by pages and must not depend on one",
  },
  {
    name: "shared UI imports a feature",
    from: /^components\//,
    to: /^features\//,
    why: "components/ sit below features; code a feature shares moves into components/ or lib/",
  },
  {
    name: "module below the shell imports app/",
    from: /^(?:components|features|lib|platform|store|storage)\//,
    to: /^app\//,
    why: "app/ is the shell that mounts everything; a service others need belongs in platform/ or lib/",
  },
  {
    name: "library imports UI or app state",
    from: /^lib\//,
    to: /^(?:components|features|pages|app|store)\//,
    why: "lib/ holds plain helpers that every layer may import",
  },
  {
    name: "feature imports a page",
    from: /^features\//,
    to: /^pages\//,
    why: "a feature is mounted by pages, not the other way round",
  },
  {
    name: "page imports another page",
    from: /^pages\/([^/]+)\//,
    to: /^pages\/([^/]+)\//,
    sibling: true,
    why: "code two pages share moves to components/, features/ or lib/",
  },
  {
    name: "feature imports another feature's internals",
    from: /^features\/([^/]+)\//,
    // A feature may compose another through its public index.ts.
    to: /^features\/([^/]+)\/(?!index\.tsx?$)/,
    sibling: true,
    why: "a feature is used only through its index.ts; shared code moves below features",
  },
];

const importSources = (ast) => {
  const sources = [];
  walk(ast, (node) => {
    if (
      [
        "ImportDeclaration",
        "ExportNamedDeclaration",
        "ExportAllDeclaration",
      ].includes(node.type) &&
      node.source
    )
      sources.push(node.source.value);
    if (
      node.type === "ImportExpression" &&
      node.source.type === "StringLiteral"
    )
      sources.push(node.source.value);
    if (
      node.type === "CallExpression" &&
      node.callee.type === "Import" &&
      node.arguments[0]?.type === "StringLiteral"
    )
      sources.push(node.arguments[0].value);
  });
  return sources.filter((source) => source.startsWith("."));
};

// Vite resolves extensionless relative imports; the candidates are matched
// against the desktop module list.
function resolveImport(from, specifier, files) {
  const base = posix.normalize(posix.join(posix.dirname(from), specifier));
  return ["", ".ts", ".tsx", "/index.ts", "/index.tsx"]
    .map((suffix) => base + suffix)
    .find((candidate) => files.has(candidate));
}

// Tarjan's strongly connected components; each component of two or more
// modules is one import cycle.
function cycles(edges) {
  const index = new Map();
  const low = new Map();
  const stack = [];
  const onStack = new Set();
  const found = [];
  let next = 0;
  const visit = (node) => {
    index.set(node, next);
    low.set(node, next++);
    stack.push(node);
    onStack.add(node);
    for (const target of edges.get(node) ?? []) {
      if (!index.has(target)) {
        visit(target);
        low.set(node, Math.min(low.get(node), low.get(target)));
      } else if (onStack.has(target))
        low.set(node, Math.min(low.get(node), index.get(target)));
    }
    if (low.get(node) !== index.get(node)) return;
    const component = [];
    let member;
    do {
      member = stack.pop();
      onStack.delete(member);
      component.push(member);
    } while (member !== node);
    if (component.length > 1) found.push(component.sort());
  };
  for (const node of edges.keys()) if (!index.has(node)) visit(node);
  return found.sort((a, b) => b.length - a.length);
}

function desktopDependencies(imports, selected) {
  const files = new Set(imports.keys());
  const edges = new Map();
  const unresolved = [];
  for (const [file, specifiers] of imports) {
    const targets = new Set();
    for (const specifier of specifiers) {
      const target = resolveImport(file, specifier, files);
      if (target) targets.add(target);
      else if (!/\.(?:css|json|svg|png|woff2?)$/.test(specifier))
        unresolved.push({ file, specifier });
    }
    edges.set(file, targets);
  }
  const violations = [];
  for (const [file, targets] of edges) {
    const from = file.slice(desktop.length);
    for (const target of targets) {
      const to = target.slice(desktop.length);
      for (const rule of desktopRules) {
        const source = rule.from.exec(from);
        const destination = rule.to.exec(to);
        if (!source || !destination) continue;
        if (rule.sibling && source[1] === destination[1]) continue;
        violations.push({ rule: rule.name, why: rule.why, file, target });
      }
    }
  }
  const fanIn = new Map();
  for (const targets of edges.values())
    for (const target of targets)
      fanIn.set(target, (fanIn.get(target) ?? 0) + 1);
  const modules = [...edges.keys()]
    .map((file) => ({
      file,
      fanOut: edges.get(file).size,
      fanIn: fanIn.get(file) ?? 0,
    }))
    .sort((a, b) => b.fanOut - a.fanOut || a.file.localeCompare(b.file));
  return {
    violations: violations.filter(
      ({ file, target }) => selected.has(file) || selected.has(target),
    ),
    cycles: cycles(edges).filter((members) =>
      members.some((file) => selected.has(file)),
    ),
    modules: modules.filter(({ file }) => selected.has(file)),
    unresolved: unresolved.filter(({ file }) => selected.has(file)),
  };
}

// jscpd finds exact and renamed copies and, with --similarity, functions of
// the same structure in every language the audit reads.
function duplicates(repo, files) {
  const output = mkdtempSync(join(tmpdir(), "quality-audit-"));
  try {
    const result = runJscpd(
      [
        "--silent",
        "--no-tips",
        "--absolute",
        "--reporters",
        "json",
        "--output",
        output,
        "--report-name",
        "clones",
        "--min-tokens",
        "60",
        "--ignore-identifiers",
        "--similarity",
        "0.85",
        ...files.map((file) => resolve(repo, file)),
      ],
      { cwd: repo, encoding: "utf8", maxBuffer: 32 * 1024 * 1024 },
    );
    if (result.error) throw result.error;
    if (result.status !== 0)
      throw new Error(
        `jscpd exited with ${result.status}: ${result.stderr.trim()}`,
      );
    const { duplicates: clones, statistics } = JSON.parse(
      readFileSync(join(output, "clones.json"), "utf8"),
    );
    const side = ({ name, start, end }) => {
      const file = name.slice(repo.length + 1);
      return { file, line: start, endLine: end, platform: platformFor(file) };
    };
    const rows = clones
      .map((clone) => {
        const first = side(clone.firstFile);
        const second = side(clone.secondFile);
        return {
          kind: clone.kind,
          lines: clone.lines,
          tokens: clone.tokens,
          similarity: clone.similarity ?? null,
          crossPlatform: first.platform !== second.platform,
          first,
          second,
        };
      })
      // A repetitive table matches itself one row down; that is one block.
      .filter(
        ({ first, second }) =>
          first.file !== second.file ||
          first.endLine < second.line ||
          second.endLine < first.line,
      )
      .sort((a, b) => b.lines - a.lines);
    return { percentage: statistics.total.percentage, rows };
  } finally {
    rmSync(output, { recursive: true, force: true });
  }
}

// Squash merges make each commit one change, so files edited in the same
// commits are edited for the same reasons. Pairs in different directories
// that change together most of the time are shotgun-surgery leads; files
// changed by many commits are divergent-change leads.
const historyDepth = 400;
const bulkCommit = 120;
function changeHistory(repo, files) {
  const audited = new Set(files);
  const log = execFileSync(
    "git",
    [
      "log",
      "--no-merges",
      "--format=%x1e%h",
      "--name-only",
      "-n",
      String(historyDepth),
      "--",
      ...sourceRoots,
    ],
    { cwd: repo, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 },
  );
  const commits = log
    .split("\x1e")
    .map((entry) =>
      entry
        .split("\n")
        .slice(1)
        .filter((file) => audited.has(file)),
    )
    .filter((changed) => changed.length > 0 && changed.length <= bulkCommit);
  const changes = new Map();
  const pairs = new Map();
  for (const changed of commits) {
    for (const file of changed) changes.set(file, (changes.get(file) ?? 0) + 1);
    for (let i = 0; i < changed.length; i++)
      for (let j = i + 1; j < changed.length; j++) {
        const [a, b] = [changed[i], changed[j]].sort();
        if (posix.dirname(a) === posix.dirname(b)) continue;
        const key = `${a}\n${b}`;
        pairs.set(key, (pairs.get(key) ?? 0) + 1);
      }
  }
  const coChanges = [...pairs]
    .map(([key, together]) => {
      const [a, b] = key.split("\n");
      const degree = together / Math.min(changes.get(a), changes.get(b));
      return {
        files: [a, b],
        together,
        degree: Math.round(degree * 100) / 100,
        crossPlatform: platformFor(a) !== platformFor(b),
      };
    })
    .filter((pair) => pair.together >= 4 && pair.degree >= 0.6)
    .sort((a, b) => b.together - a.together || b.degree - a.degree);
  const hotspots = [...changes]
    .map(([file, commits]) => ({ file, platform: platformFor(file), commits }))
    .sort((a, b) => b.commits - a.commits || a.file.localeCompare(b.file));
  return { commits: commits.length, coChanges, hotspots };
}

export function createCouplingSection({ repo }) {
  const files = [];
  const imports = new Map();
  return {
    accepts: () => true,
    visit({ file, ast, selected }) {
      if (selected) files.push(file);
      if (isDesktopModule(file)) imports.set(file, importSources(ast()));
    },
    finish() {
      const errors = [];
      const attempt = (label, run) => {
        try {
          return run();
        } catch (error) {
          errors.push({ message: `${label}: ${error.message}` });
          return null;
        }
      };
      return {
        data: {
          dependencies: desktopDependencies(imports, new Set(files)),
          duplicates: attempt("Duplicate detection", () =>
            duplicates(repo, files),
          ),
          history: attempt("Change history", () => changeHistory(repo, files)),
        },
        errors,
        skipped: [],
      };
    },
  };
}
