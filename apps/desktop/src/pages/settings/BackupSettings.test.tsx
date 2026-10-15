// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MantineProvider } from "@mantine/core";
import { ModalsProvider } from "@mantine/modals";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { appTheme } from "../../app/theme";
import { seedWorkspace } from "../../lib/model";
import type { CloudSyncState } from "../../platform/desktop-bridge";

const mocks = vi.hoisted(() => {
  const state = (enabled: boolean): CloudSyncState => ({
    syncing: false,
    providers: [
      {
        provider: "icloud",
        enabled: true,
        lastSyncedAt: "2026-10-12T03:00:00Z",
      },
      { provider: "google-drive", enabled },
    ],
    versions: [
      {
        id: "v1",
        provider: "icloud",
        savedAt: "2026-10-12T02:00:00Z",
        device: "MacBook",
      },
    ],
  });
  return {
    state,
    listeners: [] as Array<(state: CloudSyncState) => void>,
    bridge: {
      cloudSyncState: vi.fn(async () => state(false)),
      setCloudSyncProvider: vi.fn(async (_: string, enabled: boolean) =>
        state(enabled),
      ),
      syncCloudNow: vi.fn(async () => state(false)),
      setCloudSyncAutomatic: vi.fn(async (automatic: boolean) => ({
        ...state(false),
        automatic,
      })),
      restoreCloudVersion: vi.fn(async () => state(false)),
      onCloudSyncState: vi.fn((callback: (state: CloudSyncState) => void) => {
        mocks.listeners.push(callback);
        return vi.fn();
      }),
      cloudCredentialSyncStatus: vi.fn(async () => false),
      cloudCredentialBackupStatus: vi.fn(async () => false),
    },
  };
});
vi.mock("../../app/native", () => ({
  desktopBridge: mocks.bridge,
  saveExportFile: vi.fn(),
}));
vi.mock("../../store/workspace", () => ({
  workspaceActions: { refreshSchool: vi.fn(async () => {}) },
}));

import { BackupSettings } from "./BackupSettings";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement;
let root: Root;
const notify = vi.fn();
const caps = {
  "cloud-sync": { support: "supported", readiness: "ready", reason: "" },
} as never;

const flush = () =>
  act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
const button = (name: string, scope: ParentNode = host) =>
  [...scope.querySelectorAll("button")].find(
    (item) => item.textContent?.trim() === name,
  ) as HTMLButtonElement;

beforeEach(async () => {
  vi.clearAllMocks();
  mocks.listeners.length = 0;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await act(async () => {
    root.render(
      <MantineProvider theme={appTheme} defaultColorScheme="light">
        <ModalsProvider>
          <BackupSettings
            data={seedWorkspace()}
            update={vi.fn()}
            caps={caps}
            notify={notify}
          />
        </ModalsProvider>
      </MantineProvider>,
    );
  });
  await flush();
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe("BackupSettings", () => {
  it("lists providers with separate upload, pull and Auto controls", () => {
    const text = host.textContent ?? "";
    expect(text).toContain("iCloud");
    expect(text).toContain("Google Drive");
    expect(text).toContain("Upload");
    expect(text).toContain("Pull");
    expect(
      host.querySelector<HTMLInputElement>(
        '[data-setting="sync-automatic"] input',
      )?.checked,
    ).toBe(false);
    expect(text).not.toContain("Back up now");
    expect(
      host.querySelector('[data-setting="credential-keychain"]'),
    ).not.toBeNull();
    // Drive is off, so its encrypted credential flow stays hidden.
    expect(host.querySelector('[data-setting="credential-sync"]')).toBeNull();
  });

  it("turns Google Drive on and then offers the encrypted credential flow", async () => {
    const drive = host.querySelector<HTMLInputElement>(
      '[data-setting="cloud"] input[type="checkbox"]',
    )!;
    await act(async () => drive.click());
    await flush();
    expect(mocks.bridge.setCloudSyncProvider).toHaveBeenCalledWith(
      "google-drive",
      true,
    );
    expect(
      host.querySelector('[data-setting="credential-sync"]'),
    ).not.toBeNull();
    // Off: no password fields until Set up is chosen.
    expect(host.querySelectorAll('input[type="password"]')).toHaveLength(0);
    await act(async () => button("Set up").click());
    expect(host.querySelectorAll('input[type="password"]')).toHaveLength(2);
    expect(host.textContent).toContain(
      "Encrypted with this password before upload.",
    );
    await act(async () => button("Cancel").click());
    expect(host.querySelectorAll('input[type="password"]')).toHaveLength(0);
  });

  it("syncs on request and restores a kept version after confirming", async () => {
    await act(async () => button("Upload").click());
    expect(mocks.bridge.syncCloudNow).toHaveBeenCalledWith("upload");
    await act(async () => button("Pull").click());
    expect(mocks.bridge.syncCloudNow).toHaveBeenLastCalledWith("pull");
    await act(async () => button("Restore").click());
    expect(mocks.bridge.restoreCloudVersion).not.toHaveBeenCalled();
    const dialog = await vi.waitFor(() => {
      const open = document.querySelector('[role="dialog"]');
      if (!open) throw new Error("No confirm dialog");
      return open;
    });
    expect(dialog.textContent).toContain("Restore the version from");
    await act(async () => button("Restore", dialog).click());
    await flush();
    expect(mocks.bridge.restoreCloudVersion).toHaveBeenCalledWith("v1");
    expect(notify).toHaveBeenCalledWith("Restored the earlier version.");
  });

  it("follows native sync state events", async () => {
    await act(async () =>
      mocks.listeners[0]({ ...mocks.state(false), syncing: true }),
    );
    expect(host.textContent).toContain("Syncing…");
  });
  it("switches Auto explicitly and shows unfinished provider deliveries", async () => {
    const toggle = host.querySelector<HTMLInputElement>(
      '[data-setting="sync-automatic"] input',
    )!;
    await act(async () => toggle.click());
    expect(mocks.bridge.setCloudSyncAutomatic).toHaveBeenCalledWith(true);
    await act(async () =>
      mocks.listeners[0]({
        ...mocks.state(true),
        localVersion: "v1-new",
        tasks: [
          {
            id: "task-1",
            direction: "upload",
            version: "v1-new",
            createdAt: "2026-10-12T03:00:00Z",
            deliveries: [
              {
                provider: "icloud",
                status: "pending",
                attempts: 2,
                retryAt: 1,
              },
              {
                provider: "google-drive",
                status: "completed",
                attempts: 1,
                retryAt: 0,
              },
            ],
          },
        ],
      }),
    );
    expect(host.textContent).toContain("Waiting to sync");
    expect(host.textContent).toContain("task-1");
    expect(host.textContent).not.toContain("Up to date");
    expect(button("Restore").disabled).toBe(true);
  });
});
