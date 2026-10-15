// @vitest-environment jsdom
import { MantineProvider } from "@mantine/core";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

vi.mock("@mantine/core", async () => {
  const React = await import("react");
  const Wrapper = ({ children }: { children?: React.ReactNode }) =>
    React.createElement(React.Fragment, null, children);
  const Popover = Object.assign(Wrapper, {
    Target: Wrapper,
    Dropdown: Wrapper,
  });
  return {
    MantineProvider: Wrapper,
    Popover,
    Paper: Wrapper,
    Button: ({
      children,
      leftSection,
      ...props
    }: React.ButtonHTMLAttributes<HTMLButtonElement> & {
      leftSection?: React.ReactNode;
    }) => React.createElement("button", props, leftSection, children),
  };
});

import type { GpaSummary } from "../../lib/model";
import { AcademicSummary } from "./AcademicSummary";

let host: HTMLDivElement;
let root: Root;

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

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const summary: GpaSummary = {
  id: "kyomu-gpa:all",
  scope: "all",
  gpa: 3.2,
  gpt: 100,
  earnedCredits: 10,
  fetchedAt: "2026-09-01T00:00:00Z",
  rev: 1,
  updatedAt: "2026-09-01T00:00:00Z",
  updatedBy: "kyomu",
};

async function render(summaries: GpaSummary[]) {
  await act(async () => {
    root.render(
      <MantineProvider>
        <AcademicSummary
          summaries={summaries}
          termCredits={4}
          availability={{
            state: "unavailable",
            checkedAt: "2026-10-15T00:00:00Z",
            message: "このページは利用できません",
          }}
          openOfficial={() => undefined}
        />
      </MantineProvider>,
    );
  });
  await act(async () => {
    host
      .querySelector<HTMLButtonElement>("button")!
      .dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await Promise.resolve();
  });
}

it("shows the capability check instead of calling an unavailable GPA page unfetched", async () => {
  await render([]);
  expect(document.body.textContent).toContain(
    "This GPA page is not provided for this account.",
  );
  expect(document.body.textContent).toContain("Checked:");
  expect(document.body.textContent).not.toContain("Not fetched yet");
});

it("labels retained GPA rows as an earlier fetch", async () => {
  await render([summary]);
  expect(document.body.textContent).toContain(
    "The values below were retained from an earlier successful fetch.",
  );
  expect(document.body.textContent).toContain("Previous fetch:");
  expect(document.body.textContent).toContain("3.20");
});
