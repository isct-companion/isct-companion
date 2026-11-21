import { execFileSync } from "node:child_process";
import {
  mkdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { posix, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { parse as parseTs } from "@babel/parser";
import {
  isAuditedSource,
  metricLabels,
  platformFor,
  sourceRoots,
} from "./config.mjs";
import { createFunctionSection } from "./functions.mjs";
import { createStyleSection } from "./styles.mjs";
import { createCouplingSection, isDesktopModule } from "./coupling.mjs";
import { renderMarkdown } from "./report.mjs";

const usage = `Usage: yarn quality:audit [options]

  --styles          Find hard-coded UI values, aliases, and repeated values
  --functions       Measure function size, inputs, branches, and nesting
  --structure       Summarize source files and directories by size
  --coupling        Find layer violations, import cycles, duplicates, and files that change together
  --scope TEXT      Include paths containing TEXT (repeatable, case-insensitive OR;
                    end with / to match a directory, not its siblings)
  --changed         Scan branch and working-tree changes from the merge base with origin/main
  --base REF        Use REF in place of origin/main and enable changed-file mode
  --sort METRIC     Rank functions by ${Object.keys(metricLabels).join(", ")} (default: branches)
  --limit N         Show at most N rows per ranking in Markdown (default: 10)
  --python PATH     Python interpreter for Rust and Swift (default: python3)
  --kotlin-lib DIR  Kotlin compiler library directory (default: $KOTLIN_HOME/lib)
  --repo DIR        Checkout to scan (default: this repository)

All sections run by default. A language whose parser is not installed is
skipped and named in the report. Both .quality-audit/report.md and report.json
are replaced on each scan.`;

// Each section sees the files it accepts once, then reports its result.
const sections = {
  styles: createStyleSection,
  functions: createFunctionSection,
  structure: createStructureSection,
  coupling: createCouplingSection,
};

function parseOptions(argv) {
  const { values } = parseArgs({
    args: argv,
    options: {
      repo: {
        type: "string",
        default: fileURLToPath(new URL("../../..", import.meta.url)),
      },
      scope: { type: "string", multiple: true, default: [] },
      styles: { type: "boolean" },
      functions: { type: "boolean" },
      structure: { type: "boolean" },
      coupling: { type: "boolean" },
      changed: { type: "boolean" },
      base: { type: "string" },
      sort: { type: "string", default: "branches" },
      limit: { type: "string", default: "10" },
      python: { type: "string", default: "python3" },
      "kotlin-lib": { type: "string" },
      help: { type: "boolean" },
    },
  });
  if (!Object.hasOwn(metricLabels, values.sort))
    throw new Error(`Invalid --sort: ${values.sort}`);
  const limit = Number(values.limit);
  if (!Number.isSafeInteger(limit) || limit < 1)
    throw new Error("--limit must be a positive integer");
  const chosen = Object.keys(sections).filter((name) => values[name]);
  return {
    ...values,
    limit,
    repo: resolve(values.repo),
    kotlinLib: values["kotlin-lib"],
    sections: chosen.length > 0 ? chosen : Object.keys(sections),
  };
}

function main(argv) {
  const options = parseOptions(argv);
  if (options.help) {
    console.log(usage);
    return;
  }
  const { files, desktopFiles, base } = sourceFiles(options);
  const report = audit(files, desktopFiles, { ...options, base });
  const output = resolve(options.repo, ".quality-audit");
  mkdirSync(output, { recursive: true });
  writeReport(
    resolve(output, "report.md"),
    renderMarkdown(report, options.limit),
  );
  writeReport(
    resolve(output, "report.json"),
    `${JSON.stringify(report, null, 2)}\n`,
  );
  console.log(
    `Scanned ${files.length} source files. Reports written to ${output}`,
  );
  for (const { languages, reason } of report.skipped)
    console.warn(
      `Skipped ${languages.join(" and ")} function metrics. ${reason}`,
    );
  if (report.errors.length > 0) {
    console.error(
      `${report.errors.length} scan errors. See the reports for details.`,
    );
    process.exitCode = 1;
  }
}

function writeReport(path, contents) {
  const pending = `${path}.${process.pid}.tmp`;
  try {
    writeFileSync(pending, contents, "utf8");
    renameSync(pending, path);
  } finally {
    rmSync(pending, { force: true });
  }
}

// Git lists tracked and untracked, nonignored files with `/` separators on
// every host, and the changed-file mode needs it anyway.
function sourceFiles({ repo, changed, base: ref, scope, sections }) {
  const git = (...args) =>
    execFileSync("git", args, {
      cwd: repo,
      encoding: "utf8",
      maxBuffer: 32 * 1024 * 1024,
    })
      .split("\0")
      .filter(Boolean);
  const listed = (...args) =>
    git("ls-files", "-z", ...args, "--", ...sourceRoots);
  const deleted = new Set(listed("--deleted"));
  let base = null;
  let changedFiles = null;
  if (changed || ref) {
    const commit = git(
      "rev-parse",
      "--verify",
      "--end-of-options",
      `${ref ?? "origin/main"}^{commit}`,
    )[0].trim();
    base = git("merge-base", "HEAD", commit)[0].trim();
    changedFiles = new Set([
      ...git(
        "diff",
        "--name-only",
        "--diff-filter=ACMRTUXB",
        "-z",
        base,
        "--",
        ...sourceRoots,
      ),
      ...listed("--others", "--exclude-standard"),
    ]);
  }
  const fragments = scope.map((value) => value.toLowerCase());
  const sources = [
    ...new Set(listed("--cached", "--others", "--exclude-standard")),
  ]
    .filter((file) => !deleted.has(file))
    .sort();
  const files = sources
    .filter(isAuditedSource)
    .filter((file) => !changedFiles || changedFiles.has(file))
    .filter(
      (file) =>
        fragments.length === 0 ||
        fragments.some((value) => file.toLowerCase().includes(value)),
    );
  const desktopFiles =
    sections.includes("coupling") && files.some(isDesktopModule)
      ? sources.filter(isDesktopModule)
      : [];
  return { files, desktopFiles, base };
}

function createStructureSection() {
  const files = [];
  return {
    accepts: () => true,
    visit: ({ file, platform, lines }) =>
      files.push({
        file,
        platform,
        lines: lines.filter((line) => line.trim()).length,
      }),
    finish() {
      files.sort((a, b) => b.lines - a.lines || a.file.localeCompare(b.file));
      const directories = new Map();
      for (const { file, platform, lines } of files) {
        const directory = posix.dirname(file);
        if (!directories.has(directory))
          directories.set(directory, {
            directory,
            platform,
            files: 0,
            lines: 0,
          });
        const info = directories.get(directory);
        info.files++;
        info.lines += lines;
      }
      return {
        data: {
          files,
          directories: [...directories.values()].sort(
            (a, b) =>
              b.lines - a.lines || a.directory.localeCompare(b.directory),
          ),
        },
        errors: [],
        skipped: [],
      };
    },
  };
}

function audit(files, desktopFiles, options) {
  const { repo } = options;
  const read = (file) => readFileSync(resolve(repo, file), "utf8");
  const active = options.sections.map((name) => [
    name,
    sections[name]({ ...options, read }),
  ]);
  const errors = [];
  const selectedFiles = new Set(files);
  const inputs = new Set([...files, ...desktopFiles]);
  for (const file of inputs) {
    const selected = selectedFiles.has(file);
    const visitors = active.filter(
      ([name, section]) =>
        (selected || name === "coupling") && section.accepts(file),
    );
    if (visitors.length === 0) continue;
    try {
      const source = read(file);
      let ast;
      const entry = {
        file,
        selected,
        platform: platformFor(file),
        source,
        lines: source.split(/\r?\n/),
        // Parsed once, on first use, for every section that reads TS/TSX.
        ast: () =>
          (ast ??= parseTs(source, {
            sourceType: "module",
            plugins: file.endsWith(".tsx")
              ? ["typescript", "jsx"]
              : ["typescript"],
          })),
      };
      for (const [, section] of visitors) section.visit(entry);
    } catch (error) {
      errors.push({ file, message: error.reason ?? error.message });
    }
  }
  const report = {
    repo,
    scope: options.scope,
    sections: options.sections,
    base: options.base,
    sort: options.sort,
    fileCount: files.length,
    styles: null,
    functions: null,
    structure: null,
    coupling: null,
    skipped: [],
    errors,
  };
  for (const [name, section] of active) {
    const { data, errors: sectionErrors, skipped } = section.finish();
    report[name] = data;
    report.errors.push(...sectionErrors);
    report.skipped.push(...skipped);
  }
  return report;
}

try {
  main(process.argv.slice(2));
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
