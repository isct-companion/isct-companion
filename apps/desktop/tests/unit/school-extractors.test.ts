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

const extractorScript = `${asset("kyomu-maintenance.js")}\n${asset("kyomu-extract.js")}`;

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

describe("official course discovery and academic identity", () => {
  it("discovers graduate filters and posts their live values, including new choices", async () => {
    // Screenshot-informed structure, synthetic IDs/values: this is not captured
    // authenticated graduate HTML. Changed server values must still round-trip.
    const group = (id: string, labels: string[]) =>
      `<fieldset id="${id}">${labels.map((label, index) => `<input type="checkbox" id="${id}_${index}" name="${id}$${index}" value="live-${index}"><label for="${id}_${index}">${label}</label>`).join("")}</fieldset>`;
    const form = `<p>2031/01/02 13:00からメンテナンス時間に入ります。</p><h1>科目の選択－大学院課程</h1><form>
      ${group("graduate_cblKamokuCodeSeries", ["400", "500", "600"])}
      ${group("graduate_cblHisshuKbn", ["R", "A", "B", "C", "D", "F", "I", "V", "標準学修課程外"])}
      ${group("graduate_cblYoubi", ["日", "月", "土", "未定", "オンデマンド", "医学系学際開講"])}
      ${group("graduate_cblKaisiJigenS", ["1時限", "11時限", "12時限"])}
      ${group("graduate_cblQuarters", ["1Q", "2Q", "3Q", "4Q", "1～2Q", "3～4Q", "1～4Q", "1・3Q", "2・4Q", "2～3Q"])}
      ${group("graduate_Lang", ["日本語", "英語"])}
      ${group("futureControls", ["新しい授業形式"])}
      <input type="checkbox" id="graduate_cblKamokuCodeSeries_disabled" disabled value="no"><label for="graduate_cblKamokuCodeSeries_disabled">900</label>
      <input type="checkbox" id="graduate_chkSuisho" name="graduate$recommendation" value="live-rec"><label for="graduate_chkSuisho">推奨</label>
      <a id="graduate_btnSearch"></a></form>
      <input type="checkbox" id="assignmentUpload" value="no"><label for="assignmentUpload">Not a filter</label>`;
    const grid = readRepoFile(
      "crates/isct-core/tests/fixtures/kyomu/course-search.html",
    );
    const dom = new JSDOM(form + grid, {
      url: `https://kyomu0.gakumu.titech.ac.jp${paths.registrations.replace("PID1_0.aspx", "PID1_0_1.aspx")}`,
      runScripts: "outside-only",
    });
    dom.window.eval(asset("kyomu-maintenance.js"));
    const metadata = (await dom.window.eval(
      asset("kyomu-search.js"),
    )) as Extracted;
    const options = metadata.data.searchOptions as {
      id: string;
      value: string;
      label: string;
      family: string;
    }[];
    expect(
      options
        .filter((option) => option.family === "level")
        .map((option) => option.label),
    ).toEqual(["400", "500", "600"]);
    expect(
      options
        .filter((option) => option.family === "requirement")
        .map((option) => option.label),
    ).toEqual(["R", "A", "B", "C", "D", "F", "I", "V", "標準学修課程外"]);
    expect(options.some((option) => option.label === "医学系学際開講")).toBe(
      true,
    );
    expect(
      options.some(
        (option) => option.label === "900" || option.id === "assignmentUpload",
      ),
    ).toBe(false);
    const custom = options.find((option) => option.label === "新しい授業形式")!;
    dom.window.eval(`globalThis.__ISCT_COMPANION_KYOMU_HIDDEN__=true;
      globalThis.__ISCT_COMPANION_KYOMU_PAGE__=${JSON.stringify({ groups: ["600"], quarters: [3], days: [7], periodStarts: [12], language: "en", requirementGroups: ["V"], controls: [custom] })};
      globalThis.fetch=async (_url,init)=>{globalThis.posted=Object.fromEntries(init.body);return {ok:true,url:location.href,text:async()=>${JSON.stringify(form + grid)}}};`);
    const result = (await dom.window.eval(
      asset("kyomu-search.js"),
    )) as Extracted;
    expect(result.status).toBe("ok");
    expect(dom.window.eval("posted")).toMatchObject({
      graduate_cblKamokuCodeSeries$2: "live-2",
      graduate_cblHisshuKbn$7: "live-7",
      graduate_cblYoubi$0: "live-0",
      graduate_cblKaisiJigenS$2: "live-2",
      graduate_Lang$1: "live-1",
      graduate_cblQuarters$2: "live-2",
      graduate_cblQuarters$5: "live-5",
      graduate_cblQuarters$6: "live-6",
      graduate_cblQuarters$7: "live-7",
      graduate_cblQuarters$9: "live-9",
      futureControls$0: "live-0",
    });
    // Recommendation defaults are read from the live page's own handler.
    dom.window
      .eval(`document.getElementById('graduate_chkSuisho').onclick=()=>{document.getElementById('graduate_cblHisshuKbn_0').checked=true};
      globalThis.__ISCT_COMPANION_KYOMU_PAGE__={controls:[{id:'graduate_chkSuisho',value:'live-rec'}]};`);
    expect(await dom.window.eval(asset("kyomu-search.js"))).toMatchObject({
      status: "ok",
    });
    expect(dom.window.eval("posted")).toMatchObject({
      graduate$recommendation: "live-rec",
      graduate_cblHisshuKbn$0: "live-0",
    });
    dom.window.eval(
      `globalThis.__ISCT_COMPANION_KYOMU_PAGE__={recommended:true,requirementGroups:['V']}`,
    );
    expect(await dom.window.eval(asset("kyomu-search.js"))).toMatchObject({
      status: "ok",
    });
    expect(dom.window.eval("posted")).toMatchObject({
      graduate_cblHisshuKbn$7: "live-7",
    });
    expect(
      dom.window.eval("posted['graduate_cblHisshuKbn$0']"),
    ).toBeUndefined();
    dom.window.eval(
      `globalThis.__ISCT_COMPANION_KYOMU_PAGE__={periodStarts:[11,10]};globalThis.fetch=()=>{throw Error('unsupported filters must not post')}`,
    );
    expect(await dom.window.eval(asset("kyomu-search.js"))).toMatchObject({
      status: "error",
    });
  });

  it("returns timing, rooms and requirements with search results, rejecting incomplete columns", async () => {
    const html = readRepoFile(
      "crates/isct-core/tests/fixtures/kyomu/course-search.html",
    );
    const provenance = JSON.parse(
      readRepoFile(
        "crates/isct-core/tests/fixtures/kyomu/content-provenance.json",
      ),
    );
    expect(createHash("sha256").update(html).digest("hex")).toBe(
      provenance.fixtures[0].sha256,
    );
    expect(Buffer.byteLength(html)).toBe(provenance.fixtures[0].bytes);
    // Minimal public controls with the IDs/values observed on the signed-in
    // form. The committed fixture is only its public result grid; keep account
    // fields and ASP.NET session state out of test assets.
    const controls = `<form>
      <div id="ctl00_ContentPlaceHolder1_cblKamokuCodeSeries">${[100, 200, 300].map((level, i) => `<input type="checkbox" id="ctl00_ContentPlaceHolder1_cblKamokuCodeSeries_${i}" name="ctl00$ContentPlaceHolder1$cblKamokuCodeSeries$${i}" value="on"><label for="ctl00_ContentPlaceHolder1_cblKamokuCodeSeries_${i}">${level}</label>`).join("")}</div>
      <div id="cblHisshuKbn"><input type="checkbox" id="cblHisshuKbn_8" value="on"><label for="cblHisshuKbn_8">標準学修課程外</label></div>
      <div id="ctl00_ContentPlaceHolder1_cblYoubi"><input type="checkbox" id="ctl00_ContentPlaceHolder1_cblYoubi_7" value="on"><label for="ctl00_ContentPlaceHolder1_cblYoubi_7">申告専用</label></div>
      <input type="checkbox" id="chkSuisho" name="ctl00$ContentPlaceHolder1$chkSuisho" value="on"><label for="chkSuisho">推奨</label>
      <select id="drpKamokuKbnMidashiSenmon" name="ctl00$ContentPlaceHolder1$drpKamokuKbnMidashiSenmon"><option value="">-</option><option value="121100">研究関連科目</option></select>
      <a id="ctl00_ContentPlaceHolder1_btnSearch"></a></form>`;
    const read = async (source: string) => {
      const dom = new JSDOM(controls + source, {
        url: "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0_1.aspx",
        runScripts: "outside-only",
      });
      return (await dom.window.eval(asset("kyomu-search.js"))) as Extracted;
    };
    const result = await read(html);
    expect(result.status).toBe("ok");
    const options = result.data.searchOptions as {
      id: string;
      value: string;
      label: string;
      family: string;
    }[];
    expect(
      options.filter((o) => o.family === "level").map((o) => o.label),
    ).toEqual(["100", "200", "300"]);
    expect(options).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          id: "chkSuisho",
          label: "推奨",
          family: "recommended",
        }),
        expect.objectContaining({
          label: "標準学修課程外",
          family: "requirement",
        }),
        expect.objectContaining({ label: "申告専用", family: "delivery" }),
        expect.objectContaining({ label: "研究関連科目", family: "category" }),
      ]),
    );
    // Search is a real official POST. Network has no headless server in this
    // fixture, so replace only fetch and inspect its form body and response.
    const dom = new JSDOM(controls + html, {
      url: "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0_1.aspx",
      runScripts: "outside-only",
    });
    const level = options.find(
      (o) => o.label === "200" && o.family === "level",
    )!;
    dom.window.eval(`globalThis.__ISCT_COMPANION_KYOMU_HIDDEN__ = true;
      globalThis.__ISCT_COMPANION_KYOMU_PAGE__ = ${JSON.stringify({ controls: [level] })};
      globalThis.fetch = async (_url, init) => {
        if (init.body.get('__EVENTTARGET') !== 'ctl00$ContentPlaceHolder1$btnSearch') throw Error('Unexpected postback');
        if (init.body.get('ctl00$ContentPlaceHolder1$cblKamokuCodeSeries$1') !== 'on') throw Error('Missing actual checkbox');
        return {ok:true, url:location.href, text: async () => ${JSON.stringify(controls + html)}};
      };`);
    expect(await dom.window.eval(asset("kyomu-search.js"))).toMatchObject({
      status: "ok",
    });
    dom.window.eval(
      `globalThis.__ISCT_COMPANION_KYOMU_PAGE__ = {groups:['200']}`,
    );
    expect(await dom.window.eval(asset("kyomu-search.js"))).toMatchObject({
      status: "ok",
    });
    dom.window.eval(
      `globalThis.__ISCT_COMPANION_KYOMU_PAGE__ = {groups:['600']}`,
    );
    expect(await dom.window.eval(asset("kyomu-search.js"))).toMatchObject({
      status: "error",
      message:
        "This filter is not offered by the current official search form.",
    });
    dom.window.eval(
      `globalThis.__ISCT_COMPANION_KYOMU_PAGE__ = {controls:[{id:'btnRegister',value:'on'}]}`,
    );
    expect(await dom.window.eval(asset("kyomu-search.js"))).toMatchObject({
      status: "error",
    });
    // A healthy initial document can redirect to an outage during the POST.
    dom.window.eval(asset("kyomu-maintenance.js"));
    dom.window
      .eval(`globalThis.__ISCT_COMPANION_KYOMU_PAGE__ = ${JSON.stringify({ controls: [level] })};
      globalThis.fetch = async () => ({ok:true, url:'https://kyomu0.gakumu.titech.ac.jp/Maintenance.aspx', text:async()=>'<main><h1>Under maintenance</h1><p>2031/02/03 12:00–14:00</p></main>'});`);
    expect(await dom.window.eval(asset("kyomu-search.js"))).toMatchObject({
      status: "maintenance",
      data: {},
      message: expect.stringContaining("2031/02/03"),
    });
    const courses = result.data.courses as Record<string, unknown>[];
    expect(courses).toHaveLength(14);
    expect(courses.find((c) => c.id === "202602431")).toMatchObject({
      code: "CSC.T273",
      quarters: ["3Q"],
      credits: { lecture: 1, exercise: 1, lab: 0 },
      schedule: [
        {
          kind: "weekly",
          day: 1,
          periods: [3, 4],
          room: { ja: "M-B07, 情報工学系計算機室" },
        },
        {
          kind: "weekly",
          day: 4,
          periods: [3, 4],
          room: { ja: "M-B07, 情報工学系計算機室" },
        },
      ],
      kyomu: { group: "C", mark: "recommended" },
    });
    expect(
      await read(html.replace(/授業担当教員/g, "changed-column")),
    ).toMatchObject({ status: "error" });
  });

  it("merges the rooms from repeated weekday cells and extracts only academic identity", () => {
    const block = (day: string) =>
      `<div data-jwc="202602431"><p class="ttKamokuName"><a>Course</a></p><p class="ttSinkokuNo">CSC.T273</p><p class="ttYoubi">3Q：月3-4${day === "月" ? " (M-B07)" : ""}<br>3Q：木3-4${day === "木" ? " (W9-324)" : ""}</p></div>`;
    const registration = runExtractor(
      paths.registrations,
      `<div id="divSinkokuStatus"></div><table id="timetable"><tr><td>${block("月")}</td><td>${block("木")}</td></tr></table><table id="kamokuUnit_1_gvKamoku"><tr><th>授業担当教員</th><th>備考</th></tr></table>`,
    );
    expect(registration).toMatchObject({
      status: "ok",
      data: {
        courses: [
          {
            schedule: [
              { quarter: "3Q", day: "月", periods: [3, 4], room: "M-B07" },
              { quarter: "3Q", day: "木", periods: [3, 4], room: "W9-324" },
            ],
          },
        ],
      },
    });
    const path =
      "/Titech/Student/%E5%AD%A6%E7%94%9F%E6%83%85%E5%A0%B1/PID4_2.aspx";
    const profile = runExtractor(
      path,
      `<div id="ctl00_ContentPlaceHolder1_divMain"><h2>【現在】 学士課程</h2><div><table><tr><th>所属</th><td>情報工学系</td></tr><tr><th>入学年月日</th><td>2025/04/01</td></tr><tr><th>入学試験区分</th><td>前期日程</td></tr><tr><th>氏名</th><td>PRIVATE-NAME</td></tr><tr><th>学籍番号</th><td>PRIVATE-ID</td></tr><tr><th>個人メールアドレス</th><td>private@example.invalid</td></tr></table></div></div>`,
    );
    expect(profile).toMatchObject({
      status: "ok",
      kind: "profile",
      data: {
        department: "情報工学系",
        admittedOn: "2025-04-01",
        programme: "学士課程",
      },
    });
    expect(JSON.stringify(profile)).not.toMatch(/PRIVATE|private@example/);
  });
});

