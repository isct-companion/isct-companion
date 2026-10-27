import { afterEach, describe, expect, it } from "vitest";

import i18n, { intlLocale, relativeTimeAgo, t } from "@app/i18n";
import { EMPTY_WORKSPACE } from "@app/generated/persistence";
import { termForDate } from "@app/lib/academic";
import {
  LMS_BACKGROUND_REFRESH_TASK_ID,
  REFERENCE_REFRESH_TASK_ID,
  contentLanguage,
  pickLocalized,
  seedWorkspace,
} from "@app/lib/model";
import { decodeEntities } from "@app/lib/text";
import { readRepoFile } from "@tests/harness";

/**
 * School text arrives escaped, is published in a Japanese/English pair that the
 * UI language does not always decide, and the workspace starts from Core's
 * defaults plus launch-local identity. The Chinese catalogue is checked against
 * the Japanese one because it is a translation of it, not a second source.
 */

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
  it("decodes a named escape without disturbing the surrounding title", () => {
    expect(decodeEntities("Archaeology &amp; Biological Anthropology C")).toBe(
      "Archaeology & Biological Anthropology C",
    );
  });

  it("decodes every named entity the school pages emit, including nbsp and quot", () => {
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

describe("localized school text selection", () => {
  it("returns the published text for the selected UI language", () => {
    const both = { ja: "日本語", en: "English" };
    expect(pickLocalized(both, "ja")).toBe("日本語");
    expect(pickLocalized(both, "en")).toBe("English");
  });

  it("treats a Chinese UI as Japanese-first for school text", () => {
    expect(pickLocalized({ ja: "日本語", en: "English" }, "zh")).toBe("日本語");
  });

  it("prefers English for a UI language that is neither Japanese nor Chinese", () => {
    expect(pickLocalized({ ja: "日本語", en: "English" }, "fr")).toBe(
      "English",
    );
  });

  it("falls back to the other published language rather than inventing a translation", () => {
    expect(pickLocalized({ ja: "日本語" }, "en")).toBe("日本語");
    expect(pickLocalized({ en: "English" }, "zh")).toBe("English");
  });

  it("returns the caller's fallback when nothing is published", () => {
    expect(pickLocalized({}, "en", "Fallback")).toBe("Fallback");
    expect(pickLocalized(undefined, "en", "Fallback")).toBe("Fallback");
    expect(pickLocalized(undefined, "en")).toBe("");
  });

  it("skips blank published strings and trims the text it returns", () => {
    expect(pickLocalized({ ja: "   ", en: "English" }, "ja")).toBe("English");
    expect(pickLocalized({ ja: " 日本語 " }, "ja")).toBe("日本語");
  });
});

describe("school content language", () => {
  it("follows the UI language when no explicit preference is stored", () => {
    expect(contentLanguage({ locale: "ja" })).toBe("ja");
    expect(contentLanguage({ locale: "en" })).toBe("en");
    expect(contentLanguage({ locale: "zh" })).toBe("ja");
  });

  it("honours an explicit preference over the UI language", () => {
    expect(contentLanguage({ locale: "zh", contentLanguage: "en" })).toBe("en");
    expect(contentLanguage({ locale: "zh", contentLanguage: "ja" })).toBe("ja");
    expect(contentLanguage({ locale: "en", contentLanguage: "ja" })).toBe("ja");
    expect(contentLanguage({ locale: "ja", contentLanguage: "en" })).toBe("en");
  });

  it("treats an explicit auto as the UI-derived default", () => {
    expect(contentLanguage({ locale: "zh", contentLanguage: "auto" })).toBe(
      "ja",
    );
    expect(contentLanguage({ locale: "en", contentLanguage: "auto" })).toBe(
      "en",
    );
    expect(contentLanguage({ locale: "ja", contentLanguage: "auto" })).toBe(
      "ja",
    );
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
    "plans",
    "academicCalendar",
    "gpaSummary",
    "departments",
    "recommendedCourses",
    "importantDates",
    "scheduleOverrides",
    "subscriptions",
  ] as const;

  it("starts a real workspace at schema version 3 with no school or personal rows", () => {
    const workspace = seedWorkspace();
    expect(workspace.schemaVersion).toBe(3);
    for (const key of EMPTY_COLLECTIONS) {
      expect(workspace[key], key).toEqual([]);
    }
  });

  it("seeds the term from the Tokyo academic calendar and selects it", () => {
    const workspace = seedWorkspace();
    const now = Date.now();
    const around = [
      termForDate(new Date(now - 1000)),
      termForDate(new Date(now + 1000)),
    ];
    expect(around).toContain(workspace.selectedTerm);
    expect(workspace.selectedTerm).toMatch(/^\d{4}-[1-4]Q$/);
    expect(workspace.currentTerm).toBe(workspace.selectedTerm);
    expect(workspace.terms).toEqual([workspace.selectedTerm]);
  });

  it("enables the reference refresh task and leaves the LMS background refresh off", () => {
    const workspace = seedWorkspace();
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
  const keys = Object.keys(ja);

  it("covers exactly the Japanese key set", () => {
    expect(Object.keys(zh).sort()).toEqual(keys.slice().sort());
  });

  it("leaves no key untranslated and keeps every interpolation placeholder", () => {
    for (const [key, value] of Object.entries(zh)) {
      expect(value.trim(), key).not.toBe("");
      // An English key is its own copy; a namespaced key (`error.*`) has
      // its English copy in en.json.
      expect(placeholders(value), key).toEqual(placeholders(en[key] ?? key));
    }
  });

  it("contains real Chinese text rather than copies of the Japanese or English entries", () => {
    const copiedFromJapanese = keys.filter((key) => zh[key] === ja[key]);
    const copiedFromEnglish = keys.filter((key) => zh[key] === key);
    // A handful of entries are genuinely the same word or a proper noun.
    expect(copiedFromJapanese.length).toBeLessThan(keys.length * 0.05);
    expect(copiedFromEnglish.length).toBeLessThan(keys.length * 0.05);
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
    expect(t("Home")).toBe("首页");
    expect(t("{{count}} files", { count: 3 })).toBe("3 个文件");
  });

  it("returns the key itself when the English catalogue has no entry", async () => {
    await i18n.changeLanguage("en");
    expect(t("Home")).toBe("Home");
  });
});
