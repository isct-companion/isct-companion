import { describe, expect, it } from "vitest";
import {
  seedWorkspace,
  type AcademicCalendarDay,
  type Workspace,
} from "./model";
import { academicYearCalendar, academicYearOf } from "./calendar-view";

const meta = (id: string) => ({
  id,
  rev: 1,
  updatedAt: "2026-10-01T00:00:00Z",
  updatedBy: "reference",
});

function officialDay(
  date: string,
  values: Partial<AcademicCalendarDay>,
): AcademicCalendarDay {
  return {
    ...meta(`calendar:2026:day:${date}`),
    academicYear: 2026,
    date,
    type: "no-class",
    label: { ja: "授業休み", en: "No classes" },
    ...values,
  };
}

function workspace(): Workspace {
  const data = seedWorkspace();
  data.academicCalendar = [
    {
      ...meta("calendar:2026:quarter-start:3Q"),
      academicYear: 2026,
      quarter: "3Q",
      date: "2026-09-28",
      type: "quarter-start",
      label: { en: "3Q starts" },
    },
    {
      ...meta("calendar:2026:quarter-end:3Q"),
      academicYear: 2026,
      quarter: "3Q",
      date: "2026-12-02",
      type: "quarter-end",
      label: { en: "3Q ends" },
    },
    officialDay("2026-10-09", { label: { en: "Campus Festival" } }),
    officialDay("2026-10-15", {
      type: "replacement",
      replacementDay: 1,
      label: { en: "Monday classes" },
    }),
    officialDay("2026-11-03", { missingSince: "2026-10-05T00:00:00Z" }),
    officialDay("2025-10-10", { academicYear: 2025 }),
  ];
  data.exceptions = [
    {
      ...meta("own-1"),
      date: "2026-10-15",
      type: "no-class",
      label: "Lab trip",
    },
    {
      ...meta("own-2"),
      date: "2027-01-05",
      type: "break",
      label: "Winter break",
    },
    { ...meta("own-old"), date: "2026-03-30", type: "break", label: "Old" },
  ];
  return data;
}

describe("academic year calendar view", () => {
  it("assigns January to March to the previous academic year", () => {
    expect(academicYearOf("2027-03-31")).toBe(2026);
    expect(academicYearOf("2026-04-01")).toBe(2026);
  });

  it("merges published days with personal changes for one year", () => {
    const view = academicYearCalendar(workspace(), 2026);
    expect(view.published).toBe(true);
    expect(
      view.days.map(({ date, origin, label }) => ({ date, origin, label })),
    ).toEqual([
      { date: "2026-10-09", origin: "official", label: "Campus Festival" },
      { date: "2026-10-15", origin: "changed", label: "Lab trip" },
      { date: "2027-01-05", origin: "own", label: "Winter break" },
    ]);
    expect(view.days[1].official).toEqual({
      type: "replacement",
      replacementDay: 1,
      label: "Monday classes",
    });
  });

  it("labels quarters by where their dates came from", () => {
    const data = workspace();
    const view = academicYearCalendar(data, 2026);
    expect(view.quarters[2]).toEqual({
      quarter: "3Q",
      from: "2026-09-28",
      to: "2026-12-02",
      origin: "official",
    });
    expect(view.quarters[0].origin).toBe("estimated");
    data.academicProfile.years = [
      {
        academicYear: 2026,
        quarterStarts: {
          "1Q": "2026-04-07",
          "2Q": "2026-06-10",
          "3Q": "2026-10-01",
          "4Q": "2026-12-03",
        },
      },
    ];
    expect(academicYearCalendar(data, 2026).quarters[2]).toMatchObject({
      from: "2026-10-01",
      origin: "own",
    });
  });

  it("reports an unpublished year", () => {
    expect(academicYearCalendar(workspace(), 2027)).toMatchObject({
      published: false,
      days: [],
    });
  });
});
