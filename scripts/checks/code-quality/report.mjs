import { resolve } from "node:path";
import { metricLabels, platforms } from "./config.mjs";

const code = (value) => `\`${String(value).replaceAll("`", "ˋ")}\``;

export function renderMarkdown(report, limit) {
  const link = ({ file, line }) =>
    `[${file}${line ? `:${line}` : ""}](<${resolve(report.repo, file)}${line ? `:${line}` : ""}>)`;
  const lines = [
    "# Code quality audit",
    "",
    `Selected ${report.fileCount} source files.`,
    `Sections: ${report.sections.join(", ")}.`,
  ];
  if (report.scope.length)
    lines.push(`Path filters: ${report.scope.map(code).join(", ")}.`);
  if (report.base) lines.push(`Change set starts at ${code(report.base)}.`);
  if (report.styles) lines.push(...styleSection(report.styles, link, limit));
  if (report.functions)
    lines.push(...functionSection(report.functions, report.sort, link, limit));
  if (report.structure)
    lines.push(...structureSection(report.structure, link, limit));
  if (report.coupling)
    lines.push(...couplingSection(report.coupling, link, limit));
  if (report.skipped.length) {
    lines.push("", "## Skipped languages", "");
    for (const { languages, reason } of report.skipped)
      lines.push(`- ${languages.join(", ")} · ${reason}`);
  }
  if (report.errors.length) {
    lines.push("", "## Scan errors", "");
    for (const { file, message } of report.errors)
      lines.push(`- ${file ? `${link({ file })} · ` : ""}${message}`);
  }
  return `${lines.join("\n")}\n`;
}

function styleSection({ findings, repeated, tokenReferences }, link, limit) {
  const lines = [
    "",
    "## UI style candidates",
    "",
    `${findings.length} candidates. Native matching covers direct arguments to common UI APIs. Check the role, platform conventions, and layout geometry in context.`,
    "",
    "### Common theme and token references",
    "",
  ];
  for (const { name: platform } of platforms) {
    const top = tokenReferences
      .filter((ref) => ref.platform === platform)
      .slice(0, limit);
    if (top.length)
      lines.push(
        `- ${platform} · ${top.map((ref) => `${code(ref.name)} (${ref.count})`).join(", ")}`,
      );
  }
  lines.push("", "### Values repeated across files", "");
  for (const group of repeated.slice(0, limit)) {
    const { platform, rule, property, value } = group[0];
    const distinct = [...new Map(group.map((hit) => [hit.file, hit])).values()];
    lines.push(
      `- ${platform} · ${rule} · ${code(property)} ${code(value)} · ${group.length} occurrences in ${distinct.length} files · ${distinct.slice(0, 3).map(link).join(", ")}`,
    );
  }
  lines.push("", "### Candidate locations", "");
  for (const hit of findings) {
    lines.push(
      `- ${hit.platform} · ${hit.rule} · ${code(hit.property)} ${code(hit.value)} · ${code(hit.code)} · ${link(hit)}`,
    );
  }
  return lines;
}

function functionSection({ rows, summary }, sort, link, limit) {
  const lines = [
    "",
    "## Function size and control flow",
    "",
    `${rows.length} functions, methods, constructors and computed properties. Named local functions have their own rows. Inline callbacks contribute to the enclosing function's control flow.`,
    "Expanded inputs include top-level destructuring in TS/TSX signatures. Branches count explicit conditions, loops, match or switch arms, and catches. Lines include signatures, blank lines, and comments within the function span.",
  ];
  if (!rows.length) return lines;
  lines.push(
    "",
    `| Language | Functions | ${Object.values(metricLabels).join(" | ")} |`,
    `| --- |${" ---: |".repeat(Object.keys(metricLabels).length + 1)}`,
  );
  for (const { language, count, metrics } of summary) {
    const cells = Object.values(metrics).map(
      ({ p50, p90, max }) => `${p50} / ${p90} / ${max}`,
    );
    lines.push(`| ${language} | ${count} | ${cells.join(" | ")} |`);
  }
  lines.push(
    "",
    `Metric columns show median / P90 / maximum. Rankings use ${code(sort)}. The JSON report contains every row.`,
  );
  for (const { language } of summary) {
    lines.push("", `### ${language}`, "");
    for (const item of rows
      .filter((row) => row.language === language)
      .slice(0, limit)) {
      const inputCount =
        item.inputs === item.parameters
          ? ""
          : ` (${item.inputs} expanded inputs)`;
      lines.push(
        `- ${code(item.name)} · ${item.lines} lines · ${item.parameters} parameters${inputCount} · ${item.branches} branches · nesting ${item.nesting} · ${link(item)}`,
      );
    }
  }
  return lines;
}

