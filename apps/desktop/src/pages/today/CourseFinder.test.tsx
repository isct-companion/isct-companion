// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MantineProvider } from "@mantine/core";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { appTheme } from "../../app/theme";
import { seedWorkspace, type Course, type TermId } from "../../lib/model";
import { CourseFinder } from "./CourseFinder";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const course = (id: string, termId: TermId, title: string, code: string) =>
  ({
    kind: "course",
    id,
    termId,
    title,
    code,
    instructor: "",
    color: "#888",
    meetings: [],
    rev: 1,
    updatedAt: "2026-01-01T00:00:00Z",
    updatedBy: "local-device",
  }) as unknown as Course;

function render(open = vi.fn()) {
  const data = seedWorkspace();
  data.courses = [
    course("a", "2025-4Q", "Old Algebra", "MTH.A201"),
    course("b", "2026-3Q", "Signals", "EEE.S301"),
    course("c", "2026-1Q", "Linear Algebra", "MTH.A101"),
  ];
  act(() =>
    root.render(
      <MantineProvider theme={appTheme}>
        <CourseFinder data={data} term="2026-3Q" open={open} />
      </MantineProvider>,
    ),
  );
  const input = host.querySelector("input")!;
  act(() => input.focus());
  return { input, open };
}

const groups = () =>
  [...document.querySelectorAll(".courseFinderList section")].map((section) => [
    section.querySelector("small")?.textContent,
    ...[...section.querySelectorAll("button span")].map((s) => s.textContent),
  ]);

it("lists this term first, then older quarters newest first", () => {
  render();
  expect(groups()).toEqual([
    ["This term · 2026 · 3Q", "Signals"],
    ["2026 · 1Q", "Linear Algebra"],
    ["2025 · 4Q", "Old Algebra"],
  ]);
});

it("filters by title or code and opens the chosen course", () => {
  const { input, open } = render();
  act(() => {
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!;
    setter.call(input, "algebra");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(groups()).toEqual([
    ["2026 · 1Q", "Linear Algebra"],
    ["2025 · 4Q", "Old Algebra"],
  ]);
  act(() =>
    input.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    ),
  );
  expect(open).toHaveBeenCalledWith("c");
});
