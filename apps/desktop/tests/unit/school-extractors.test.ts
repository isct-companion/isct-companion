import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { basename, dirname, join } from "node:path";
import { describe, expect, it } from "vitest";

import { readRepoFile, repositoryRoot } from "@tests/harness";

/**
 * The shipped 教務 reader, the LMS transport allowlist, the source-safety audit
 * and the provenance of the committed public fixtures.
 *
 * The extractor is production `kyomu-extract.js`, driven through a real DOM
 * built over the committed fixtures — nothing here re-implements it. The suite
 * runs in plain node and owns its JSDOM, so no jsdom environment is declared.
 */

type JsdomWindow = { document: Document; eval(source: string): unknown };
type JsdomInstance = { window: JsdomWindow; serialize(): string };

/**
 * `jsdom` ships no types and `@types/jsdom` is not a dependency. Loading it
 * through a locally typed `require` keeps this suite a single file with no
 * ambient declaration of a module the repository does not type.
 */
const { JSDOM } = createRequire(import.meta.url)("jsdom") as {
  JSDOM: new (
    html: string,
    options: { url: string; runScripts: "outside-only" },
  ) => JsdomInstance;
};

const ASSET_DIR = "crates/isct-core/src/school_browser/assets";
/** The LMS transport is `concat!`ed from these sibling modules. */
const LMS_MODULE_DIR = "crates/isct-core/src";
const LMS_MODULES = [
  "lms_course_content.js",
  "lms_forum_content.js",
  "lms_resource_resolve.js",
  "lms_submission_status.js",
] as const;

const assetPath = (name: string) => `${ASSET_DIR}/${name}`;
const asset = (name: string) => readRepoFile(assetPath(name));

/** The five approved 教務 paths the extractor maps to a page kind. */
const paths = {
  registrations:
    "/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0.aspx",
  declared: "/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_1.aspx",
  grades: "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_0.aspx",
  requirements:
    "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_6.aspx",
  gpa: "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_7.aspx",
};

type Extracted = {
  status: "ok" | "login" | "unavailable" | "error";
  sourceUrl: string;
  kind: string;
  message?: string;
  data: Record<string, unknown>;
};

const extractorScript = asset("kyomu-extract.js");

/**
 * Runs the production extractor against one page. `readyState` is forced to
 * complete because nothing in jsdom has actually finished loading, mirroring
 * how the script sees a fully rendered page in the browser.
 */
function runExtractor(
  path: string,
  html: string,
  host = "kyomu0.gakumu.titech.ac.jp",
): Extracted {
  const dom = new JSDOM(html, {
    url: `https://${host}${path}`,
    runScripts: "outside-only",
  });
  Object.defineProperty(dom.window.document, "readyState", {
    configurable: true,
    value: "complete",
  });
  return dom.window.eval(extractorScript) as Extracted;
}

/**
 * Reads a `&str` constant out of Rust exactly as the application ships it,
 * following `include_str!` and `concat!` so the audit sees the real script
 * bytes rather than a Rust source snapshot.
 */
function rustConstant(sourcePath: string, constant: string): string {
  const source = readRepoFile(sourcePath);
  const raw = source.match(
    new RegExp(`pub const ${constant}: &str = r#\\"([\\s\\S]*?)\\"#;`),
  );
  if (raw) return raw[1];
  const included = source.match(
    new RegExp(`pub const ${constant}: &str = include_str!\\("([^"]+)"\\);`),
  );
  if (included)
    return readRepoFile(
      join(dirname(sourcePath), included[1]).replaceAll("\\", "/"),
    );
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
    .map(
      (part) =>
        part[1] ??
        readRepoFile(join(dirname(sourcePath), part[2]).replaceAll("\\", "/")),
    )
    .join("");
}

