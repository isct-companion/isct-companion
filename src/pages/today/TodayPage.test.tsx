// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { seedWorkspace, type Assignment, type Course } from "../../core/model";
import { Today } from "./TodayPage";

const core = vi.hoisted(() => ({ tasks: undefined as unknown }));
vi.mock("../../platform/core-query", () => ({
  useScheduleQuery: (query: { kind: string }) => ({
    value: query.kind === "taskView" ? core.tasks : undefined,
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
  vi.useFakeTimers({ now: new Date("2026-10-07T03:00:00Z") });
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.useRealTimers();
});

const meta = {
  rev: 1,
  updatedAt: "2026-10-05T00:00:00Z",
  updatedBy: "test",
};
const course = (id: string, termId: Course["termId"]): Course => ({
  ...meta,
  id,
  kind: "course",
  termId,
  code: id.toUpperCase(),
  title: `Course ${id}`,
  instructor: "",
  credits: 2,
  creditSplit: { lecture: 2, exercise: 0, lab: 0 },
  instructionLanguage: "ja",
  color: "blue",
  meetings: [],
});
const assignment = (
  id: string,
  courseId: string,
  dueAt: string,
): Assignment => ({
  ...meta,
  id,
  kind: "assignment",
  courseId,
  title: `Report ${id}`,
  dueAt,
  submission: "open",
});

const render = async (data: ReturnType<typeof seedWorkspace>) => {
  const ordered = data.assignments.map((item) => item.id);
  core.tasks = {
    byTerm: {
      "2026-3Q": {
        pendingIds: ordered.filter((id) => ["upcoming", "pinned"].includes(id)),
        pinnedIds: ordered.filter((id) => id === "pinned"),
        overdueIds: ordered
          .filter((id) => ["closed", "missed"].includes(id))
          .reverse(),
        dueThisWeekIds: ordered.filter((id) => id === "upcoming"),
        laterIds: [],
        earlierDueThisWeekIds: ordered.filter((id) =>
          ["upcoming", "handed-in"].includes(id),
        ),
        earlierLaterIds: [],
      },
    },
    nearestByCourse: {},
    deadlines: Object.fromEntries(
      data.assignments.map((item) => [
        item.id,
        {
          label: "Due soon",
          tone: "normal",
          countdownLabel: "1d",
          countdownTone: "soon",
          overdue: ["closed", "missed"].includes(item.id),
        },
      ]),
    ),
  };
  await act(async () =>
    root.render(
      <Today
        data={data}
        term="2026-3Q"
        open={() => {}}
        openEvent={() => {}}
        openPlan={() => {}}
        openTask={() => {}}
        hideAssignment={() => {}}
        openSchedule={() => {}}
      />,
    ),
  );
  return host.querySelector('[data-today-section="assignments"]')!;
};
const rowOf = (section: Element, title: string) =>
  [...section.querySelectorAll(".dueRow")].find((row) =>
    row.textContent?.includes(title),
  );

describe("Today assignments", () => {
  const workspace = () => {
    const data = seedWorkspace();
    data.courses = [course("now", "2026-3Q"), course("old", "2026-2Q")];
    data.assignments = [
      assignment("upcoming", "now", "2026-10-09T00:00:00Z"),
      {
        ...assignment("handed-in", "now", "2026-10-12T00:00:00Z"),
        submission: "submitted",
      },
      assignment("missed", "now", "2026-10-03T00:00:00Z"),
      {
        ...assignment("closed", "now", "2026-10-02T00:00:00Z"),
        submission: "graded",
      },
      assignment("stale", "old", "2026-08-01T00:00:00Z"),
    ];
    return data;
  };

  it("keeps a pinned assignment above earlier deadlines without duplicating it", async () => {
    const data = workspace();
    data.assignments.push({
      ...assignment("pinned", "now", "2026-11-12T00:00:00Z"),
      pinned: true,
    });
    const section = await render(data);
    const titles = [...section.querySelectorAll(".dueRow b")].map(
      (node) => node.textContent,
    );
    expect(titles[0]).toBe("Report pinned");
    expect(titles.filter((title) => title === "Report pinned")).toHaveLength(1);
  });

  it("shows only open work from now on under Today", async () => {
    const section = await render(workspace());
    const titles = [...section.querySelectorAll(".dueRow b")].map(
      (title) => title.textContent,
    );
    expect(titles).toEqual(["Report upcoming"]);
    expect(section.textContent).toContain("1 active");
  });

  it("adds submitted and past work of this quarter under Earlier", async () => {
    const section = await render(workspace());
    const earlier = [...section.querySelectorAll(".dueScope button")].find(
      (button) => button.textContent === "Earlier",
    )!;
    await act(async () => (earlier as HTMLButtonElement).click());
    expect(rowOf(section, "Report upcoming")?.className).toBe("dueRow");
    expect(rowOf(section, "Report handed-in")?.className).toContain("done");
    expect(rowOf(section, "Report missed")?.className).toContain("missed");
    expect(rowOf(section, "Report closed")?.className).toContain("closed");
    expect(rowOf(section, "Report stale")).toBeUndefined();
    expect(section.textContent).toContain("Past deadline · 2");
  });
});
