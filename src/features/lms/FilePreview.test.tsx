// @vitest-environment jsdom
import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ open: vi.fn(), stored: vi.fn() }));
vi.mock("../../app/native", () => ({ desktopBridge: {} }));
vi.mock("./files", async () => {
  class LmsFileError extends Error {
    constructor(public reason: string, public bytes?: number) { super(reason); }
  }
  return {
    LmsFileError,
    localFileUrl: (path: string) => `asset://localhost${path}`,
    openMaterial: mocks.open,
    openStoredFile: mocks.stored,
    previewKind: (url: string) => url.endsWith(".txt") ? "text" : undefined,
  };
});
vi.mock("../../components/ExternalLink", () => ({ ExternalLink: ({ href, children }: { href: string; children: React.ReactNode }) => <a href={href}>{children}</a> }));
vi.mock("./TextDocument", () => ({ TextDocument: ({ value }: { value: string }) => <pre>{value}</pre> }));
import { FilePreview } from "./FilePreview";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
const context = { term: "2026-3Q", course: "CSC.T273", section: "section-91" };
const LIMIT = 20 * 1024 * 1024;
const host = document.createElement("div");
let root: Root;
const render = async (element: React.ReactElement) => {
  document.body.append(host); root = createRoot(host);
  await act(async () => root.render(element));
  await vi.waitFor(() => expect(host.querySelector(".lmsLoading")).toBeNull());
};
const button = (label: string) => [...host.querySelectorAll("button")].find((item) => item.textContent?.includes(label))!;

afterEach(async () => {
  await act(async () => root?.unmount()); host.remove(); host.replaceChildren();
  mocks.open.mockReset(); mocks.stored.mockReset();
});

it("prepares through Rust and uses the local data URL to preview", async () => {
  mocks.open.mockResolvedValue({ state: "ready", file: { name: "a.txt", path: "/course/a.txt", text: "Lecture notes" } });
  await render(<FilePreview url="https://lms.s.isct.ac.jp/2025/pluginfile.php/1/a.txt" title="Notes" context={context} limit={LIMIT} />);
  expect(host.textContent).toContain("Lecture notes");
  expect(mocks.open).toHaveBeenNthCalledWith(1, context, expect.stringContaining("a.txt"), {
    download: true, readText: true, limit: LIMIT,
  });
  await act(async () => button("Show in").click());
  expect(mocks.open).toHaveBeenLastCalledWith(context, expect.stringContaining("a.txt"), { download: true });
  expect(mocks.stored).toHaveBeenCalledWith("/course/a.txt", true);
});

it("shows size and retries explicitly without the automatic limit", async () => {
  mocks.open.mockResolvedValueOnce({ state: "tooLarge", bytes: 45 * 1024 * 1024 })
    .mockResolvedValueOnce({ state: "ready", file: { name: "big.txt", path: "/course/big.txt", text: "Chapter 1" } });
  await render(<FilePreview url="https://lms.s.isct.ac.jp/2025/pluginfile.php/1/big.txt" title="Big" context={context} limit={LIMIT} />);
  expect(host.textContent).toContain("45 MB");
  await act(async () => button("Preview").click());
  await vi.waitFor(() => expect(host.textContent).toContain("Chapter 1"));
  expect(mocks.open.mock.calls.at(-1)?.[2]).toMatchObject({ download: true, limit: undefined });
});

it.each([
  ["login", "Sign in to LMS"],
  ["unresolved", "Open in LMS"],
  ["network", "Open in LMS"],
] as const)("maps %s to its viewer fallback", async (reason, label) => {
  mocks.open.mockResolvedValueOnce({ state: "failed", reason });
  const openInApp = vi.fn();
  await render(<FilePreview url="https://lms.s.isct.ac.jp/2025/mod/resource/view.php?id=42" title="File" context={context} limit={LIMIT} openInApp={openInApp} />);
  await act(async () => button(label).click());
  expect(openInApp).toHaveBeenCalledOnce();
});
