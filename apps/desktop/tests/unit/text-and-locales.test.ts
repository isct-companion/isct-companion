import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import i18n, { intlLocale, lookup, relativeTimeAgo, t } from "@app/i18n";
import { EMPTY_WORKSPACE } from "@app/generated/persistence";
import {
  LMS_BACKGROUND_REFRESH_TASK_ID,
  REFERENCE_REFRESH_TASK_ID,
  seedWorkspace,
} from "@app/lib/model";
import { decodeEntities } from "@app/lib/text";
import { readRepoFile, repositoryRoot } from "@tests/harness";

/**
 * School text arrives escaped (Core picks its language), and the workspace
 * starts from Core's defaults plus launch-local identity. The Chinese catalogue is checked against
 * the Japanese one because it is a translation of it, not a second source.
 */

/** `<area>.<subject>.<what>`: camelCase segments, so a key never reads as a sentence. */
const KEY_SHAPE = /^[a-z][A-Za-z0-9]*(\.[A-Za-z0-9-]+)+$/;

/** Keys no source spells out whole. Core rejects with a code and the shells
 * show `error.<code>`; every shell reads Core's sync label keys with `lookup`;
 * a campus map amenity is named after the key Core gives it. Past-exam labels
 * include the selected tab, document metadata and request status. */
const KEYS_BUILT_AT_RUNTIME = [
  "error.",
  "sync.run.",
  "sync.current.",
  "campusMap.amenity.",
  "examShare.(",
  "examShare.occasion.",
  "examShare.requestStatus.",
  "examShare.role.",
  "examShare.reason.",
];

/** Dotted strings in the sources that look like keys but are not: Core's
 * dataset ids, storage names, hosts and file names. */
const NOT_KEYS = new Set([
  "lms.enrolment",
  "lms.deadlines",
  "lms.course",
  "lms.assign",
  "lms.history",
  "lms.s.isct.ac.jp",
  "web.textScale",
  "web.firstSite",
  "guide.welcome",
  "guide.resetTips",
  "assignments.json",
  "app.campuslife.sciencetokyo",
]);
const isNotAKey = (value: string) =>
  NOT_KEYS.has(value) ||
  value.startsWith("app.isctcompanion") ||
  value.startsWith("sync.");

const SOURCE_ROOTS = [
  "apps/desktop/src",
  "apps/desktop/src-tauri/src",
  "apps/ios",
  "apps/android",
  // The OTP guide's step list names its captions.
  "resources/otp-guide",
];
const SOURCE_FILE = /\.(tsx?|rs|swift|kt)$|^resources\/otp-guide\/.*\.json$/;

/** Sources that spell keys: files in the working tree, including new files
 * before they are staged. Ignore deleted paths and generated code. */
function sources(): { file: string; text: string }[] {
  const listed = execFileSync(
    "git",
    [
      "ls-files",
      "-z",
      "--cached",
      "--others",
      "--exclude-standard",
      "--",
      ...SOURCE_ROOTS,
    ],
    { cwd: repositoryRoot, encoding: "utf8" },
  )
    .split("\0")
    .filter(
      (file) =>
        SOURCE_FILE.test(file) &&
        !file.includes("/generated/") &&
        !file.includes("/tests/") &&
        existsSync(join(repositoryRoot, file)),
    );
  return listed.map((file) => ({
    file,
    text: stripComments(readRepoFile(file)),
  }));
}

/** Drops whole-line comments, so a key quoted in prose does not count as use. */
const stripComments = (text: string) =>
  text
    .split("\n")
    .filter((line) => !/^\s*(\/\/|\/\*|\*)/.test(line))
    .join("\n");

/** Every double-quoted literal with its offset, escapes undone for `\"`. */
function* literals(text: string): Generator<[number, string, number]> {
  for (const match of text.matchAll(/"((?:[^"\\\n]|\\.)*)"/g))
    yield [match.index, match[1]!.replace(/\\(.)/g, "$1"), match[0].length];
}

/** The text of a call's first argument, opened at `open`: up to the first
 * comma outside brackets, strings skipped. */
function firstArgument(text: string, open: number): string {
  let depth = 0;
  let index = open + 1;
  const string = /"(?:[^"\\\n]|\\.)*"/y;
  while (index < text.length) {
    const char = text[index]!;
    if (char === '"') {
      string.lastIndex = index;
      index = string.exec(text) ? string.lastIndex : index + 1;
      continue;
    }
    if ("([{".includes(char)) depth++;
    if (")]}".includes(char)) {
      if (depth === 0) break;
      depth--;
    }
    if (char === "," && depth === 0) break;
    index++;
  }
  return text.slice(open + 1, index);
}

/** How many `[` the text leaves open. */
function opens(text: string): number {
  let open = 0;
  for (const char of text.replace(/"(?:[^"\\\n]|\\.)*"/g, '""')) {
    if (char === "[") open++;
    if (char === "]") open--;
  }
  return open;
}