it("emits at most 100 official courses per cursor and rejects a cursor reused for another query", async () => {
  const template = new JSDOM(
    readRepoFile("crates/isct-core/tests/fixtures/kyomu/course-search.html"),
    {
      url: "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0_1.aspx",
      runScripts: "outside-only",
    },
  );
  const grid = template.window.document.querySelector("table")!;
  const row = grid.querySelector('a[href*="jwc="]')!.closest("tr")!;
  const header = grid.querySelector("tr")!.outerHTML;
  grid.innerHTML =
    header +
    Array.from({ length: 205 }, (_, i) => {
      const copy = row.cloneNode(true) as HTMLTableRowElement;
      copy
        .querySelector("a")!
        .setAttribute(
          "href",
          `https://syllabus.s.isct.ac.jp/courses?jwc=${202600000 + i}`,
        );
      return copy.outerHTML;
    }).join("");
  const ids: string[] = [];
  let cursor: string | undefined;
  for (let page = 0; page < 3; page++) {
    const nextCursor = `00000000-0000-4000-8000-${String(page + 1).padStart(12, "0")}`;
    template.window.eval(
      `globalThis.__ISCT_COMPANION_KYOMU_HIDDEN__=true;globalThis.__ISCT_COMPANION_KYOMU_PAGE__=${JSON.stringify({ cursor, nextCursor })}`,
    );
    const result = (await template.window.eval(
      asset("kyomu-search.js"),
    )) as Extracted;
    expect(result.status).toBe("ok");
    const rows = result.data.courses as { id: string }[];
    expect(rows).toHaveLength(page < 2 ? 100 : 5);
    ids.push(...rows.map((row) => row.id));
    cursor = result.data.nextCursor as string | undefined;
    if (page < 2) expect(cursor).toBe(nextCursor);
  }
  expect(cursor).toBeUndefined();
  expect(new Set(ids).size).toBe(205);
  // The native boundary receives course rows and an opaque cursor, never form tokens.
  template.window.eval(
    `history.replaceState(null,'','?Name=another-query');globalThis.__ISCT_COMPANION_KYOMU_PAGE__={cursor:'00000000-0000-4000-8000-000000000002',nextCursor:'00000000-0000-4000-8000-000000000004'}`,
  );
  const stale = (await template.window.eval(
    asset("kyomu-search.js"),
  )) as Extracted;
  expect(stale).toMatchObject({
    status: "error",
    message: "The course search page expired. Refresh the search.",
  });
});

