import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const root = fileURLToPath(new URL("../../../../", import.meta.url));
const read = (path: string) => readFileSync(join(root, path), "utf8");

function rustRawString(sourcePath: string, constant: string): string {
  const source = read(sourcePath);
  const match = source.match(
    new RegExp(`pub const ${constant}: &str = r#\\"([\\s\\S]*?)\\"#;`),
  );
  if (match) return match[1];
  const included = source.match(
    new RegExp(`pub const ${constant}: &str = include_str!\\("([^"]+)"\\);`),
  );
  if (included) return read(join(dirname(sourcePath), included[1]));
  const start = source.indexOf(`pub const ${constant}: &str = concat!(`);
  const ending =
    start < 0 ? undefined : source.slice(start).match(/\}\)\(\)"#\s*\);/);
  if (start < 0 || !ending || ending.index === undefined)
    throw new Error(`Rust script ${constant} was not found.`);
  const expression = source.slice(
    start,
    start + ending.index + ending[0].length,
  );
  return [...expression.matchAll(/r#"([\s\S]*?)"#|include_str!\("([^"]+)"\)/g)]
    .map((part) => part[1] ?? read(join(dirname(sourcePath), part[2])))
    .join("");
}

describe("public syllabus fixture provenance", () => {
  it("pins every public course-page fixture to an official URL, date, and digest", () => {
    const provenancePath = join(
      root,
      "crates/isct-core/tests/fixtures/syllabus/content-provenance.json",
    );
    const provenance = JSON.parse(readFileSync(provenancePath, "utf8")) as {
      observedDate: string;
      fixtures: { file: string; url: string; bytes: number; sha256: string }[];
    };

    expect(provenance.observedDate).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    expect(provenance.fixtures.length).toBeGreaterThan(0);
    for (const fixture of provenance.fixtures) {
      const url = new URL(fixture.url);
      expect(url.protocol).toBe("https:");
      expect(url.hostname).toBe("syllabus.s.isct.ac.jp");
      const path = join(dirname(provenancePath), fixture.file);
      expect(existsSync(path), basename(path)).toBe(true);
      // Git may check text fixtures out with CRLF on Windows. Provenance pins
      // the repository's canonical LF bytes, not a platform-specific checkout.
      const bytes = Buffer.from(
        readFileSync(path, "utf8").replaceAll("\r\n", "\n"),
      );
      expect(bytes.byteLength, fixture.file).toBe(fixture.bytes);
      expect(
        createHash("sha256").update(bytes).digest("hex"),
        fixture.file,
      ).toBe(fixture.sha256);
    }
  });
});

describe("authenticated school extractor safety", () => {
  it("keeps the 教務 extractor DOM-read-only", () => {
    const script = rustRawString(
      "crates/isct-core/src/kyomu_ingest.rs",
      "KYOMU_EXTRACT_SCRIPT",
    );
    expect(script).not.toMatch(
      /__doPostBack|\.submit\s*\(|\.click\s*\(|\$\s*\.ajax|fetch\s*\(|XMLHttpRequest|sendBeacon/,
    );
    expect(script).toContain("querySelector");
    expect(script).toContain("querySelectorAll");
  });

  it("keeps the LMS AJAX method allowlist closed and exact", () => {
    const script = rustRawString(
      "crates/isct-core/src/lms_ingest.rs",
      "LMS_EXTRACT_SCRIPT",
    );
    const allowlistBody = script.match(
      /const methods = new Set\(\[([\s\S]*?)\]\);/,
    )?.[1];
    expect(allowlistBody).toBeDefined();
    const methods = [...(allowlistBody ?? "").matchAll(/'([^']+)'/g)].map(
      (match) => match[1],
    );
    expect(methods).toEqual([
      "core_course_get_enrolled_courses_by_timeline_classification",
      "core_courseformat_get_state",
      "core_calendar_get_action_events_by_timesort",
      "core_calendar_get_action_events_by_course",
      "core_calendar_get_calendar_upcoming_view",
      "core_calendar_get_calendar_monthly_view",
      "core_course_get_recent_courses",
    ]);
    expect(script.indexOf("if (!methods.has(methodname))")).toBeGreaterThan(-1);
    expect(script.indexOf("if (!methods.has(methodname))")).toBeLessThan(
      script.indexOf("fetch(endpoint"),
    );
  });
});

describe("bundled public subscription data", () => {
  it("ships the 17 documented departments and the cited CS recommendation", () => {
    const data = JSON.parse(
      read(
        "crates/isct-core/tests/fixtures/subscriptions/data/2026/departments.json",
      ),
    ) as {
      source: { url: string }[];
      departments: {
        id: string;
        syllabusScope: string;
        guidePdf: string;
        recommended: { code: string; group: string; mark?: string }[];
      }[];
    };
    expect(data.source.length).toBeGreaterThan(0);
    expect(
      data.source.every((source) => source.url.startsWith("https://")),
    ).toBe(true);
    expect(data.departments).toHaveLength(17);
    expect(
      new Set(data.departments.map((department) => department.id)).size,
    ).toBe(17);
    expect(
      data.departments.every(
        (department) =>
          /^\d\/[^/]+$/.test(department.syllabusScope) &&
          department.guidePdf.startsWith("https://www.titech.ac.jp/"),
      ),
    ).toBe(true);
    const cs = data.departments.find((department) => department.id === "cs");
    expect(cs?.recommended).toContainEqual(
      expect.objectContaining({
        code: "CSC.T241",
        group: "A",
        mark: "recommended",
      }),
    );
  });

  it("cites an official source for every non-empty academic-calendar day list", () => {
    const calendar = JSON.parse(
      read(
        "crates/isct-core/tests/fixtures/subscriptions/data/2026/academic-calendar.json",
      ),
    ) as {
      source: { title: string; url: string }[];
      days: unknown[];
    };
    if (calendar.days.length > 0) {
      expect(calendar.source.length).toBeGreaterThan(0);
      for (const source of calendar.source) {
        expect(source.title.trim()).not.toBe("");
        const url = new URL(source.url);
        expect(url.protocol).toBe("https:");
        expect(url.hostname.endsWith("titech.ac.jp")).toBe(true);
      }
    }
  });
});
