// @vitest-environment jsdom
import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MantineProvider } from "@mantine/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { appTheme } from "../app/theme";
import { seedWorkspace, type Plan, type Workspace } from "../core/model";
import { resolveCapabilities } from "../platform/capabilities";
import { PlanEditor } from "../pages/schedule/PlanEditor";
import { Settings } from "../pages/settings/SettingsPage";

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

function renderSettings(data: Workspace, section: "hidden" | "scheduled") {
  const update = vi.fn((operation: (workspace: Workspace) => Workspace) => {
    operation(data);
  });
  act(() => {
    root.render(
      <MantineProvider theme={appTheme} defaultColorScheme="light">
        <Settings
          platform="browser-preview"
          data={data}
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
  return update;
}

describe("Task 9 plan editor", () => {
  it("shows the next five Tokyo-calendar occurrences for an advanced rule", () => {
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
          <PlanEditor
            plan={plan}
            opened
            close={vi.fn()}
            save={vi.fn()}
          />
        </MantineProvider>,
      );
    });

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
});

describe("Task 10 Settings", () => {
  it("restores one hidden local item and clear-all restores the rest with revisions", () => {
    const data = seedWorkspace();
    data.assignments = ["first", "second"].map((id, index) => ({
      id: `local-${id}`,
      rev: 1,
      updatedAt: "2026-09-29T00:00:00Z",
      updatedBy: data.deviceId,
      kind: "assignment" as const,
      courseId: "local-course",
      title: `Local hidden item ${index + 1}`,
      details: "",
      dueAt: `2026-10-${10 + index}T09:00:00+09:00`,
      hidden: true,
      source: { provider: "manual" as const },
    }));
    const update = renderSettings(data, "hidden");

    clickButton(host, "Restore");
    expect(data.assignments[0]).toMatchObject({ hidden: false, rev: 2 });
    expect(data.assignments[0].updatedAt).not.toBe("2026-09-29T00:00:00Z");
    expect(data.assignments[1]).toMatchObject({ hidden: true, rev: 1 });

    clickButton(host, "Clear all");
    expect(data.assignments.map(({ hidden, rev }) => ({ hidden, rev }))).toEqual([
      { hidden: false, rev: 2 },
      { hidden: false, rev: 2 },
    ]);
    expect(update).toHaveBeenCalledTimes(2);
  });

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
    renderSettings(data, "scheduled");
    const toggle = host.querySelector<HTMLInputElement>('input[type="checkbox"]');
    expect(toggle).not.toBeNull();
    act(() => toggle!.click());

    expect(data.scheduledTasks[0]).toMatchObject({
      enabled: true,
      rev: 4,
      schedule: "FREQ=DAILY;BYHOUR=21;BYMINUTE=0",
    });
    expect(data.scheduledTasks[0].updatedAt).not.toBe("2026-09-29T00:00:00Z");
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
    renderSettings(data, "scheduled");

    expect(host.textContent).toContain("Last run");
    expect(host.textContent).toContain("2026-09-28");
    expect(host.textContent).toContain("Export completed");
    expect(
      [...host.querySelectorAll("button")].some(
        (button) => button.textContent?.trim() === "Run now",
      ),
    ).toBe(true);
  });
});
