import { describe, expect, it } from "vitest";
import { schoolWebBounds } from "./school-web-bounds";

const viewport = (
  platform: string,
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
    navigator: { platform, userAgent: platform },
    ...dimensions,
  }) as Window;

describe("schoolWebBounds", () => {
  it("uses the DOM client-area rectangle unchanged on Windows", () => {
    expect(
      schoolWebBounds(
        { left: 180.4, top: 32.4, width: 987.6, height: 674.6 },
        viewport("Win32"),
      ),
    ).toEqual({ x: 180, y: 32, width: 988, height: 675 });
  });

  it("keeps the WKWebView title-bar compensation on macOS", () => {
    expect(
      schoolWebBounds(
        { left: 180, top: 32, width: 988, height: 675 },
        viewport("MacIntel", {
          outerWidth: 1180,
          outerHeight: 740,
        }),
      ),
    ).toEqual({ x: 180, y: 60, width: 988, height: 675 });
  });
});