/** The literals in a source that stand for a catalog key: whatever a call to
 * the lookup function is given (`t` and `lookup`, `english` in TypeScript,
 * `text` in the Rust shell; Swift and Kotlin placeholder names such as
 * `["count": n]` and `"count" to n` are not keys), and any literal shaped
 * like a key of one of the catalog's areas, which is how a label held in a
 * table or a constant is found. */
function usedKeys(file: string, text: string): string[] {
  const found = new Set<string>();
  const callee = file.endsWith(".rs")
    ? "text"
    : /\.tsx?$/.test(file)
      ? "t|lookup|english"
      : "t|lookup";
  const calls = new RegExp(`(?<![\\w.$])(?:${callee})\\(`, "g");
  for (const call of text.matchAll(calls)) {
    const open = call.index + call[0].length - 1;
    // The Rust shell passes the app first: `text(app, "key")`.
    const body = file.endsWith(".rs")
      ? text.slice(open + 1, text.indexOf(")", open))
      : firstArgument(text, open);
    for (const [at, value, length] of literals(body)) {
      // A literal inside brackets indexes a table or names a placeholder.
      if (opens(body.slice(0, at)) > 0) continue;
      const before = body.slice(Math.max(0, at - 3), at).trimEnd();
      const after = body.slice(at + length, at + length + 4).trimStart();
      // A comparison operand is not a key either.
      const compared = /[=!]=\s*$/.test(before) || /^[=!]=/.test(after);
      if (!compared) found.add(value);
    }
  }
  const areas = new Set(Object.keys(en).map((key) => key.split(".")[0]));
  for (const [, value] of literals(text))
    if (KEY_SHAPE.test(value) && areas.has(value.split(".")[0]!))
      found.add(value);
  return [...found];
}

/** The harness has no placeholder extractor, so this suite owns one. */
function placeholders(text: string): string[] {
  return [...text.matchAll(/{{[^}]+}}/g)].map((match) => match[0]).sort();
}

function loadLocale(name: "en" | "ja" | "zh"): Record<string, string> {
  return JSON.parse(readRepoFile(`resources/locales/${name}.json`)) as Record<
    string,
    string
  >;
}

const en = loadLocale("en");
const ja = loadLocale("ja");
const zh = loadLocale("zh");

describe("HTML entities in school text", () => {
  it("decodes every named entity the school pages emit, including nbsp and quot", () => {
    expect(decodeEntities("Archaeology &amp; Biological Anthropology C")).toBe(
      "Archaeology & Biological Anthropology C",
    );
    expect(decodeEntities("&lt;b&gt; &quot;x&quot; &apos;y&apos;")).toBe(
      `<b> "x" 'y'`,
    );
    expect(decodeEntities("A&nbsp;B")).toBe("A B");
    expect(decodeEntities("R&amp;D")).toBe("R&D");
  });

  it("decodes decimal and hexadecimal character references", () => {
    expect(decodeEntities("&#39;x&#39; &#x41;")).toBe("'x' A");
    expect(decodeEntities("&#12354;&#x3044;")).toBe("あい");
  });

  it("leaves text with no complete entity untouched", () => {
    expect(decodeEntities("R&D & more")).toBe("R&D & more");
    expect(decodeEntities("&unknown; &#; &; &amp")).toBe(
      "&unknown; &#; &; &amp",
    );
  });

  it("undoes exactly one level of escaping", () => {
    expect(decodeEntities("R&D & more &amp;amp; &unknown;")).toBe(
      "R&D & more &amp; &unknown;",
    );
  });

  it("leaves numeric references outside the Unicode range untouched", () => {
    expect(decodeEntities("&#0; &#x110000;")).toBe("&#0; &#x110000;");
  });
});

