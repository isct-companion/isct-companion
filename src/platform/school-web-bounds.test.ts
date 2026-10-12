import { describe, expect, it } from "vitest";
import { schoolWebBounds } from "./school-web-bounds";

const viewport = (
  dimensions: Partial<{
    innerWidth: number;
    innerHeight: number;
    outerWidth: number;
    outerHeight: number;
  }> = {},
) =>
  ({
    innerWidth: 1180,
    innerHeight: 740,
    outerWidth: 1196,
    outerHeight: 779,
    ...dimensions,
  }) as Window;

describe("schoolWebBounds", () => {
  it("uses the DOM client-area rectangle unchanged on Windows", () => {
    expect(
      schoolWebBounds(
        { left: 180.4, top: 32.4, width: 987.6, height: 674.6 },
        viewport(),
        "windows",
      ),
    ).toEqual({ x: 180, y: 32, width: 988, height: 675 });
  });

  it.each(["ios", "android"] as const)(
    "adds no title-bar inset on %s",
    (os) => {
      expect(
        schoolWebBounds(
          { left: 0, top: 107.6, width: 402, height: 620.4 },
          viewport({
            innerWidth: 402,
            innerHeight: 874,
            outerWidth: 402,
            outerHeight: 874,
          }),
          os,
        ),
      ).toEqual({ x: 0, y: 108, width: 402, height: 620 });
    },
  );

  it("keeps the WKWebView title-bar compensation on macOS", () => {
    expect(
      schoolWebBounds(
        { left: 180, top: 32, width: 988, height: 675 },
        viewport({ outerWidth: 1180, outerHeight: 740 }),
        "macos",
      ),
    ).toEqual({ x: 180, y: 60, width: 988, height: 675 });
  });
});
