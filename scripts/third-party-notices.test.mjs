import path from "node:path";
import { describe, expect, it } from "vitest";
import {
  buildNotices,
  packageRoot,
  renderNotices,
} from "./third-party-notices.mjs";

describe("third-party notices", () => {
  it("maps bundled modules to their package root, scoped or not", () => {
    const nm = `${path.sep}app${path.sep}node_modules${path.sep}`;
    expect(packageRoot(`${nm}react${path.sep}index.js`)).toBe(`${nm}react`);
    expect(
      packageRoot(`${nm}@mantine${path.sep}core${path.sep}esm${path.sep}x.mjs?v=1`),
    ).toBe(`${nm}@mantine${path.sep}core`);
    expect(packageRoot(`${path.sep}app${path.sep}src${path.sep}main.tsx`)).toBe(
      undefined,
    );
  });

  it("ships each license text once, with the bundled packages and models", () => {
    const notices = buildNotices({
      moduleIds: [
        path.resolve("node_modules/react/index.js"),
        path.resolve("node_modules/react-dom/index.js"),
      ],
    });
    const names = notices.components.map((item) => item.name);
    expect(names).toEqual(
      expect.arrayContaining(["react", "react-dom", "ONNX Runtime"]),
    );
    for (const item of notices.components) {
      expect(item.texts.length).toBeGreaterThan(0);
      for (const index of item.texts)
        expect(notices.texts[index]).toEqual(expect.any(String));
    }
    expect(new Set(notices.texts).size).toBe(notices.texts.length);
    const text = renderNotices(notices);
    expect(text).toContain("react-dom");
    expect(text).toContain("Copyright (c) Meta Platforms");
  });
});
