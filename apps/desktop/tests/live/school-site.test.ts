import { describe, expect, it } from "vitest";
import { readRepoFile } from "@tests/harness";

/**
 * The only suite that talks to the real school.
 *
 * It exists to catch what frozen fixtures cannot: the school changing a page
 * the app depends on, moving a system, or rolling the LMS site into a new
 * academic year. It asserts the *inputs* the app requires of the live pages,
 * not parsed output.
 *
 * Skipped unless `ISCT_LIVE_SCHOOL_TESTS=1`: CI has no campus network, and
 * `syllabus.s.isct.ac.jp` is reachable only from the school network.
 */

const enabled = process.env.ISCT_LIVE_SCHOOL_TESTS === "1";
const TIMEOUT = 30_000;

describe.skipIf(!enabled)("live Science Tokyo school sites", () => {
  it("serves the syllabus page Core links to, and lets the app frame it", async () => {
    // `catalog::syllabus_url`: the offering-id form, shown in an iframe.
    for (const url of [
      "https://syllabus.s.isct.ac.jp/courses/-/-/-/202602440",
      "https://syllabus.s.isct.ac.jp/courses/-/-/-/202602440?hl=en",
    ]) {
      const response = await fetch(url, {
        redirect: "manual",
        signal: AbortSignal.timeout(TIMEOUT),
        headers: { "user-agent": "IsctCompanion test suite" },
      });
      expect(response.status, url).toBe(200);
      expect(response.headers.get("x-frame-options"), url).toBeNull();
      expect(
        response.headers.get("content-security-policy") ?? "",
        url,
      ).not.toMatch(/frame-ancestors/i);
    }
  });

  it("keeps the LMS site on the academic year the app pins", async () => {
    const response = await fetch("https://lms.s.isct.ac.jp/", {
      redirect: "manual",
      signal: AbortSignal.timeout(TIMEOUT),
    });
    const location = response.headers.get("location") ?? "";
    const year = location.match(/\/(\d{4})\/?$/)?.[1];
    expect(
      year,
      `the LMS root should redirect to its academic-year path, got ${response.status} ${location}`,
    ).toBeDefined();

    const session = readRepoFile("crates/isct-core/src/lms_session.rs");
    expect(session, `LMS_HOME must name the live LMS year (${year})`).toContain(
      `lms.s.isct.ac.jp/${year}/`,
    );

    // Every LMS path literal in the Rust readers must agree with the same
    // year; a stale one silently breaks reading instead of failing loudly.
    const parser = readRepoFile("crates/isct-core/src/school_parser/lms.rs");
    const pinned = new Set(
      [...parser.matchAll(/\/(\d{4})\//g)].map((match) => match[1]),
    );
    expect(
      [...pinned],
      `the LMS readers pin a year other than the live one (${year})`,
    ).toEqual([year]);
  });
});