it("posts only grid paging events and keeps form state out of course results", async () => {
  const url =
    "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0_1.aspx";
  const dom = new JSDOM(
    readRepoFile("crates/isct-core/tests/fixtures/kyomu/course-search.html"),
    { url, runScripts: "outside-only" },
  );
  const grid = dom.window.document.querySelector("table")!;
  const row = grid.querySelector('a[href*="jwc="]')!.closest("tr")!;
  const header = grid.querySelector("tr")!.outerHTML;
  const target = "ctl00$ContentPlaceHolder1$gvGrid$ctl01$ctl03";
  const page = (start: number, count: number, more: boolean) => {
    const rows = Array.from({ length: count }, (_, index) => {
      const copy = row.cloneNode(true) as HTMLTableRowElement;
      copy
        .querySelector("a")!
        .setAttribute(
          "href",
          `https://syllabus.s.isct.ac.jp/courses?jwc=${202600000 + start + index}`,
        );
      return copy.outerHTML;
    }).join("");
    return `<form><input type="hidden" name="__VIEWSTATE" value="school-token"><input type="submit" name="btnSelect" value="追加申告する"><table id="ctl00_ContentPlaceHolder1_gvGrid">${header}${rows}${more ? `<tr><td><a href="javascript:__doPostBack('${target}','')">&gt;&gt;</a></td></tr>` : ""}</table></form>`;
  };
  const first = page(0, 100, true);
  const second = page(100, 5, false);
  dom.window.document.body.innerHTML = page(0, 10, false);
  const select = dom.window.document.createElement("select");
  select.id = "ctl00_ContentPlaceHolder1_gvGrid_ctl01_drpPageSize";
  select.name = "ctl00$ContentPlaceHolder1$gvGrid$ctl01$drpPageSize";
  select.innerHTML =
    '<option value="10">10</option><option value="100">100</option>';
  dom.window.document.querySelector("table")!.appendChild(select);
  // The network is the only dependency replaced: execute the shipped reader
  // over real DOM/FormData, with the university responses supplied at its port.
  dom.window.eval(
    `globalThis.responses=${JSON.stringify([first, second])};globalThis.posts=[];globalThis.fetch=async (url,options)=>{posts.push(Object.fromEntries(options.body));return {ok:true,url,text:async()=>responses.shift()}};globalThis.__ISCT_COMPANION_KYOMU_HIDDEN__=true;globalThis.__ISCT_COMPANION_KYOMU_PAGE__={nextCursor:'00000000-0000-4000-8000-000000000001'}`,
  );
  const a = (await dom.window.eval(asset("kyomu-search.js"))) as Extracted;
  expect(a.status).toBe("ok");
  expect(a.data.courses).toHaveLength(100);
  expect(JSON.stringify(a)).not.toContain("school-token");
  dom.window.eval(
    `globalThis.__ISCT_COMPANION_KYOMU_PAGE__={cursor:'00000000-0000-4000-8000-000000000001',nextCursor:'00000000-0000-4000-8000-000000000002'}`,
  );
  const b = (await dom.window.eval(asset("kyomu-search.js"))) as Extracted;
  expect(b.status).toBe("ok");
  expect(b.data.courses).toHaveLength(5);
  expect(b.data.nextCursor).toBeUndefined();
  const posts = dom.window.eval("posts") as Record<string, string>[];
  expect(posts.map((p) => p.__EVENTTARGET)).toEqual([select.name, target]);
  expect(posts[0][select.name]).toBe("100");
  expect(
    posts.every((p) => p.__VIEWSTATE === "school-token" && !("btnSelect" in p)),
  ).toBe(true);
});

