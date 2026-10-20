import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { describe, expect, it } from "vitest";

import { readRepoFile, repositoryRoot } from "@tests/harness";

/**
 * Provenance of the committed public fixtures and the bundled subscription
 * data. School pages are read in Rust now (`net::moodle`, `net::kyomu`); their
 * parsers are tested in `crates/isct-core/src/tests/`.
 */

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
