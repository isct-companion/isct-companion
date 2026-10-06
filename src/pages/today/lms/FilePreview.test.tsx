// @vitest-environment jsdom
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ open: vi.fn(), stored: vi.fn(), previewKind: vi.fn((_url: string): string | undefined => "text") }));
vi.mock("../../../app/native", () => ({ desktopBridge: {} }));
vi.mock("./files", () => ({ previewKind: mocks.previewKind, openMaterial: mocks.open, openStoredFile: mocks.stored, fileSrc: () => "asset://localhost/a" }));
vi.mock("../../../components/ExternalLink", () => ({ ExternalLink: ({ href, children }: { href: string; children: React.ReactNode }) => <a href={href}>{children}</a> }));
vi.mock("./TextDocument", () => ({ TextDocument: ({ value }: { value: string }) => <pre>{value}</pre> }));
import { FilePreview } from "./FilePreview";
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const context = { term: "2026-3Q", course: "CSC.T273", section: "section-91" };
const LIMIT = 20 * 1024 * 1024;
const file = (name: string, extra = {}) => ({ id: name, name, section: "", path: `/course/${name}`, bytes: 2048, cached: true, ...extra });
const host = document.createElement("div");
let root = createRoot(host);
const render = async (element: React.ReactElement) => { document.body.append(host); root = createRoot(host); await act(async () => root.render(element)); };
const button = (label: string) => [...host.querySelectorAll("button")].find((b) => b.textContent?.includes(label))!;
afterEach(async () => { await act(async () => root.unmount()); host.remove(); mocks.open.mockReset(); mocks.stored.mockReset(); mocks.previewKind.mockImplementation(() => "text"); });

it("previews a file within the limit from the course folder and reveals it", async () => {
  const url = "https://lms.s.isct.ac.jp/2025/pluginfile.php/1/a.txt";
  mocks.open.mockResolvedValue({ state: "ready", file: file("a.txt", { text: "Lecture notes" }) });
  await render(<FilePreview url={url} title="Notes" context={context} limit={LIMIT} />);
  expect(mocks.open).toHaveBeenCalledWith(context, url, { download: true, limit: LIMIT, readText: true });
  expect(host.textContent).toContain("Lecture notes");
  await act(async () => button("Show in").click());
  expect(mocks.stored).toHaveBeenCalledWith("/course/a.txt", true);
});

it("asks before downloading a preview over the limit, then shows it", async () => {
  const url = "https://lms.s.isct.ac.jp/2025/pluginfile.php/1/big.txt";
  mocks.open.mockResolvedValueOnce({ state: "tooLarge", bytes: 45 * 1024 * 1024 }).mockResolvedValueOnce({ state: "ready", file: file("big.txt", { text: "Chapter 1" }) });
  await render(<FilePreview url={url} title="Big" context={context} limit={LIMIT} />);
  expect(host.querySelector(".lmsFileCard")?.textContent).toContain("45 MB");
  await act(async () => button("Preview").click());
  expect(mocks.open).toHaveBeenLastCalledWith(context, url, { download: true, limit: undefined, readText: true });
  expect(host.textContent).toContain("Chapter 1");
});

it("offers a download card for files it cannot preview, then Open and Show in Finder", async () => {
  const url = "https://lms.s.isct.ac.jp/2025/pluginfile.php/1/report.zip";
  mocks.previewKind.mockImplementation(() => undefined);
  mocks.open.mockResolvedValueOnce({ state: "missing" }).mockResolvedValueOnce({ state: "ready", file: file("report.zip") });
  const openInApp = vi.fn();
  await render(<FilePreview url={url} title="Report" context={context} limit={LIMIT} openInApp={openInApp} />);
  expect(mocks.open).toHaveBeenCalledWith(context, url, { download: false, limit: LIMIT, readText: false });
  expect(host.querySelector(".lmsFileCard h3")?.textContent).toBe("Report");
  await act(async () => button("Download").click());
  expect(mocks.open).toHaveBeenLastCalledWith(context, url, { download: true, limit: undefined, readText: false });
  await act(async () => button("Open").click());
  expect(mocks.stored).toHaveBeenCalledWith("/course/report.zip");
  await act(async () => button("Open in app").click());
  expect(openInApp).toHaveBeenCalledOnce();
  expect([...host.querySelectorAll("a")].map((a) => a.textContent)).toContain("Open in browser");
});

it("shows why a download failed and retries on request", async () => {
  const url = "https://lms.s.isct.ac.jp/2025/pluginfile.php/1/a.txt";
  mocks.open.mockRejectedValueOnce("LMS sign-in required").mockResolvedValueOnce({ state: "ready", file: file("a.txt", { text: "Back" }) });
  await render(<FilePreview url={url} title="Notes" context={context} limit={LIMIT} />);
  expect(host.querySelector("[role=alert]")?.textContent).toBe("LMS sign-in required");
  await act(async () => button("Retry").click());
  expect(host.textContent).toContain("Back");
});
