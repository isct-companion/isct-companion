// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { seedWorkspace, type Course, type Meeting } from "../../core/model";
import { LectureDesk } from "./LectureDesk";

const core = vi.hoisted(() => ({
  schedule: undefined as unknown,
  tasks: undefined as unknown,
}));
vi.mock("../../platform/core-query", () => ({
  useScheduleQuery: (query: { kind: string }) => ({
    value: query.kind === "taskView" ? core.tasks : core.schedule,
    pending: false,
  }),
}));

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  core.schedule = undefined;
  core.tasks = undefined;
  // Wednesday 7 October, 12:00 in Japan.
  vi.useFakeTimers({ now: new Date("2026-10-07T03:00:00Z") });
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.useRealTimers();
});

const meta = { rev: 1, updatedAt: "2026-10-05T00:00:00Z", updatedBy: "test" };
const course = (id: string, meetings: Meeting[]): Course => ({
  ...meta,
  id,
  kind: "course",
  termId: "2026-3Q",
  code: id.toUpperCase(),
  title: `Course ${id}`,
  instructor: "",
  credits: 2,
  creditSplit: { lecture: 2, exercise: 0, lab: 0 },
  instructionLanguage: "ja",
  color: "blue",
  meetings,
});
const meeting = (day: number, startTime: string, endTime: string): Meeting => ({
  days: [day],
  periods: [],
  startTime,
  endTime,
  room: `R${day}`,
});

it("lists lectures in class order and waits for an explicit selection", async () => {
  const data = seedWorkspace();
  data.academicProfile.years = [
    {
      academicYear: 2026,
      quarterStarts: {
        "1Q": "2026-04-06",
        "2Q": "2026-06-10",
        "3Q": "2026-10-01",
        "4Q": "2026-12-08",
      },
    },
  ];
  data.courses = [
    course("friday", [meeting(5, "08:50", "10:30")]),
    course("later", [meeting(3, "13:30", "15:10")]),
    course("now", [meeting(3, "10:45", "12:25")]),
  ];
  data.memberships = data.courses.map((item) => ({
    ...meta,
    id: `m-${item.id}`,
    kind: "membership" as const,
    termId: item.termId,
    courseId: item.id,
    status: "lms-member" as const,
    evidence: { provider: "lms" as const, externalId: item.id },
  }));
  data.assignments = [
    {
      ...meta,
      id: "a1",
      kind: "assignment",
      courseId: "later",
      title: "Report",
      dueAt: "2026-10-08T15:00:00Z",
      submission: "open",
    },
  ];
  const select = vi.fn();
  const block = (
    id: string,
    date: string,
    startTime: string,
    endTime: string,
    room: string,
  ) => ({
    key: `${id}-${date}`,
    id,
    kind: "course",
    date,
    title: `Course ${id}`,
    subtitle: id.toUpperCase(),
    room,
    startTime,
    endTime,
    color: "blue",
  });
  core.schedule = {
    allTerms: ["2026-3Q"],
    courses: data.courses,
    agendas: [
      {
        date: "2026-10-07",
        occurrences: [
          block("now", "2026-10-07", "10:45", "12:25", "R3"),
          block("later", "2026-10-07", "13:30", "15:10", "R3"),
        ],
      },
      {
        date: "2026-10-08",
        occurrences: [],
      },
      {
        date: "2026-10-09",
        occurrences: [block("friday", "2026-10-09", "08:50", "10:30", "R5")],
      },
    ],
  };
  core.tasks = {
    byTerm: { "2026-3Q": { lectureOpenIds: ["a1"], lectureHiddenIds: [] } },
    nearestByCourse: { later: "a1" },
    deadlines: { a1: { countdownLabel: "1d 12h", countdownTone: "soon" } },
  };
  await act(async () =>
    root.render(
      <LectureDesk
        data={data}
        term="2026-3Q"
        entry={null}
        select={select}
        clear={() => {}}
        active
        renderWorkspace={() => null}
      />,
    ),
  );
  const rows = [...host.querySelectorAll(".lectureRow")];
  expect(rows.map((row) => row.querySelector("b")?.textContent)).toEqual([
    "Course now",
    "Course later",
    "Course friday",
  ]);
  expect(rows[0].querySelector("small")?.textContent).toBe("Now · R3");
  expect(rows[1].querySelector(".dueChip")?.textContent).toBe("1d 12h");
  expect(select).not.toHaveBeenCalled();
  await act(async () => (rows[0] as HTMLButtonElement).click());
  expect(select).toHaveBeenCalledWith("now");

  const filter = host.querySelector<HTMLInputElement>(
    ".lectureFilter input, input.lectureFilter",
  )!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(filter, "FRI");
    filter.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(
    [...host.querySelectorAll(".lectureRow b")].map((b) => b.textContent),
  ).toEqual(["Course friday"]);

  // The same list switches to the quarter's open assignments.
  await act(async () => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(filter, "");
    filter.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const assignments = [
    ...host.querySelectorAll<HTMLButtonElement>(".lectureMode button"),
  ][1];
  await act(async () => assignments.click());
  const work = [...host.querySelectorAll<HTMLButtonElement>(".lectureRow")];
  expect(work.map((row) => row.querySelector("b")?.textContent)).toEqual([
    "Report",
  ]);
  expect(work[0].querySelector("small")?.textContent).toBe("Course later");
  await act(async () => work[0].click());
  expect(select).toHaveBeenLastCalledWith("later", "a1");

  // Hidden assignments fold under the open ones and can be restored there.
  const restore = vi.fn();
  data.assignments = [
    ...data.assignments,
    {
      ...meta,
      id: "a2",
      kind: "assignment",
      courseId: "now",
      title: "Quiz",
      dueAt: "2026-10-09T15:00:00Z",
      submission: "open",
      hidden: true,
    },
  ];
  core.tasks = {
    ...(core.tasks as object),
    byTerm: {
      "2026-3Q": { lectureOpenIds: ["a1"], lectureHiddenIds: ["a2"] },
    },
  };
  await act(async () =>
    root.render(
      <LectureDesk
        data={{ ...data }}
        term="2026-3Q"
        entry={null}
        select={select}
        clear={() => {}}
        active
        renderWorkspace={() => null}
        restoreAssignment={restore}
      />,
    ),
  );
  expect(host.textContent).not.toContain("Quiz");
  await act(async () =>
    host.querySelector<HTMLButtonElement>(".lectureHiddenToggle")!.click(),
  );
  const hidden = host.querySelector(".lectureRow.hidden")!;
  expect(hidden.querySelector("b")?.textContent).toBe("Quiz");
  await act(async () => hidden.querySelector("button")!.click());
  expect(restore).toHaveBeenCalledWith("a2");
});
