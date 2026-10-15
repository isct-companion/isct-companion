// @vitest-environment jsdom
import { MantineProvider } from "@mantine/core";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { seedWorkspace } from "../../lib/model";
import { StudyProfileSettings } from "./StudyProfileSettings";

vi.mock("../../platform/core-query", () => ({
  useScheduleQuery: () => ({ value: { currentYear: 2026 } }),
}));

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

function workspace() {
  const data = seedWorkspace();
  data.selectedTerm = "2026-3Q";
  data.academicProfile.admissionYear = 2026;
  data.academicProfile.studyEnrollments = [
    {
      stage: "undergraduate",
      entryKind: "regular",
      entryGrade: 1,
      admittedTerm: "2026-1Q",
    },
  ];
  return data;
}

function change(select: HTMLSelectElement, value: string) {
  Object.getOwnPropertyDescriptor(
    HTMLSelectElement.prototype,
    "value",
  )!.set!.call(select, value);
  select.dispatchEvent(new Event("change", { bubbles: true }));
}

it("saves consecutive stage edits as one profile without losing fields", async () => {
  let finish!: () => void;
  const pending = new Promise<void>((resolve) => {
    finish = resolve;
  });
  const setStudyProfile = vi.fn(() => pending);
  await act(async () =>
    root.render(
      <MantineProvider>
        <StudyProfileSettings
          data={workspace()}
          update={() => {}}
          setStudyProfile={setStudyProfile}
        />
      </MantineProvider>,
    ),
  );

  const stage = host.querySelector<HTMLSelectElement>(
    '[aria-label="Degree stage, stage 1"]',
  )!;
  const kind = host.querySelector<HTMLSelectElement>(
    '[aria-label="Entry kind, stage 1"]',
  )!;
  const grade = host.querySelector<HTMLSelectElement>(
    '[aria-label="Entry grade, stage 1"]',
  )!;
  const term = host.querySelector<HTMLSelectElement>(
    '[aria-label="Stage admission term, stage 1"]',
  )!;
  await act(async () => {
    change(stage, "professional");
    change(kind, "transfer");
    change(grade, "2");
    change(term, "2026-2Q");
  });
  const save = [...host.querySelectorAll<HTMLButtonElement>("button")].find(
    (button) => button.textContent === "Save study profile",
  )!;
  await act(async () => save.click());

  expect(setStudyProfile).toHaveBeenCalledWith(2026, [
    {
      stage: "professional",
      entryKind: "transfer",
      entryGrade: 2,
      admittedTerm: "2026-2Q",
    },
  ]);
  expect(stage.disabled).toBe(true);
  await act(async () => finish());
});

it("keeps a rejected draft visible for correction", async () => {
  const setStudyProfile = vi
    .fn()
    .mockRejectedValue(new Error("duplicate term"));
  await act(async () =>
    root.render(
      <MantineProvider>
        <StudyProfileSettings
          data={workspace()}
          update={() => {}}
          setStudyProfile={setStudyProfile}
        />
      </MantineProvider>,
    ),
  );
  const stage = host.querySelector<HTMLSelectElement>(
    '[aria-label="Degree stage, stage 1"]',
  )!;
  await act(async () => change(stage, "doctoral"));
  const save = [...host.querySelectorAll<HTMLButtonElement>("button")].find(
    (button) => button.textContent === "Save study profile",
  )!;
  await act(async () => save.click());

  expect(stage.value).toBe("doctoral");
  expect(host.querySelector('[role="alert"]')?.textContent).toContain(
    "duplicate term",
  );
  expect(save.disabled).toBe(false);
});

it("does not overwrite an external profile change with a dirty draft", async () => {
  const initial = workspace();
  const setStudyProfile = vi.fn().mockResolvedValue(undefined);
  const render = (data: ReturnType<typeof workspace>) =>
    root.render(
      <MantineProvider>
        <StudyProfileSettings
          data={data}
          update={() => {}}
          setStudyProfile={setStudyProfile}
        />
      </MantineProvider>,
    );
  await act(async () => render(initial));
  const stage = host.querySelector<HTMLSelectElement>(
    '[aria-label="Degree stage, stage 1"]',
  )!;
  await act(async () => change(stage, "doctoral"));

  const external = structuredClone(initial);
  external.academicProfile.studyEnrollments![0].entryGrade = 2;
  await act(async () => render(external));
  const save = [...host.querySelectorAll<HTMLButtonElement>("button")].find(
    (button) => button.textContent === "Save study profile",
  )!;
  await act(async () => save.click());

  expect(setStudyProfile).not.toHaveBeenCalled();
  expect(stage.value).toBe("doctoral");
  expect(host.querySelector('[role="alert"]')?.textContent).toContain(
    "changed elsewhere",
  );
});

it("adopts a newer external profile after a successful save", async () => {
  const initial = workspace();
  const render = (data: ReturnType<typeof workspace>) =>
    root.render(
      <MantineProvider>
        <StudyProfileSettings
          data={data}
          update={() => {}}
          setStudyProfile={async () => {}}
        />
      </MantineProvider>,
    );
  await act(async () => render(initial));
  const stage = host.querySelector<HTMLSelectElement>(
    '[aria-label="Degree stage, stage 1"]',
  )!;
  await act(async () => change(stage, "master"));
  const save = [...host.querySelectorAll<HTMLButtonElement>("button")].find(
    (button) => button.textContent === "Save study profile",
  )!;
  await act(async () => save.click());
  expect(stage.value).toBe("master");

  const external = structuredClone(initial);
  external.academicProfile.studyEnrollments![0].stage = "doctoral";
  external.academicProfile.studyEnrollments![0].entryGrade = 2;
  await act(async () => render(external));
  expect(
    host.querySelector<HTMLSelectElement>(
      '[aria-label="Degree stage, stage 1"]',
    )?.value,
  ).toBe("doctoral");
});
