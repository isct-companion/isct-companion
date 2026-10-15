// @vitest-environment jsdom
import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MantineProvider } from "@mantine/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { appTheme } from "../app/theme";
import { seedWorkspace, type Plan, type Workspace } from "../lib/model";
import { resolveCapabilities } from "../platform/capabilities";
import { PlanEditor } from "../pages/schedule/PlanEditor";
import { Settings } from "../pages/settings/SettingsPage";

vi.mock("../platform/core-query", () => ({
  useScheduleQuery: (query: { kind: string; plan?: { start?: string } }) =>
    query.kind === "planCivilStart"
      ? {
          value: {
            date: query.plan?.start?.slice(0, 10) ?? "2026-09-29",
            time: query.plan?.start?.slice(11, 16) ?? "19:00",
          },
          pending: false,
        }
      : { pending: false },
  scheduleQuery: async (query: { kind: string }) => {
    if (query.kind !== "nextPlanOccurrences") return undefined;
    return [
      "2026-09-29",
      "2026-10-02",
      "2026-10-06",
      "2026-10-09",
      "2026-10-13",
    ];
  },
}));

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
globalThis.ResizeObserver = class ResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
};

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
  vi.restoreAllMocks();
});

function clickButton(scope: ParentNode, label: string) {
  const button = [...scope.querySelectorAll("button")].find(
    (candidate) => candidate.textContent?.trim() === label,
  );
  if (!button) throw new Error(`Missing ${label} button`);
  act(() => (button as HTMLButtonElement).click());
}

function renderSettings(data: Workspace, section: "automation") {
  // Automation sits behind Developer options.
  localStorage.setItem("isct-developer-mode", "1");
  const update = vi.fn((operation: (workspace: Workspace) => Workspace) => {
    operation(data);
  });
  const mutate = vi.fn(async () => undefined);
  act(() => {
    root.render(
      <MantineProvider theme={appTheme} defaultColorScheme="light">
        <Settings
          platform="browser-preview"
          data={data}
          mutate={mutate}
          update={update}
          advancedCalendarUpdate={update}
          caps={resolveCapabilities("browser-preview")}
          section={section}
          setSection={vi.fn()}
          notify={vi.fn()}
          showLoginPage={vi.fn()}
        />
      </MantineProvider>,
    );
  });
  return mutate;
}

describe("Task 9 plan editor", () => {
  it("shows the next five Tokyo-calendar occurrences returned by Core", async () => {
    const plan: Plan = {
      id: "local-plan",
      rev: 1,
      updatedAt: "2026-09-29T00:00:00Z",
      updatedBy: "local-device",
      title: "Local study plan",
      start: "2026-09-29T19:00:00+09:00",
      duration: 60,
      rrule: "FREQ=WEEKLY;BYDAY=TU,FR",
      exdates: [],
    };
    act(() => {
      root.render(
        <MantineProvider theme={appTheme} defaultColorScheme="light">
          <PlanEditor plan={plan} opened close={vi.fn()} save={vi.fn()} />
        </MantineProvider>,
      );
    });

    await vi.waitFor(() =>
      expect(document.body.textContent ?? "").toContain("Next 5 occurrences"),
    );
    const editorText = document.body.textContent ?? "";
    expect(editorText).toContain("Next 5 occurrences");
    for (const date of [
      "2026-09-29",
      "2026-10-02",
      "2026-10-06",
      "2026-10-09",
      "2026-10-13",
    ]) {
      expect(editorText).toContain(date);
    }
  });

  it("submits once while a save is pending and allows a later retry", async () => {
    let resolveFirst!: () => void;
    const firstSave = new Promise<void>((resolve) => {
      resolveFirst = resolve;
    });
    const save = vi
      .fn()
      .mockImplementationOnce(() => firstSave)
      .mockResolvedValueOnce(undefined);
    const plan: Plan = {
      id: "guarded-plan",
      rev: 1,
      updatedAt: "2026-09-29T00:00:00Z",
      updatedBy: "local-device",
      title: "Guarded plan",
      start: "2026-09-29T19:00:00+09:00",
      duration: 60,
      exdates: [],
    };
    act(() => {
      root.render(
        <MantineProvider theme={appTheme} defaultColorScheme="light">
          <PlanEditor plan={plan} opened close={vi.fn()} save={save} />
        </MantineProvider>,
      );
    });

    const saveButton = [...document.body.querySelectorAll("button")].find(
      (button) => button.textContent?.trim() === "Save",
    ) as HTMLButtonElement;
    await vi.waitFor(() => expect(saveButton.disabled).toBe(false));
    act(() => {
      saveButton.click();
      saveButton.click();
    });
    expect(save).toHaveBeenCalledTimes(1);
    expect(saveButton.disabled).toBe(true);

    await act(async () => resolveFirst());
    await vi.waitFor(() => expect(saveButton.disabled).toBe(false));
    act(() => saveButton.click());
    await vi.waitFor(() => expect(save).toHaveBeenCalledTimes(2));
  });
});

describe("Task 10 Settings", () => {
  it("toggles a scheduled task and preserves its schedule", () => {
    const data = seedWorkspace();
    data.scheduledTasks = [
      {
        id: "local-daily-export",
        rev: 3,
        updatedAt: "2026-09-29T00:00:00Z",
        updatedBy: data.deviceId,
        name: "Local daily export",
        action: "export",
        schedule: "FREQ=DAILY;BYHOUR=21;BYMINUTE=0",
        enabled: false,
      },
    ];
    const mutate = renderSettings(data, "automation");
    const toggle = host.querySelector<HTMLInputElement>(
      'input[type="checkbox"]',
    );
    expect(toggle).not.toBeNull();
    act(() => toggle!.click());

    expect(mutate).toHaveBeenCalledWith({
      kind: "saveScheduledTask",
      existingId: "local-daily-export",
      draft: expect.objectContaining({
        enabled: true,
        rev: 3,
        schedule: "FREQ=DAILY;BYHOUR=21;BYMINUTE=0",
      }),
    });
  });

  it("offers Run now and displays the recorded run time and result", () => {
    const data = seedWorkspace();
    data.scheduledTasks = [
      {
        id: "local-daily-export",
        rev: 4,
        updatedAt: "2026-09-29T00:00:00Z",
        updatedBy: data.deviceId,
        name: "Local daily export",
        action: "export",
        schedule: "FREQ=DAILY;BYHOUR=21;BYMINUTE=0",
        enabled: true,
        lastRunAt: "2026-09-28T21:00:00+09:00",
        lastResult: "Export completed",
      },
    ];
    renderSettings(data, "automation");

    expect(host.textContent).toMatch(/Last run 28 Sept?, 21:00/);
    expect(host.textContent).toContain("Export completed");
    expect(
      [...host.querySelectorAll("button")].some(
        (button) => button.textContent?.trim() === "Run now",
      ),
    ).toBe(true);
  });
});