describe("workspace seeding", () => {
  const EMPTY_COLLECTIONS = [
    "courses",
    "memberships",
    "assignments",
    "materials",
    "events",
    "planItems",
    "exceptions",
    "intensiveSchedules",
    "academicCalendar",
    "gpaSummary",
    "departments",
    "recommendedCourses",
    "importantDates",
    "scheduleOverrides",
    "subscriptions",
  ] as const;

  it("keeps the Core seed schema, term, empty rows and scheduled-task defaults", () => {
    const workspace = seedWorkspace();
    expect(workspace.schemaVersion).toBe(3);
    for (const key of EMPTY_COLLECTIONS) {
      expect(workspace[key], key).toEqual([]);
    }
    expect(workspace.selectedTerm).toBe(EMPTY_WORKSPACE.selectedTerm);
    expect(workspace.selectedTerm).toMatch(/^\d{4}-[1-4]Q$/);
    expect(workspace.currentTerm).toBe(workspace.selectedTerm);
    expect(workspace.terms).toEqual([workspace.selectedTerm]);
    expect(workspace.scheduledTasks).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          id: REFERENCE_REFRESH_TASK_ID,
          action: "reference-refresh",
          enabled: true,
        }),
        expect.objectContaining({
          id: LMS_BACKGROUND_REFRESH_TASK_ID,
          action: "lms-refresh",
          enabled: false,
        }),
      ]),
    );
  });

  it("gives the workspace a launch-local device identity stamped on every default task", () => {
    const workspace = seedWorkspace();
    expect(workspace.deviceId).toBeTruthy();
    expect(workspace.deviceId).not.toBe(EMPTY_WORKSPACE.deviceId);
    expect(workspace.scheduledTasks.length).toBeGreaterThan(0);
    for (const task of workspace.scheduledTasks) {
      expect(task.updatedBy, task.id).toBe(workspace.deviceId);
      const stampedAt = Date.parse(task.updatedAt);
      expect(Number.isFinite(stampedAt), task.id).toBe(true);
      expect(Math.abs(Date.now() - stampedAt), task.id).toBeLessThan(60_000);
    }
  });

  it("gives each seed its own identity and its own copy of the defaults", () => {
    const first = seedWorkspace();
    const second = seedWorkspace();
    expect(second.deviceId).not.toBe(first.deviceId);

    first.courses.push({ id: "mutated" } as (typeof first.courses)[number]);
    expect(second.courses).toEqual([]);
    expect(seedWorkspace().courses).toEqual([]);
  });
});

describe("Chinese translation catalogue", () => {
  const keys = Object.keys(en);

  it("gives English, Japanese and Chinese exactly the same key set", () => {
    expect(Object.keys(ja).sort()).toEqual(keys.slice().sort());
    expect(Object.keys(zh).sort()).toEqual(keys.slice().sort());
  });

  it("names every key <area>.<subject>.<what> instead of by its English text", () => {
    expect(keys.filter((key) => !KEY_SHAPE.test(key))).toEqual([]);
  });

  it("leaves no value empty and keeps every interpolation placeholder in all three", () => {
    for (const key of keys) {
      for (const [language, catalog] of [
        ["en", en],
        ["ja", ja],
        ["zh", zh],
      ] as const) {
        expect(catalog[key]!.trim(), `${language} ${key}`).not.toBe("");
      }
      expect(placeholders(ja[key]!), `ja ${key}`).toEqual(
        placeholders(en[key]!),
      );
      expect(placeholders(zh[key]!), `zh ${key}`).toEqual(
        placeholders(en[key]!),
      );
    }
  });

  it("defines every key the desktop, iOS, Android and desktop-shell sources use", () => {
    const undefinedKeys = sources().flatMap(({ file, text }) =>
      usedKeys(file, text)
        .filter(
          (key) =>
            !(key in en) &&
            !isNotAKey(key) &&
            // A dynamic family still requires catalog entries for literal keys.
            !(
              KEYS_BUILT_AT_RUNTIME.some((p) => key.startsWith(p)) &&
              /[$(]/.test(key)
            ),
        )
        .map((key) => `${file}: ${key}`),
    );
    expect(undefinedKeys).toEqual([]);
  });

  it("maps each UI locale to the locale used for dates", () => {
    expect(intlLocale("en")).toBe("en-GB");
    expect(intlLocale("ja")).toBe("ja-JP");
    expect(intlLocale("zh")).toBe("zh-CN");
    expect(intlLocale("fr")).toBe("en-GB");
  });

  it("states a data age in the largest whole unit and leaves the first minute to the caller", () => {
    const now = "2026-10-17T12:00:00Z";
    expect(relativeTimeAgo("2026-10-17T11:59:30Z", now, "en")).toBeNull();
    expect(relativeTimeAgo("2026-10-17T11:48:00Z", now, "en")).toBe(
      "12 minutes ago",
    );
    expect(relativeTimeAgo("2026-10-17T09:10:00Z", now, "zh")).toBe("2小时前");
    expect(relativeTimeAgo("2026-10-16T08:00:00Z", now, "ja")).toBe("昨日");
    expect(relativeTimeAgo("not a time", now, "en")).toBeNull();
  });
});

describe("live translation lookups", () => {
  afterEach(() => {
    void i18n.changeLanguage("en");
  });

  it("translates through the live instance and interpolates stored values", async () => {
    await i18n.changeLanguage("zh");
    expect(t("common.today")).toBe("今天");
    expect(t("lms.countFiles", { count: 3 })).toBe("3 个文件");
  });

  it("reads the English catalogue for a key the selected language lacks", async () => {
    await i18n.changeLanguage("zh");
    const zhCatalog = i18n.getResourceBundle("zh", "translation");
    const saved = zhCatalog["common.today"];
    delete zhCatalog["common.today"];
    try {
      expect(t("common.today")).toBe("Today");
    } finally {
      zhCatalog["common.today"] = saved;
    }
    expect(lookup("not.a.key")).toBeUndefined();
  });
});
