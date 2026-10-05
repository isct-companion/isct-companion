// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { seedWorkspace } from "../core/model";
import { addSyllabusDraft } from "../core/syllabus";
import { catalogCourse } from "../platform/desktop-bridge";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
globalThis.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as never;
window.matchMedia = ((query: string) => ({
  matches: false,
  media: query,
  addEventListener() {},
  removeEventListener() {},
  addListener() {},
  removeListener() {},
  onchange: null,
  dispatchEvent: () => false,
})) as never;

const offering = (id: string, code: string, title: string) => ({
  id,
  year: 2026,
  quarters: ["3Q"],
  code,
  title: { ja: title, en: title },
  instructors: [{ ja: "教員", en: "Teacher" }],
  syllabusUrl: `https://syllabus.s.isct.ac.jp/courses/2026/4/0-904-342300-0-0/${id}`,
  credits: { lecture: 2, exercise: 0, lab: 0 },
  instructionLanguage: "ja",
  format: { kinds: ["lecture"] },
  schedule: [{ kind: "weekly", day: 2, periods: [7, 8] }],
});
const placed = offering("202602421", "CSC.T252", "Placed");
const clashing = offering("202600009", "CSC.T999", "Clashing");

let saved = (() => {
  const workspace = seedWorkspace();
  workspace.selectedTerm = "2026-3Q";
  workspace.terms = ["2026-3Q"];
  addSyllabusDraft(workspace, {
    ...catalogCourse(placed, "en"),
    quarter: "3Q",
  });
  return workspace;
})();
(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
  transformCallback: () => 1,
  unregisterCallback: () => {},
  invoke: async (command: string, args: Record<string, unknown>) => {
    switch (command) {
      case "load_workspace":
        return structuredClone(saved);
      case "save_workspace":
        saved = structuredClone((args.workspace ?? saved) as typeof saved);
        return 1;
      case "catalog_offerings":
        return [placed, clashing];
      case "catalog_query":
        return {
          items: [{ id: placed.id }, { id: clashing.id }],
          total: 2,
          returned: 2,
          truncated: false,
        };
      case "catalog_sync_offerings":
        return [];
      case "school_records":
        return { lastRuns: {}, lmsCourses: [], registrations: [], registrationStates: [], grades: [], gpaByTerm: [], requirements: [] };
      case "runtime_capabilities":
        return {};
      case "school_credential_status":
        return {
          science: {
            saved: false,
            username: "",
            hasOtp: false,
            hasMatrix: false,
          },
          tokyoTech: {
            saved: false,
            username: "",
            hasOtp: false,
            hasMatrix: false,
          },
        };
      default:
        return null;
    }
  },
};

const wait = (ms: number) =>
  act(async () => new Promise((resolve) => setTimeout(resolve, ms)));
const titles = (host: HTMLElement) => [
  ...new Set(
    [...host.querySelectorAll(".ttBlock b")].map((b) => b.textContent),
  ),
];

it("adds a clashing course as a replacement, and Undo brings the old one back", async () => {
  const errors = vi.spyOn(console, "error").mockImplementation(() => {});
  const { default: App } = await import("../app/App");
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  await act(async () => root.render(<App />));
  const nav = await vi.waitFor(() => {
    const button = [...host.querySelectorAll("button")].find((b) =>
      /^Schedule/.test(b.getAttribute("aria-label") ?? b.textContent ?? ""),
    );
    if (!button) throw new Error("app not ready");
    return button;
  });
  await act(async () => nav!.click());
  const edit = host.querySelector<HTMLInputElement>('input[role="switch"]')!;
  await act(async () => edit.click());
  const input = host.querySelector<HTMLInputElement>("#course-finder-input")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(input, "CSC ");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(titles(host)).toEqual(["Placed"]);
  const replace = await vi.waitFor(
    () => {
      const button = [
        ...host.querySelectorAll<HTMLButtonElement>(".finderAdd"),
      ].find((candidate) => candidate.textContent === "Replace");
      if (!button) throw new Error("no Replace yet");
      return button;
    },
    { timeout: 5000 },
  );
  await act(async () => replace.click());
  await vi.waitFor(() => expect(titles(host)).toEqual(["Clashing"]));

  const undo = [...host.querySelectorAll<HTMLButtonElement>("button")].find(
    (b) => b.textContent === "Undo",
  );
  await act(async () => undo!.click());
  await vi.waitFor(() => expect(titles(host)).toEqual(["Placed"]));
  await wait(50);
  expect(host.querySelector(".pageError")).toBeNull();
  // React's "not wrapped in act" notices come from async polling, not from the app.
  const real = errors.mock.calls.filter(
    (call) => !String(call[0]).includes("not wrapped in act"),
  );
  expect(real).toEqual([]);
  errors.mockRestore();
  act(() => root.unmount());
}, 15_000);