describe("official maintenance notices", () => {
  it("reads live outage periods and recurring notes without a baked-in calendar", () => {
    const html = `<main><h1>ただいまメンテナンス中です</h1><h2>メンテナンス情報</h2>
      <p>下記期間中はログインできません。</p><b>2031/01/02 10:00 ～ 12:30</b>
      <textarea>毎週水曜日の6:00–7:00</textarea></main>`;
    const dom = new JSDOM(html, {
      url: "https://kyomu0.gakumu.titech.ac.jp/Maintenance.aspx",
      runScripts: "outside-only",
    });
    const result = dom.window.eval(extractorScript) as Extracted;
    expect(result.status).toBe("maintenance");
    expect(result.message).toContain("2031/01/02 10:00");
    expect(result.message).toContain("毎週水曜日の6:00");
    expect(result.data).toEqual({});
    const search = dom.window.eval(
      `${asset("kyomu-maintenance.js")}\n${asset("kyomu-search.js")}`,
    );
    return expect(search).resolves.toMatchObject({
      status: "maintenance",
      message: result.message,
    });
  });
  it("does not mistake future notices, course text, or foreign hosts for an outage", () => {
    for (const html of [
      "<h2>メンテナンス情報</h2><p>明日メンテナンスを行います</p>",
      "<h1>Course search</h1><p>Software maintenance</p>",
    ]) {
      const dom = new JSDOM(html, {
        url: "https://kyomu0.gakumu.titech.ac.jp/Maintenance.aspx",
        runScripts: "outside-only",
      });
      dom.window.eval(asset("kyomu-maintenance.js"));
      expect(dom.window.eval("__isctKyomuMaintenance(document)")).toBeNull();
    }
    const dom = new JSDOM("<h1>Under maintenance</h1>", {
      url: "https://example.org/",
      runScripts: "outside-only",
    });
    expect((dom.window.eval(extractorScript) as Extracted).status).toBe(
      "error",
    );
  });
});
