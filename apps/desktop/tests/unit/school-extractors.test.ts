import { describe, expect, it } from "vitest";

import { readRepoFile } from "@tests/harness";

/**
 * Provenance of the bundled subscription data. School pages are read in Rust
 * (`net::moodle`, `net::kyomu`); their parsers are tested in
 * `crates/app-core/src/tests/`.
 */

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
        `crates/app-core/tests/fixtures/subscriptions/data/${year}/departments.json`,
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

  it("cites an official source for the academic-calendar day list", () => {
    const calendar = JSON.parse(
      readRepoFile(
        "crates/app-core/tests/fixtures/subscriptions/data/2026/academic-calendar.json",
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
