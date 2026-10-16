import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import { readRepoFile } from "@tests/harness";

/**
 * The only suite that talks to the real school.
 *
 * It exists to catch what frozen fixtures cannot: the school changing its
 * markup, moving a system, or rolling the LMS site into a new academic year.
 * It asserts the *inputs* the parsers require — the elements, labels and shape
 * of the live pages — not the parsed output, because parsing frozen bytes is
 * `cargo test`'s job (`isct-core` embeds the same files). Between the two, a
 * live change fails here and a parser regression fails there.
 *
 * Skipped unless `ISCT_LIVE_SCHOOL_TESTS=1`: CI has no campus network, and
 * `syllabus.s.isct.ac.jp` is reachable only from the school network.
 */

const enabled = process.env.ISCT_LIVE_SCHOOL_TESTS === "1";
const ORIGIN = "https://syllabus.s.isct.ac.jp";
const scope = "4/0-904-342300-0-0";
const TIMEOUT = 30_000;

type Provenance = {
  observedDate: string;
  scope: string;
  fixtures: { file: string; url: string; bytes: number; sha256: string }[];
};

function provenance(): Provenance {
  return JSON.parse(
    readRepoFile(
      "crates/isct-core/tests/fixtures/syllabus/content-provenance.json",
    ),
  ) as Provenance;
}

/** Follows redirects, because `?hl=ja` redirects to the bare path. */
async function get(url: string): Promise<Buffer> {
  const response = await fetch(url, {
    redirect: "follow",
    signal: AbortSignal.timeout(TIMEOUT),
    headers: { "user-agent": "IsctCompanion test suite" },
  });
  if (!response.ok) throw new Error(`${url} answered ${response.status}`);
  return Buffer.from(await response.arrayBuffer());
}

/**
 * The documented excerpt rule, applied at byte boundaries: from the first
 * `h1.c-h1` to the first `aside`, LF-normalised. Both cut points sit on ASCII
 * markers, so slicing bytes and slicing the decoded text give the same result.
 * `content-provenance.json` pins the digest of the outcome, so a drift report
 * here means the fixture is stale, not that the rule changed.
 */
function excerpt(page: Buffer): Buffer {
  const start = page.indexOf("<h1");
  const end = page.indexOf("<aside");
  if (start < 0 || end <= start) {
    throw new Error("the page no longer contains an h1 followed by an aside");
  }
  const slice = page
    .subarray(start, end)
    .toString("utf8")
    .replaceAll("\r\n", "\n");
  return Buffer.from(slice, "utf8");
}

const digest = (bytes: Buffer) =>
  createHash("sha256").update(bytes).digest("hex");

function text(page: Buffer): string {
  return page.toString("utf8");
}

describe.skipIf(!enabled)("live Science Tokyo syllabus site", () => {
  it("still serves the search result table the parser reads", async () => {
    const page = text(
      await get(`${ORIGIN}/courses/2026/${scope}/search?q=CSC.T354&hl=en`),
    );
    // The count marker and its `N items / out of M` label.
    const count = page.match(
      /c-item-amount__number"[^>]*>\s*(\d+)\s+items?\s*\/\s*out of\s*(\d+)/,
    );
    expect(count, "the result-count marker is still present").not.toBeNull();
    const [, returned, total] = count!;
    expect(Number(returned)).toBeGreaterThan(0);
    expect(Number(total)).toBeGreaterThanOrEqual(Number(returned));
    // The result table and the exact column order `parse_search` indexes.
    expect(page).toContain('class="c-table is-table-style1"');
    const table = page.slice(page.indexOf("is-table-style1"));
    const header = (
      table
        .slice(0, table.indexOf("</table>"))
        .match(/<th\b[^>]*>[\s\S]*?<\/th>/g) ?? []
    ).map((cell) =>
      cell
        .replace(/<[^>]*>/g, " ")
        .replace(/\s+/g, " ")
        .trim(),
    );
    expect(header, "the six result columns, in order").toEqual([
      "Course Code",
      "Course Title",
      "Instructor(s)",
      "Academic unit or major",
      "Course offered (Quarter)",
      "Syllabus updated",
    ]);
    // Rows link to a detail page under the requested year.
    expect(page).toMatch(/href="[^"]*\/courses\/2026\//);
  });

  it("still serves the detail metadata the parser requires", async () => {
    const pin = provenance();
    const languages = ["-en-", "-ja-"];
    for (const marker of languages) {
      const entry = pin.fixtures.find((fixture) =>
        fixture.file.includes(marker),
      );
      if (!entry) continue;
      const page = text(await get(entry.url));
      expect(page, `${entry.file}: one syllabus heading`).toMatch(
        /<h1 class="c-h1">/,
      );
      const items = page.match(/class="c-dl-2col__item"/g) ?? [];
      expect(
        items.length,
        `${entry.file}: the metadata definition list still carries at least the twelve required fields`,
      ).toBeGreaterThanOrEqual(12);
      expect(page, `${entry.file}: credit spans`).toMatch(
        /class="is-dl-number-of-credits"/,
      );
      expect(page, `${entry.file}: a labelled term for every value`).toContain(
        "<dt>",
      );
    }
  });

  it("reports every fixture whose live bytes have moved", async () => {
    const pin = provenance();
    const stale: string[] = [];
    for (const fixture of pin.fixtures) {
      const live = excerpt(await get(fixture.url));
      if (
        digest(live) !== fixture.sha256 ||
        live.byteLength !== fixture.bytes
      ) {
        stale.push(
          `${fixture.file}: pinned ${fixture.bytes}B/${fixture.sha256.slice(0, 12)}, ` +
            `live ${live.byteLength}B/${digest(live).slice(0, 12)}`,
        );
      }
    }
    expect(
      stale,
      "The school changed a pinned page. Re-fetch it, replace the fixture and " +
        "update content-provenance.json — see " +
        "crates/isct-core/tests/fixtures/syllabus/README.md.",
    ).toEqual([]);
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

    // Every path literal in the injected extractor must agree with the same
    // year; a stale one silently disables extraction instead of failing loudly.
    const ingest = readRepoFile("crates/isct-core/src/lms_ingest.rs");
    const pinned = new Set(
      [...ingest.matchAll(/\/(\d{4})\//g)].map((match) => match[1]),
    );
    expect(
      [...pinned],
      `the injected LMS extractor pins a year other than the live one (${year})`,
    ).toEqual([year]);
  });
});
