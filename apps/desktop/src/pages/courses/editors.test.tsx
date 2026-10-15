// @vitest-environment jsdom
import { MantineProvider } from "@mantine/core";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import {
  DEFAULT_TEACHING_PERIODS,
  type Course,
  type Meeting,
} from "../../generated/persistence";
import { CourseEditor, type CourseDraft } from "./editors";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
window.matchMedia = (() => ({
  matches: false,
  addEventListener() {},
  removeEventListener() {},
  addListener() {},
  removeListener() {},
})) as never;

let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});

const meeting = (
  periods: number[],
  startTime: string,
  endTime: string,
): Meeting => ({
  days: [1],
  periods,
  startTime,
  endTime,
  room: "W9-201",
});

const course = (value: Meeting): Course => ({
  kind: "course",
  termId: "2026-1Q",
  code: "CSC.T354",
  title: "Parallel programming",
  instructor: "Instructor",
  credits: 2,
  creditSplit: { lecture: 2, exercise: 0, lab: 0 },
  instructionLanguage: "en",
  color: "#355ba7",
  meetings: [value],
  source: { provider: "manual" },
  id: "course-1",
  rev: 1,
  updatedAt: "2026-10-15T00:00:00Z",
  updatedBy: "test",
});

async function editAndSave(value: Meeting, selectValue?: string) {
  const onSave = vi.fn<(draft: CourseDraft) => void>();
  await act(async () => {
    root.render(
      <MantineProvider>
        <CourseEditor
          course={course(value)}
          term="2026-1Q"
          onTimetable
          onClose={() => undefined}
          onSave={onSave}
          teachingPeriods={DEFAULT_TEACHING_PERIODS}
        />
      </MantineProvider>,
    );
  });

  const title = document.querySelector<HTMLInputElement>(
    'input[placeholder="並列プログラミング"]',
  )!;
  const titleSetter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    "value",
  )!.set!;
  await act(async () => {
    titleSetter.call(title, "Renamed course");
    title.dispatchEvent(new Event("input", { bubbles: true }));
  });

  const period = document.querySelector<HTMLSelectElement>(
    'select[aria-label="Period"]',
  )!;
  if (selectValue) {
    const selectSetter = Object.getOwnPropertyDescriptor(
      HTMLSelectElement.prototype,
      "value",
    )!.set!;
    await act(async () => {
      selectSetter.call(period, selectValue);
      period.dispatchEvent(new Event("change", { bubbles: true }));
    });
  }

  const save = [...document.querySelectorAll<HTMLButtonElement>("button")].find(
    (button) => button.textContent === "Save",
  )!;
  await act(async () => save.click());
  return { draft: onSave.mock.calls[0][0], period };
}

it("keeps a cross-block period range when only course details change", async () => {
  const { draft, period } = await editAndSave(
    meeting([1, 2, 3, 4], "08:50", "12:25"),
  );
  expect(period.value).toBe("periods:1,2,3,4");
  expect(period.selectedOptions[0].textContent).toContain("1-4 · 08:50–12:25");
  expect(draft.title).toBe("Renamed course");
  expect(draft.meetings[0].periods).toEqual([1, 2, 3, 4]);
});

it("keeps an explicit custom meeting custom", async () => {
  const { draft, period } = await editAndSave(meeting([], "09:10", "10:20"));
  expect(period.value).toBe("custom");
  expect(draft.meetings[0]).toMatchObject({
    periods: [],
    startTime: "09:10",
    endTime: "10:20",
  });
});

it("keeps a normal paired teaching slot period-backed", async () => {
  const { draft, period } = await editAndSave(
    meeting([1, 2], "08:50", "10:30"),
  );
  expect(period.value).toBe("1-2");
  expect(draft.meetings[0].periods).toEqual([1, 2]);
});

it("clears periods only when Custom time is selected", async () => {
  const { draft } = await editAndSave(
    meeting([1, 2, 3, 4], "08:50", "12:25"),
    "custom",
  );
  expect(draft.meetings[0].periods).toEqual([]);
});
