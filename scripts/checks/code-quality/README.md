# Code quality audit

Run `yarn quality:audit` from the repository root to scan UI styles, function
boundaries, and source organization. The scanner replaces
`.quality-audit/report.md` and `.quality-audit/report.json` on each scan. Both
reports stay local and Git ignores the directory.

## Setup

Run `yarn install`; style, structure and TS/TSX function scans need nothing
else. Full function scans also use Python with the pinned Tree-sitter
packages for Rust and Swift, and Java 17 or newer with the Kotlin compiler
libraries for Kotlin. Kotlin 2.4.0 was used to verify the scanner.

In a Python environment outside the checkout, install the parser packages with
`python3 -m pip install -r scripts/checks/code-quality/requirements.txt`. Activate
that environment or pass its interpreter with `--python`. Set `KOTLIN_HOME` to
a Kotlin distribution root, or pass the directory containing
`kotlin-compiler.jar` with `--kotlin-lib`. Each language needs its native parser
only when files of that language are selected for function analysis. A language
whose parser is not installed is skipped: the scan still succeeds and the
report names the language and what to install.

## Layout

`scan.mjs` lists the files and hands each one to the selected sections:
`styles.mjs`, `functions.mjs`, `coupling.mjs`, and the structure section in
`scan.mjs`. Each
section accepts the files it reads, visits them, and returns its data, errors,
and skipped languages. `config.mjs` holds the source roots, exclusions,
platforms, and function row shape they share. `native-functions.py` and
`KotlinFunctions.java` print one JSON object per line for `functions.mjs`.

## Examples

```sh
yarn quality:audit --styles --scope lms
yarn quality:audit --functions --scope exam --sort inputs --limit 15
yarn quality:audit --structure --changed
yarn quality:audit --styles --structure --base main
yarn quality:audit --coupling
yarn quality:audit --help
```

The section switches can be combined. Repeated `--scope` filters include
paths matching any fragment, ignoring case; end a fragment with `/` to match
one directory and not a sibling that shares its prefix (`apps/desktop/src/`
leaves out `apps/desktop/src-tauri/`). `--changed` compares the checkout
with the merge base against `origin/main` and includes untracked, nonignored
source files. Use `--base` to choose a different base ref. Deleted files have
no source left to scan. Markdown rankings honor `--limit`. The JSON file keeps
all candidate locations, function rows, and source counts for selected sections.

Style results include common token references and direct styling literals in
CSS, JSX inline styles, SwiftUI, and Compose. Function metrics count declared
parameters, top-level TS/TSX destructuring, explicit branches, and nesting. An
`else if` continues its chain at the same depth in every language, so a long
chain ranks by branches, not by nesting.
Structure results rank files and direct directory contents by nonblank lines.
Use these as review leads, then trace the calls and verify which code owns the
rule or component before reporting a finding. The scanner exits with a failure
status when a file does not parse or an installed parser fails, and the reports
list partial results and the errored files or languages.

Coupling results are leads for the coupling and duplication checks in
`docs/code-quality.md` §3. Desktop imports are resolved from the Babel AST
(the repository's TypeScript 7 has no JavaScript API for dependency-cruiser)
and checked against the layer rules in `coupling.mjs`. When the selection
includes desktop modules, the scanner computes violations, cycles and module
counts from the full desktop graph, then reports results that touch the
selected modules. Duplication and Git history use the selected files. jscpd
lists exact and renamed copies and
functions of the same structure in every scanned language, flagging pairs on
different platforms. Git history lists files in different directories that
change in the same commits and the most changed files; with few commits these
are weak signals.