const PAGE_MUTATION =
  /__doPostBack|\.submit\s*\(|\.click\s*\(|\$\s*\.ajax|sendBeacon/;
const KYOMU_NETWORK = /fetch\s*\(|XMLHttpRequest/;

describe("the production 教務 extractor reads the committed grade fixtures", () => {
  it("reads the manual-derived graduate two-table shape with expanded headers", () => {
    const fixture = asset("fixtures/kyomu-graduate-grades-manual.html");
    const result = runExtractor(
      paths.grades,
      `<table id="webforms-layout"><tr><td>${fixture}</td></tr></table>`,
    );
    expect(result.status).toBe("ok");
    expect(result.data).toMatchObject({
      complete: true,
      shape: "graduate-flat",
      empty: false,
      groups: [
        {
          section: "graduate",
          rows: [
            {
              title: "先端システム論",
              instructor: "架空 教員",
              credits: "2",
              result: "A",
              completed: "2026年度 1Q",
            },
          ],
        },
        {
          section: "undergraduate",
          rows: [{ title: "基礎設計演習" }],
        },
      ],
    });
  });

  it("keeps the observed undergraduate grouped shape working", () => {
    const result = runExtractor(
      paths.grades,
      asset("fixtures/kyomu-undergraduate-grades.html"),
    );
    expect(result).toMatchObject({
      status: "ok",
      data: {
        complete: true,
        shape: "undergraduate-grouped",
        groups: [
          {
            category: "文系教養科目",
            rows: [
              {
                title: "社会と技術",
                code: "LAH.H101",
                result: "90",
              },
            ],
          },
        ],
      },
    });
  });

  it("requires scoped empty evidence for every recognized grade section", () => {
    const flat = (graduateBody: string, undergraduateBody: string) => `
      <h2>大学院科目</h2><table><thead><tr><th>授業科目名</th><th>授業担当教員</th><th>単位</th><th>成績</th><th>修得時期</th></tr></thead><tbody>${graduateBody}</tbody></table>
      <h2>学部科目</h2><table><thead><tr><th>授業科目名</th><th>授業担当教員</th><th>単位</th><th>成績</th><th>修得時期</th></tr></thead><tbody>${undergraduateBody}</tbody></table>`;
    const row =
      "<tr><td>先端システム論</td><td>架空 教員</td><td>2</td><td>A</td><td>2026年度 1Q</td></tr>";
    const empty = '<tr><td colspan="5">対象データはありません</td></tr>';
    expect(runExtractor(paths.grades, flat("", ""))).toMatchObject({
      status: "error",
      message: "A flat grade table has no explicit rows.",
    });
    expect(runExtractor(paths.grades, flat(row, ""))).toMatchObject({
      status: "error",
      message: "A flat grade table has no explicit rows.",
    });
    expect(runExtractor(paths.grades, flat(empty, empty))).toMatchObject({
      status: "ok",
      data: { complete: true, empty: true },
    });

    const grouped = (body: string) => `
      <h3 id="resultUnit_110100_h3">文系教養科目</h3>
      <table id="resultUnit_110100_grid"><thead><tr><th>授業科目名</th><th>成績</th></tr></thead><tbody>${body}</tbody></table>`;
    expect(runExtractor(paths.grades, grouped(""))).toMatchObject({
      status: "error",
      message: "A grouped grade table has no explicit rows.",
    });
    expect(runExtractor(paths.grades, grouped(empty))).toMatchObject({
      status: "ok",
      data: { complete: true, empty: true },
    });
  });

  it("only accepts an empty grade list when the page content region says it is empty", () => {
    expect(
      runExtractor(
        paths.grades,
        '<main id="ctl00_ContentPlaceHolder1"><p>対象データはありません</p></main>',
      ),
    ).toMatchObject({
      status: "ok",
      data: { complete: true, shape: "graduate-flat", empty: true, groups: [] },
    });
    expect(
      runExtractor(
        paths.grades,
        "<aside>対象データはありません</aside><main>成績照会</main>",
      ),
    ).toMatchObject({ status: "error" });
    expect(runExtractor(paths.grades, "<main>成績照会</main>")).toMatchObject({
      status: "error",
      message: "The grade page shape is unknown.",
    });
  });

  it("rejects a recognized grade page while its DOM still reports loading", () => {
    const dom = new JSDOM(asset("fixtures/kyomu-graduate-grades-manual.html"), {
      url: `https://kyomu0.gakumu.titech.ac.jp${paths.grades}`,
      runScripts: "outside-only",
    });
    expect(dom.window.eval(extractorScript)).toMatchObject({
      status: "error",
      message: "The school page is still loading.",
    });
  });

  it("leaves every committed page byte-identical after it runs", () => {
    for (const name of [
      "kyomu-graduate-grades-manual.html",
      "kyomu-undergraduate-grades.html",
      "kyomu-requirements-three-tables.html",
    ]) {
      const path = name.includes("requirements")
        ? paths.requirements
        : paths.grades;
      const dom = new JSDOM(asset(`fixtures/${name}`), {
        url: `https://kyomu0.gakumu.titech.ac.jp${path}`,
        runScripts: "outside-only",
      });
      Object.defineProperty(dom.window.document, "readyState", {
        configurable: true,
        value: "complete",
      });
      const before = dom.serialize();
      dom.window.eval(extractorScript);
      expect(dom.serialize(), name).toBe(before);
    }
  });
});

describe("the production 教務 extractor reads the committed requirement fixture", () => {
  it("accepts semantic requirement tables instead of requiring exactly two", () => {
    const result = runExtractor(
      paths.requirements,
      asset("fixtures/kyomu-requirements-three-tables.html"),
    );
    expect(result.status).toBe("ok");
    expect(result.data).toMatchObject({
      complete: true,
      shape: "requirements-tables",
    });
    const tables = result.data.tables as Array<{
      headers: string[];
      rows: unknown[];
    }>;
    expect(tables).toHaveLength(3);
    expect(tables[0].headers).toEqual(["区分", "教養 / 文系", "教養 / 英語"]);
  });

  it("rejects one unknown requirement table beside recognized tables", () => {
    const html = asset("fixtures/kyomu-requirements-three-tables.html").replace(
      "</div>",
      '<table class="tblYouken"><tr><th>未知</th><th>列</th></tr><tr><td>値</td><td>1</td></tr></table></div>',
    );
    expect(runExtractor(paths.requirements, html)).toMatchObject({
      status: "error",
      message: "A requirement table has no recognized rows.",
    });
  });
});

describe("the production 教務 extractor reads registration pages", () => {
  it("rejects partial registration and declared-course structures", () => {
    expect(
      runExtractor(paths.registrations, '<div id="divSinkokuStatus"></div>'),
    ).toMatchObject({
      status: "error",
      message: "The registration course area is missing.",
    });
    const emptyRegistration = `
      <div id="divSinkokuStatus"></div>
      <table id="timetable"></table>
      <table id="kamokuUnit_1_gvKamoku"><tr><th>授業担当教員</th><th>備考</th></tr></table>`;
    expect(runExtractor(paths.registrations, emptyRegistration)).toMatchObject({
      status: "error",
      message: "The empty registration list is not explicit.",
    });
    expect(
      runExtractor(
        paths.registrations,
        emptyRegistration.replace(
          "</table>",
          "<tr><td>対象データはありません</td></tr></table>",
        ),
      ),
    ).toMatchObject({
      status: "ok",
      data: { complete: true, empty: true, courses: [] },
    });
    expect(
      runExtractor(
        paths.declared,
        '<table id="kamokuUnit_1_gvKamoku"><tr><th>授業科目名</th></tr><tr><td>構造変更後の行</td></tr></table>',
      ),
    ).toMatchObject({
      status: "error",
      message: "A declared-course row is incomplete.",
    });
  });
});

describe("the production 教務 extractor reads GPA pages", () => {
  it("requires the GPA total and term tables to both be complete", () => {
    const overall = `
      <table class="tblGPASummary mediaPC">
        <thead><tr><th colspan="4">全ての科目</th></tr><tr><th>GPA</th><th>GPT</th><th>修得済単位</th><th>合格率</th></tr></thead>
        <tbody><tr><td>3.20</td><td>100</td><td>10</td><td>90</td></tr></tbody>
      </table>`;
    expect(
      runExtractor(
        paths.gpa,
        `<main id="ctl00_ContentPlaceHolder1">${overall}</main>`,
      ),
    ).toMatchObject({
      status: "error",
      message: "The GPA term table is missing.",
    });
    const terms = `
      <table class="tblQ">
        <thead><tr><th rowspan="2">年度</th><th rowspan="2">修得時期</th><th>全ての科目</th></tr><tr><th>GPA</th></tr></thead>
        <tbody><tr><td>2026</td><td>1Q</td><td>3.20</td></tr></tbody>
      </table>`;
    expect(
      runExtractor(
        paths.gpa,
        `<main id="ctl00_ContentPlaceHolder1">${overall}${terms}</main>`,
      ),
    ).toMatchObject({
      status: "ok",
      data: {
        complete: true,
        shape: "gpa-summary",
        empty: false,
        terms: [["2026", "1Q", "3.20"]],
      },
    });
  });
});

describe("the production 教務 extractor judges page identity before parsing", () => {
  it("reports only an explicit not-offered message as unavailable", () => {
    expect(
      runExtractor(
        paths.requirements,
        '<div class="errorMessage">このページは利用できません</div>',
      ),
    ).toMatchObject({
      status: "unavailable",
      kind: "requirements",
      message: "このページは利用できません",
    });
  });

  it("rejects an unapproved origin instead of treating its login form as authentication", () => {
    expect(
      runExtractor(
        paths.grades,
        '<form action="/login"><input type="password"></form>',
        "example.invalid",
      ),
    ).toMatchObject({ status: "error", kind: "grades" });
  });

  it("recognizes an explicit login form on the approved identity hosts", () => {
    expect(
      runExtractor(
        "/login",
        '<form action="/login"><input type="password"></form>',
        "portal.titech.ac.jp",
      ),
    ).toMatchObject({ status: "login", kind: "unknown" });
  });
});

describe("the shipped school scripts stay read-only", () => {
  it("extracts the 教務 constant from its include_str! asset unchanged", () => {
    const script = rustConstant(
      "crates/isct-core/src/kyomu_ingest.rs",
      "KYOMU_EXTRACT_SCRIPT",
    );
    expect(script).toBe(asset("kyomu-extract.js"));
  });

  it("keeps the 教務 extractor free of any call that fetches or mutates the page", () => {
    const script = rustConstant(
      "crates/isct-core/src/kyomu_ingest.rs",
      "KYOMU_EXTRACT_SCRIPT",
    );
    expect(script).not.toMatch(PAGE_MUTATION);
    expect(script).not.toMatch(KYOMU_NETWORK);
    expect(script).toContain("querySelector");
    expect(script).toContain("querySelectorAll");
    // The emitted asset is the same bytes the constant includes.
    expect(asset("kyomu-extract.js")).not.toMatch(PAGE_MUTATION);
    expect(asset("kyomu-extract.js")).not.toMatch(KYOMU_NETWORK);
  });

  it("keeps the LMS transport and its included modules free of page-mutating calls", () => {
    const script = rustConstant(
      "crates/isct-core/src/lms_ingest.rs",
      "LMS_EXTRACT_SCRIPT",
    );
    expect(script).not.toMatch(PAGE_MUTATION);
    for (const name of LMS_MODULES) {
      const module = readRepoFile(`${LMS_MODULE_DIR}/${name}`);
      expect(module, name).not.toMatch(PAGE_MUTATION);
    }
  });

  it("closes the LMS AJAX method allowlist to exactly the seven documented methods", () => {
    const script = rustConstant(
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
  });

  it("checks the LMS allowlist before it builds any AJAX request", () => {
    const script = rustConstant(
      "crates/isct-core/src/lms_ingest.rs",
      "LMS_EXTRACT_SCRIPT",
    );
    const guard = script.indexOf("if (!methods.has(methodname))");
    expect(guard).toBeGreaterThan(-1);
    expect(guard).toBeLessThan(script.indexOf("fetch(endpoint"));
  });
});

describe("public syllabus fixture provenance", () => {
  const provenancePath =
    "crates/isct-core/tests/fixtures/syllabus/content-provenance.json";

  type ProvenanceEntry = {
    file: string;
    url: string;
    bytes: number;
    sha256: string;
  };
  const readProvenance = () =>
    JSON.parse(readRepoFile(provenancePath)) as {
      observedDate: string;
      fixtures: ProvenanceEntry[];
    };

  it("pins every public course-page fixture to an https URL on the official host", () => {
    const provenance = readProvenance();
    // A real ISO date, not just a string that looks like one.
    expect(provenance.observedDate).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    const observed = new Date(`${provenance.observedDate}T00:00:00.000Z`);
    expect(Number.isNaN(observed.getTime())).toBe(false);
    expect(observed.toISOString().slice(0, 10)).toBe(provenance.observedDate);

    expect(provenance.fixtures.length).toBeGreaterThan(0);
    for (const fixture of provenance.fixtures) {
      const url = new URL(fixture.url);
      expect(url.protocol, fixture.file).toBe("https:");
      expect(url.hostname, fixture.file).toBe("syllabus.s.isct.ac.jp");
    }
  });

  it("matches every committed fixture's canonical LF byte count and SHA-256", () => {
    const provenance = readProvenance();
    for (const fixture of provenance.fixtures) {
      const path = join(dirname(provenancePath), fixture.file);
      const absolute = join(repositoryRoot, path);
      expect(existsSync(absolute), basename(absolute)).toBe(true);
      // Git may check text fixtures out with CRLF on Windows. Provenance pins
      // the repository's canonical LF bytes, not a platform-specific checkout.
      const bytes = Buffer.from(
        readFileSync(absolute, "utf8").replaceAll("\r\n", "\n"),
      );
      expect(bytes.byteLength, fixture.file).toBe(fixture.bytes);
      expect(
        createHash("sha256").update(bytes).digest("hex"),
        fixture.file,
      ).toBe(fixture.sha256);
    }
  });
});

describe("bundled public subscription data", () => {
  type Source = { title: string; url: string };
  type Department = {
    id: string;
    name: { ja: string; en: string };
    syllabusScope: string;
    guidePdf: string;
    recommended: { code: string; group: string; mark?: string }[];
  };
  const readDepartments = (year: "2025" | "2026") =>
    JSON.parse(
      readRepoFile(
        `crates/isct-core/tests/fixtures/subscriptions/data/${year}/departments.json`,
      ),
    ) as {
      admissionYear: number;
      source: Source[];
      departments: Department[];
    };

  it("ships the 17 documented department guides with names and cited sources", () => {
    const data = readDepartments("2026");
    expect(data.source.length).toBeGreaterThan(0);
    for (const source of data.source) {
      expect(source.title.trim()).not.toBe("");
      const url = new URL(source.url);
      expect(url.protocol).toBe("https:");
      expect(url.hostname.endsWith("titech.ac.jp")).toBe(true);
    }
    expect(data.departments).toHaveLength(17);
    expect(
      new Set(data.departments.map((department) => department.id)).size,
    ).toBe(17);
    expect(
      data.departments.every(
        (department) =>
          department.name.ja.trim() !== "" &&
          department.name.en.trim() !== "" &&
          /^\d\/[^/]+$/.test(department.syllabusScope) &&
          department.guidePdf.startsWith("https://www.titech.ac.jp/"),
      ),
    ).toBe(true);
    expect(
      data.departments.find((department) => department.id === "cs")?.name.ja,
    ).toBe("情報工学系");
  });

  it("keeps the cited CS recommendation", () => {
    const data = readDepartments("2026");
    const cs = data.departments.find((department) => department.id === "cs");
    expect(cs?.recommended).toContainEqual(
      expect.objectContaining({
        code: "CSC.T241",
        group: "A",
        mark: "recommended",
      }),
    );
  });

  it("cites an official source for the academic-calendar day list", () => {
    const calendar = JSON.parse(
      readRepoFile(
        "crates/isct-core/tests/fixtures/subscriptions/data/2026/academic-calendar.json",
      ),
    ) as { source: Source[]; days: unknown[] };
    expect(calendar.days.length).toBeGreaterThan(0);
    expect(calendar.source.length).toBeGreaterThan(0);
    for (const source of calendar.source) {
      expect(source.title.trim()).not.toBe("");
      const url = new URL(source.url);
      expect(url.protocol).toBe("https:");
      expect(url.hostname.endsWith("titech.ac.jp")).toBe(true);
    }
  });

  it("keeps the two admission cohorts distinct", () => {
    const current = readDepartments("2026");
    const previous = readDepartments("2025");
    expect(current.admissionYear).toBe(2026);
    expect(previous.admissionYear).toBe(2025);
    expect(current.source[0].url).not.toBe(previous.source[0].url);
    const previousPdfs = new Set(
      previous.departments.map((department) => department.guidePdf),
    );
    for (const department of current.departments) {
      expect(previousPdfs.has(department.guidePdf), department.id).toBe(false);
    }
  });
});