function structureSection({ files, directories }, link, limit) {
  const lines = ["", "## Source structure", "", "### Largest source files", ""];
  for (const item of files.slice(0, limit))
    lines.push(
      `- ${item.platform} · ${item.lines} nonblank lines · ${link(item)}`,
    );
  lines.push(
    "",
    "### Largest source directories",
    "",
    "Counts include files directly in each directory.",
    "",
  );
  for (const item of directories.slice(0, limit)) {
    lines.push(
      `- ${item.platform} · ${item.files} files · ${item.lines} nonblank lines · ${link({ file: item.directory })}`,
    );
  }
  return lines;
}

function couplingSection({ dependencies, duplicates, history }, link, limit) {
  const span = ({ file, line }) => link({ file, line });
  const lines = [
    "",
    "## Coupling and duplication",
    "",
    "Leads for docs/code-quality.md §3. Read both sides before calling a pair one concept (§4).",
    "",
    "### Desktop layer violations",
    "",
  ];
  const byRule = Map.groupBy(dependencies.violations, (item) => item.rule);
  for (const [rule, items] of byRule) {
    lines.push(`- ${rule} · ${items.length} imports · ${items[0].why}`);
    for (const item of items.slice(0, limit))
      lines.push(`  - ${link({ file: item.file })} → ${code(item.target)}`);
  }
  lines.push("", "### Desktop import cycles", "");
  for (const cycle of dependencies.cycles.slice(0, limit))
    lines.push(`- ${cycle.length} modules · ${cycle.map(code).join(" · ")}`);
  lines.push("", "### Desktop modules with the most internal imports", "");
  for (const item of dependencies.modules.slice(0, limit))
    lines.push(
      `- ${item.fanOut} imports · ${item.fanIn} importers · ${link(item)}`,
    );
  if (duplicates) {
    const cross = duplicates.rows.filter((row) => row.crossPlatform);
    lines.push(
      "",
      "### Duplicated code",
      "",
      `${duplicates.rows.length} clones, ${duplicates.percentage.toFixed(1)}% of lines. Kinds: exact copies, renamed copies, and functions of the same structure (ast).`,
      "",
    );
    for (const row of duplicates.rows
      .filter((item) => !item.crossPlatform)
      .slice(0, limit))
      lines.push(
        `- ${row.kind} · ${row.lines} lines · ${span(row.first)} · ${span(row.second)}`,
      );
    lines.push("", "### Code repeated across platforms", "");
    for (const row of cross.slice(0, limit))
      lines.push(
        `- ${row.kind} · ${row.lines} lines · ${row.first.platform} ${span(row.first)} · ${row.second.platform} ${span(row.second)}`,
      );
  }
  if (history) {
    lines.push(
      "",
      "### Files in different directories that change together",
      "",
      `From the last ${history.commits} commits. Degree is the share of the rarer file's commits that include the other.`,
      "",
    );
    for (const pair of history.coChanges.slice(0, limit))
      lines.push(
        `- ${pair.together} commits · degree ${pair.degree}${pair.crossPlatform ? " · across platforms" : ""} · ${pair.files.map((file) => link({ file })).join(" · ")}`,
      );
    lines.push("", "### Most changed files", "");
    for (const item of history.hotspots.slice(0, limit))
      lines.push(
        `- ${item.platform} · ${item.commits} commits · ${link(item)}`,
      );
  }
  return lines;
}
