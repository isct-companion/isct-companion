import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { runInNewContext } from "node:vm";
import { expect, it } from "vitest";

it("installs Android support once per document and into later popup frames", () => {
  const frame = () => {
    const scripts: string[] = [];
    const listeners: ((event: unknown) => void)[] = [];
    const children: { contentWindow: unknown }[] = [];
    const win = {
      eval: (source: string) => scripts.push(source),
      document: {
        addEventListener: (_: string, callback: (event: unknown) => void) =>
          listeners.push(callback),
        querySelectorAll: () => children,
      },
    };
    return { win, scripts, listeners, children };
  };
  const main = frame();
  const existing = frame();
  main.children.push({ contentWindow: existing.win });
  const source = readFileSync(
    resolve("backend/scripts/android-school-support.js"),
    "utf8",
  ).replace(
    "__ISCT_SUPPORT_SCRIPTS__",
    JSON.stringify(["popup support", "link support"]),
  );
  runInNewContext(source, { window: main.win });
  runInNewContext(source, { window: main.win });
  expect(main.scripts).toEqual(["popup support", "link support"]);
  expect(existing.scripts).toEqual(main.scripts);
  const popup = frame();
  main.listeners[0]({
    target: { tagName: "IFRAME", contentWindow: popup.win },
  });
  main.listeners[0]({
    target: { tagName: "IFRAME", contentWindow: popup.win },
  });
  expect(popup.scripts).toEqual(main.scripts);
  const nested = frame();
  popup.listeners[0]({
    target: { tagName: "IFRAME", contentWindow: nested.win },
  });
  expect(nested.scripts).toEqual(main.scripts);
  const foreign = {
    get document(): never {
      throw new Error("Cross origin");
    },
  };
  expect(() =>
    main.listeners[0]({
      target: { tagName: "IFRAME", contentWindow: foreign },
    }),
  ).not.toThrow();
});
