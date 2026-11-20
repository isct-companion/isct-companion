// What the audit scans and how its rows are shaped. Every section and the
// report read these, so a new root, platform or metric is added here once.

export const sourceRoots = [
  "apps/desktop/src",
  "apps/desktop/src-tauri/src",
  "apps/ios",
  "apps/android/app/src/main",
  "apps/apple-widgets",
  "bindings/apple-widgets/Sources",
  "crates",
];

export const sourceExtensions = /\.(?:css|tsx?|swift|kt|rs)$/;

// Paths that are not hand-written product code.
const excludedPaths = [
  /(?:^|\/)generated\//i, // design tokens and bindings written by generators
  /(?:^|\/)tests\//i, // test code, measured by its own suites
  /(?:^|\/)vendor\//i, // third-party sources
  /(?:^|\/)(?:\.build|target)\//i, // SwiftPM and Cargo build output
  /\/app\/isct\/design\//, // Android token classes from design/build.mjs
  /\/IsctWidgets\/IsctTokens\.swift$/, // widget tokens from design/build.mjs
  /\/course_palette\.rs$/, // Core course palette from design/build.mjs
  /\.gen\.rs$/, // other generated Rust
];

export const isAuditedSource = (file) =>
  sourceExtensions.test(file) && !excludedPaths.some((path) => path.test(file));

// Order is the order the report lists them in.
export const platforms = [
  { name: "Desktop", owns: (file) => file.startsWith("apps/desktop/") },
  { name: "iOS", owns: (file) => file.startsWith("apps/ios/") },
  { name: "Android", owns: (file) => file.startsWith("apps/android/") },
  {
    name: "Apple Widgets",
    owns: (file) => /^(?:apps|bindings)\/apple-widgets\//.test(file),
  },
  { name: "Rust crates", owns: (file) => file.startsWith("crates/") },
];

export const platformFor = (file) =>
  platforms.find((platform) => platform.owns(file))?.name ?? "Other";

export const languages = ["TS/TSX", "Rust", "Swift", "Kotlin"];

export const metricLabels = {
  lines: "Lines",
  parameters: "Parameters",
  inputs: "Expanded inputs",
  branches: "Branches",
  nesting: "Nesting",
};

// One row shape for every language. Native parsers have no destructuring in
// signatures, so their expanded inputs equal their parameters.
export function functionRow({
  file,
  name,
  language,
  line,
  endLine,
  parameters,
  inputs = parameters,
  branches,
  nesting,
}) {
  return {
    file,
    name,
    language,
    line,
    endLine,
    lines: endLine - line + 1,
    parameters,
    inputs,
    branches,
    nesting,
  };
}
