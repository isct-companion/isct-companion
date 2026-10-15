// @vitest-environment node
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";
import { describe, expect, it } from "vitest";

type Extracted = {
  status: "ok" | "login" | "unavailable" | "error";
  sourceUrl: string;
  kind: string;
  message?: string;
  data: Record<string, unknown>;
};

const asset = (name: string) =>
  readFileSync(
    new URL(
      `../../../../crates/isct-core/src/school_browser/assets/${name}`,
      import.meta.url,
    ),
    "utf8",
  );
const script = asset("kyomu-extract.js");
const paths = {
  registrations:
    "/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0.aspx",
  declared: "/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_1.aspx",
  grades: "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_0.aspx",
  requirements:
    "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_6.aspx",
  gpa: "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_7.aspx",
};
const run = (
  path: string,
  html: string,
  host = "kyomu0.gakumu.titech.ac.jp",
) => {
  const dom = new JSDOM(html, {
    url: `https://${host}${path}`,
    runScripts: "outside-only",
  });
  Object.defineProperty(dom.window.document, "readyState", {
    configurable: true,
    value: "complete",
  });
  return dom.window.eval(script) as Extracted;
};

describe("the production 教務 extractor", () => {
  it("reads the manual-derived graduate two-table shape with expanded headers", () => {
    const fixture = asset("fixtures/kyomu-graduate-grades-manual.html");
    const result = run(
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
    const result = run(
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
    expect(run(paths.grades, flat("", ""))).toMatchObject({
      status: "error",
      message: "A flat grade table has no explicit rows.",
    });
    expect(run(paths.grades, flat(row, ""))).toMatchObject({
      status: "error",
      message: "A flat grade table has no explicit rows.",
    });
    expect(run(paths.grades, flat(empty, empty))).toMatchObject({
      status: "ok",
      data: { complete: true, empty: true },
    });

    const grouped = (body: string) => `
      <h3 id="resultUnit_110100_h3">文系教養科目</h3>
      <table id="resultUnit_110100_grid"><thead><tr><th>授業科目名</th><th>成績</th></tr></thead><tbody>${body}</tbody></table>`;
    expect(run(paths.grades, grouped(""))).toMatchObject({
      status: "error",
      message: "A grouped grade table has no explicit rows.",
    });
    expect(run(paths.grades, grouped(empty))).toMatchObject({
      status: "ok",
      data: { complete: true, empty: true },
    });
  });

  it("accepts semantic requirement tables instead of requiring exactly two", () => {
    const result = run(
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

  it("only accepts an empty grade list when the page says it is empty", () => {
    expect(
      run(
        paths.grades,
        '<main id="ctl00_ContentPlaceHolder1"><p>対象データはありません</p></main>',
      ),
    ).toMatchObject({
      status: "ok",
      data: { complete: true, shape: "graduate-flat", empty: true, groups: [] },
    });
    expect(
      run(
        paths.grades,
        "<aside>対象データはありません</aside><main>成績照会</main>",
      ),
    ).toMatchObject({ status: "error" });
    expect(run(paths.grades, "<main>成績照会</main>")).toMatchObject({
      status: "error",
      message: "The grade page shape is unknown.",
    });
  });

  it("rejects partial registration and declared-course structures", () => {
    expect(
      run(paths.registrations, '<div id="divSinkokuStatus"></div>'),
    ).toMatchObject({
      status: "error",
      message: "The registration course area is missing.",
    });
    const emptyRegistration = `
      <div id="divSinkokuStatus"></div>
      <table id="timetable"></table>
      <table id="kamokuUnit_1_gvKamoku"><tr><th>授業担当教員</th><th>備考</th></tr></table>`;
    expect(run(paths.registrations, emptyRegistration)).toMatchObject({
      status: "error",
      message: "The empty registration list is not explicit.",
    });
    expect(
      run(
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
      run(
        paths.declared,
        '<table id="kamokuUnit_1_gvKamoku"><tr><th>授業科目名</th></tr><tr><td>構造変更後の行</td></tr></table>',
      ),
    ).toMatchObject({
      status: "error",
      message: "A declared-course row is incomplete.",
    });
  });

  it("rejects one unknown requirement table beside recognized tables", () => {
    const html = asset("fixtures/kyomu-requirements-three-tables.html").replace(
      "</div>",
      '<table class="tblYouken"><tr><th>未知</th><th>列</th></tr><tr><td>値</td><td>1</td></tr></table></div>',
    );
    expect(run(paths.requirements, html)).toMatchObject({
      status: "error",
      message: "A requirement table has no recognized rows.",
    });
  });

  it("requires the GPA total and term tables to both be complete", () => {
    const overall = `
      <table class="tblGPASummary mediaPC">
        <thead><tr><th colspan="4">全ての科目</th></tr><tr><th>GPA</th><th>GPT</th><th>修得済単位</th><th>合格率</th></tr></thead>
        <tbody><tr><td>3.20</td><td>100</td><td>10</td><td>90</td></tr></tbody>
      </table>`;
    expect(
      run(paths.gpa, `<main id="ctl00_ContentPlaceHolder1">${overall}</main>`),
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
      run(
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

  it("reports only an explicit not-offered message as unavailable", () => {
    expect(
      run(
        paths.requirements,
        '<div class="errorMessage">このページは利用できません</div>',
      ),
    ).toMatchObject({
      status: "unavailable",
      kind: "requirements",
      message: "このページは利用できません",
    });
  });

  it("rejects a recognized page while its DOM still reports loading", () => {
    const dom = new JSDOM(asset("fixtures/kyomu-graduate-grades-manual.html"), {
      url: `https://kyomu0.gakumu.titech.ac.jp${paths.grades}`,
      runScripts: "outside-only",
    });
    expect(dom.window.eval(script)).toMatchObject({
      status: "error",
      message: "The school page is still loading.",
    });
  });

  it("rejects an unapproved origin instead of treating its login form as authentication", () => {
    expect(
      run(
        paths.grades,
        '<form action="/login"><input type="password"></form>',
        "example.invalid",
      ),
    ).toMatchObject({ status: "error", kind: "grades" });
  });

  it("recognizes an explicit login form on the approved identity hosts", () => {
    expect(
      run(
        "/login",
        '<form action="/login"><input type="password"></form>',
        "portal.titech.ac.jp",
      ),
    ).toMatchObject({ status: "login", kind: "unknown" });
  });

  it("contains no action that mutates the school page", () => {
    expect(script).not.toMatch(
      /__doPostBack|\.click\s*\(|\.submit\s*\(|\$\.ajax/,
    );
  });
});
