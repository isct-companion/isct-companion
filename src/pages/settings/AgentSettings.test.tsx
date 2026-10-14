// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MantineProvider } from "@mantine/core";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { appTheme } from "../../app/theme";
import { seedWorkspace } from "../../core/model";

const mocks = vi.hoisted(() => ({ agentConnection: vi.fn(), copy: vi.fn() }));
vi.mock("../../app/native", () => ({
  desktopBridge: { agentConnection: mocks.agentConnection },
}));
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
const mount = (enabled = true) =>
  act(async () => {
    const data = seedWorkspace();
    data.settings.agentAccess = { enabled, lastWrites: [] };
    root.render(
      <MantineProvider theme={appTheme} defaultColorScheme="light">
        <AgentSettings data={data} update={vi.fn()} notify={vi.fn()} />
      </MantineProvider>,
    );
  });

it("shows the HTTP address and copies the token only on an explicit click", async () => {
  const url = "http://127.0.0.1:47832/mcp";
  const token = "private-test-token";
  mocks.agentConnection.mockResolvedValue({ state: "listening", url, token });
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
  mocks.agentConnection.mockResolvedValue({
    state: "failed",
    error: "The MCP port 47832 is already in use.",
  });
  await mount();
  expect(host.querySelector('[role="alert"]')?.textContent).toContain(
    "already in use",
  );
  expect(host.querySelector("code")).toBeNull();
  expect(host.textContent).not.toContain("Copy token");
});

it("offers no address or token while agent access is off", async () => {
  mocks.agentConnection.mockResolvedValue({ state: "off" });
  await mount(false);
  expect(host.textContent).toContain("Turn on Allow local agents");
  expect(host.querySelector("code")).toBeNull();
  expect(host.querySelector('[role="alert"]')).toBeNull();
});
