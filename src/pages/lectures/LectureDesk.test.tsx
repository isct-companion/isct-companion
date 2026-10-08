// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { dueCountdown } from "../../core/academic";
import { seedWorkspace, type Course, type Meeting } from "../../core/model";
import { LectureDesk } from "./LectureDesk";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
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

it("lists lectures in class order and opens the one in class", async () => {
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
  expect(select).toHaveBeenCalledWith("now");

  const filter = host.querySelector<HTMLInputElement>(".lectureFilter input, input.lectureFilter")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(filter, "FRI");
    filter.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect([...host.querySelectorAll(".lectureRow b")].map((b) => b.textContent)).toEqual(["Course friday"]);
});

it("counts down to the hour inside three days and in days beyond", () => {
  const now = new Date("2026-10-07T00:00:00Z");
  expect(dueCountdown("2026-10-07T00:40:00Z", now)).toMatchObject({
    label: "40m",
    tone: "urgent",
  });
  expect(dueCountdown("2026-10-08T12:00:00Z", now)).toMatchObject({
    label: "1d 12h",
    tone: "soon",
  });
  expect(dueCountdown("2026-10-12T05:00:00Z", now)).toMatchObject({
    label: "5d",
    tone: "normal",
  });
});
