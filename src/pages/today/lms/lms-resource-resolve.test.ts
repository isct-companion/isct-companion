// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it } from "vitest";

type Result =
  | { kind: "file"; url: string }
  | { kind: "external"; url: string }
  | { kind: "folder"; files: { name: string; url: string }[] }
  | { kind: "login" | "unresolved" };

type Resolver = (input: {
  page: Document;
  responseUrl: string;
  cmid: number;
  module: string;
}) => Result;

const source = readFileSync(
  resolve(process.cwd(), "backend/src/lms_resource_resolve.js"),
  "utf8",
);
const resolveResource = Function(`return (${source})`)() as Resolver;

function fixture(body: string, module = "resource", cmid = 42) {
  document.documentElement.innerHTML = `<head></head><body class="cmid-${cmid}">${body}</body>`;
  return resolveResource({
    page: document,
    responseUrl: `https://lms.s.isct.ac.jp/2025/mod/${module}/view.php?id=${cmid}`,
    cmid,
    module,
  });
}

describe("shared LMS resource resolver", () => {
  beforeEach(() => {
    document.documentElement.innerHTML = "<head></head><body></body>";
  });

  it.each([
    [
      "embedded image",
      '<div class="resourcecontent"><img src="/2025/pluginfile.php/1/a.png"></div>',
    ],
    [
      "embedded object",
      '<div class="resourcecontent"><object data="/2025/pluginfile.php/1/a.pdf"></object></div>',
    ],
    [
      "resource workaround",
      '<div class="resourceworkaround"><a href="/2025/pluginfile.php/1/a.zip">Open</a></div>',
    ],
    [
      "embedded iframe",
      '<div class="resourcecontent"><iframe src="/2025/pluginfile.php/1/a.html"></iframe></div>',
    ],
    [
      "embedded video source",
      '<div class="resourcecontent"><video><source src="/2025/pluginfile.php/1/a.mp4"></video></div>',
    ],
    [
      "embedded audio source",
      '<div class="resourcecontent"><audio><source src="/2025/pluginfile.php/1/a.mp3"></audio></div>',
    ],
  ])("resolves an %s", (_name, html) => {
    expect(fixture(html)).toMatchObject({
      kind: "file",
      url: expect.stringContaining("/2025/pluginfile.php/1/"),
    });
  });

  it("returns every allowlisted folder file", () => {
    expect(
      fixture(
        '<div class="foldertree"><a href="/2025/pluginfile.php/1/a.pdf">A</a><a href="/2025/pluginfile.php/1/b.pdf">B</a></div>',
        "folder",
      ),
    ).toEqual({
      kind: "folder",
      files: [
        {
          name: "A",
          url: "https://lms.s.isct.ac.jp/2025/pluginfile.php/1/a.pdf",
        },
        {
          name: "B",
          url: "https://lms.s.isct.ac.jp/2025/pluginfile.php/1/b.pdf",
        },
      ],
    });
  });

  it("returns an external mod/url target", () => {
    expect(
      fixture(
        '<main id="region-main"><a href="https://example.edu/page">Open</a></main>',
        "url",
      ),
    ).toEqual({ kind: "external", url: "https://example.edu/page" });
  });

  it("rejects a cmid mismatch", () => {
    document.documentElement.innerHTML =
      '<head></head><body class="cmid-7"></body>';
    expect(
      resolveResource({
        page: document,
        responseUrl:
          "https://lms.s.isct.ac.jp/2025/mod/resource/view.php?id=42",
        cmid: 42,
        module: "resource",
      }),
    ).toEqual({ kind: "unresolved" });
  });

  it("recognizes a login page before checking cmid", () => {
    document.documentElement.innerHTML =
      '<head></head><body><form action="/login"><input type="password"></form></body>';
    expect(
      resolveResource({
        page: document,
        responseUrl: "https://lms.s.isct.ac.jp/2025/login/index.php",
        cmid: 42,
        module: "resource",
      }),
    ).toEqual({ kind: "login" });
  });
});
