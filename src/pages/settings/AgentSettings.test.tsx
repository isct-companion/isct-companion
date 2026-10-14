// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MantineProvider } from "@mantine/core";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { appTheme } from "../../app/theme";
import { seedWorkspace } from "../../core/model";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), copy: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../../app/native", () => ({ desktopBridge: {} }));
import { AgentSettings } from "./AgentSettings";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  vi.clearAllMocks();
  mocks.copy.mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText: mocks.copy },
  });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});
const mount = () =>
  act(async () => {
    root.render(
      <MantineProvider theme={appTheme} defaultColorScheme="light">
        <AgentSettings
          data={seedWorkspace()}
          update={vi.fn()}
          notify={vi.fn()}
        />
      </MantineProvider>,
    );
  });

it("shows the HTTP address and copies the token only on an explicit click", async () => {
  const url = "http://127.0.0.1:47832/mcp";
  const token = "private-test-token";
  mocks.invoke.mockResolvedValue({ url, token });
  await mount();
  expect(host.textContent).toContain(url);
  expect(host.innerHTML).not.toContain(token);
  expect(host.textContent).not.toContain("--bridge");
  expect(mocks.copy).not.toHaveBeenCalled();
  const copyToken = [...host.querySelectorAll("button")].find(
    (button) => button.textContent === "Copy token",
  );
  expect(copyToken).toBeDefined();
  await act(async () => copyToken!.click());
  expect(mocks.copy).toHaveBeenCalledExactlyOnceWith(token);
});

it("reports server startup failure instead of offering a nonworking address", async () => {
  mocks.invoke.mockRejectedValue("Address already in use");
  await mount();
  expect(host.querySelector('[role="alert"]')?.textContent).toContain(
    "Address already in use",
  );
  expect(host.querySelector("code")).toBeNull();
  expect(host.textContent).not.toContain("Copy token");
});
