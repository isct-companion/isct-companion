import { describe, expect, it } from "vitest";
import {
  seedWorkspace,
  type Course,
  type Plan,
  type AcademicCalendarDay,
} from "./model";
import { occurrencesOnDate, quarterRange, tokyoDate } from "./schedule";
import { exportQuarterIcs } from "./calendar-ics";
import {
  nextPlanOccurrences,
  planOccursOn,
  clockForPeriods,
} from "./user-schedule";
const meta = { rev: 1, updatedAt: "2026-01-01T00:00:00Z", updatedBy: "test" };
function fixture() {
  const data = seedWorkspace();
  data.selectedTerm = "2026-1Q";
  const course: Course = {
    ...meta,
    id: "course",
    kind: "course",
    termId: data.selectedTerm,
    title: "授業, A; B",
    code: "CSC.T241",
    instructor: "",
    credits: 2,
    creditSplit: { lecture: 2, exercise: 0, lab: 0 },
    instructionLanguage: "ja",
    color: "#123456",
    source: { provider: "manual" },
    meetings: [
      {
        days: [1],
        periods: [1, 2],
        startTime: "08:50",
        endTime: "10:30",
        room: "M-101",
      },
    ],
  };
  data.courses = [course];
  data.memberships = [
    {
      ...meta,
      id: "member",
      kind: "membership",
      termId: data.selectedTerm,
      courseId: course.id,
      status: "declared",
      evidence: { provider: "manual" },
    },
  ];
  data.academicCalendar = [
    {
      ...meta,
      id: "calendar:2026:quarter-start:1Q",
      academicYear: 2026,
      quarter: "1Q",
      date: "2026-04-07",
      type: "quarter-start",
      label: { ja: "開始" },
      teachingRanges: [
        { start: "2026-04-07", end: "2026-05-27" },
        { start: "2026-05-30", end: "2026-06-03" },
      ],
    },
    {
      ...meta,
      id: "calendar:2026:quarter-end:1Q",
      academicYear: 2026,
      quarter: "1Q",
      date: "2026-06-09",
      type: "quarter-end",
      label: { ja: "終了" },
    },
  ];
  return data;
}
const plan: Plan = {
  ...meta,
  id: "plan",
  title: "Personal plan",
  start: "2026-03-30T09:00:00+09:00",
  duration: 60,
  rrule: "FREQ=WEEKLY;BYDAY=MO;COUNT=10",
  exdates: [],
};
function day(
  date: string,
  type: AcademicCalendarDay["type"],
  replacementDay?: number,
): AcademicCalendarDay {
  return {
    ...meta,
    id: date,
    date,
    type,
    replacementDay,
    label: { ja: "例外" },
  };
}

describe("shared actual-date scheduling", () => {
  it("keeps all Japan date arithmetic independent of machine timezone", () => {
    expect(tokyoDate(new Date("2026-04-12T15:01:00Z"))).toBe("2026-04-13");
  });
  it("uses teaching intervals, not the exam and preparation intervals", () => {
    const data = fixture();
    expect(quarterRange(data, data.selectedTerm)).toMatchObject({
      from: "2026-04-07",
      to: "2026-06-09",
    });
    expect(occurrencesOnDate(data, data.selectedTerm, "2026-04-06")).toEqual(
      [],
    );
    expect(
      occurrencesOnDate(data, data.selectedTerm, "2026-04-13"),
    ).toHaveLength(1);
    expect(occurrencesOnDate(data, data.selectedTerm, "2026-06-08")).toEqual(
      [],
    );
  });
  it("suppresses holiday courses but keeps civil-date plans and events", () => {
    const data = fixture();
    data.academicCalendar.push(day("2026-04-13", "holiday"));
    data.plans = [plan];
    data.events = [
      {
        ...meta,
        id: "event",
        kind: "event",
        termId: data.selectedTerm,
        title: "Event",
        date: "2026-04-13",
        room: "",
        startTime: "09:30",
        endTime: "10:00",
      },
    ];
    const rows = occurrencesOnDate(data, data.selectedTerm, "2026-04-13");
    expect(rows.map((r) => r.kind).sort()).toEqual(["event", "plan"]);
    expect(rows.every((r) => r.conflict)).toBe(true);
  });
  it("uses replacement weekdays only for courses, with manual exception priority", () => {
    const data = fixture();
    data.academicCalendar.push(day("2026-04-14", "replacement", 1));
    data.plans = [plan];
    expect(
      occurrencesOnDate(data, data.selectedTerm, "2026-04-14").map(
        (r) => r.kind,
      ),
    ).toEqual(["course"]);
    data.exceptions.push({
      ...meta,
      id: "manual",
      date: "2026-04-14",
      type: "no-class",
      label: "Override",
    });
    expect(occurrencesOnDate(data, data.selectedTerm, "2026-04-14")).toEqual(
      [],
    );
  });
  it("never restores deleted intensive dates or infers unselected days", () => {
    const data = fixture();
    data.courses[0].meetings = [];
    data.intensiveSchedules = [
      {
        ...meta,
        id: "intensive",
        courseId: "course",
        from: "2026-04-12",
        to: "2026-04-14",
        periods: [1, 2],
        days: [
          { date: "2026-04-12", periods: [3, 4] },
          { date: "2026-04-14", periods: null },
        ],
      },
    ];
    data.academicCalendar.push(day("2026-04-12", "holiday"));
    expect(
      occurrencesOnDate(data, data.selectedTerm, "2026-04-12")[0].startTime,
    ).toBe("10:45");
    expect(occurrencesOnDate(data, data.selectedTerm, "2026-04-13")).toEqual(
      [],
    );
    expect(occurrencesOnDate(data, data.selectedTerm, "2026-04-14")).toEqual(
      [],
    );
  });
  it("honors per-meeting validity and renamed course details", () => {
    const data = fixture();
    data.courses[0].meetings[0].validFrom = "2026-04-20";
    data.courses[0].overrides = { title: "My title" };
    expect(occurrencesOnDate(data, data.selectedTerm, "2026-04-13")).toEqual(
      [],
    );
    expect(
      occurrencesOnDate(data, data.selectedTerm, "2026-04-20")[0].title,
    ).toBe("My title");
  });
});
describe("personal recurrence", () => {
  it("counts from DTSTART, honors intervals, ordinal weekdays, and exclusions", () => {
    const row = {
      ...plan,
      rrule: "FREQ=MONTHLY;BYDAY=1MO;COUNT=3",
      exdates: ["2026-05-04"],
    };
    expect(nextPlanOccurrences(row)).toEqual([
      "2026-04-06T09:00:00+09:00",
      "2026-06-01T09:00:00+09:00",
    ]);
    expect(planOccursOn(row, "2026-07-06")).toBe(false);
    expect(
      planOccursOn(
        { ...plan, rrule: "FREQ=WEEKLY;INTERVAL=2;COUNT=2" },
        "2026-04-13",
      ),
    ).toBe(true);
    expect(
      planOccursOn(
        { ...plan, rrule: "FREQ=WEEKLY;INTERVAL=2;COUNT=2" },
        "2026-04-20",
      ),
    ).toBe(false);
  });
  it("interprets UTC starts and UNTIL in Japan civil time", () => {
    const row = {
      ...plan,
      start: "2026-04-12T15:30:00Z",
      rrule: "FREQ=DAILY;UNTIL=20260413T153000Z",
    };
    expect(nextPlanOccurrences(row)).toEqual([
      "2026-04-13T00:30:00+09:00",
      "2026-04-14T00:30:00+09:00",
    ]);
  });
  it("rejects invalid teaching periods", () => {
    expect(() => clockForPeriods([0, 10])).toThrow();
    expect(() => clockForPeriods([4, 3])).toThrow();
  });
});
describe("calendar export shares the schedule", () => {
  it("defaults to courses and only adds plans with the explicit option", () => {
    const data = fixture();
    data.plans = [plan];
    data.academicCalendar.push(day("2026-04-13", "holiday"));
    const standard = exportQuarterIcs(data, data.selectedTerm);
    expect(standard).not.toContain("SUMMARY:Personal plan");
    expect(standard).not.toContain("DTSTART;TZID=Asia/Tokyo:20260413");
    expect(standard).toContain("SUMMARY:授業\\, A\\; B");
    const expanded = exportQuarterIcs(
      data,
      data.selectedTerm,
      "courses-and-plans",
    );
    expect(expanded).toContain("SUMMARY:Personal plan");
    expect(expanded).toContain("DTSTART;TZID=Asia/Tokyo:20260413T090000");
    const count = standard.match(/BEGIN:VEVENT/g)?.length ?? 0;
    let expected = 0;
    for (
      let date = new Date("2026-04-07T00:00:00Z");
      date <= new Date("2026-06-09T00:00:00Z");
      date.setUTCDate(date.getUTCDate() + 1)
    )
      expected += occurrencesOnDate(
        data,
        data.selectedTerm,
        date.toISOString().slice(0, 10),
        { includePersonal: false },
      ).length;
    expect(count).toBe(expected);
  });
  it("rolls overnight plans into the next date and folds Japanese text by UTF-8 bytes", () => {
    const data = fixture();
    data.plans = [
      {
        ...plan,
        start: "2026-04-13T23:30:00+09:00",
        rrule: undefined,
        title: "日本語".repeat(30),
        duration: 90,
      },
    ];
    const ics = exportQuarterIcs(data, data.selectedTerm, "courses-and-plans");
    expect(ics).toContain("DTEND;TZID=Asia/Tokyo:20260414T010000");
    expect(
      ics
        .split("\r\n")
        .every((line) => new TextEncoder().encode(line).length <= 75),
    ).toBe(true);
  });
});

it("shows the same course template while explaining a festival closure in the dated week", async () => {
  const {templateWeekColumns,weekColumnsAt,timetableCourses}=await import("../components/timetable/workspace");
  const data=fixture();data.selectedTerm="2026-3Q";data.courses[0].termId=data.selectedTerm;data.memberships[0].termId=data.selectedTerm;
  data.courses[0].meetings[0].days=[2,5];
  data.academicCalendar=[{...meta,id:"festival",academicYear:2026,date:"2026-10-09",type:"no-class",label:{en:"No classes due to Campus Festival",ja:"工大祭のため授業休み"}}];
  const courses=timetableCourses(data,data.selectedTerm);
  const template=templateWeekColumns(courses,[]);
  const dated=weekColumnsAt(data,courses,[],new Date("2026-10-05T12:00:00+09:00"));
  expect(template[4].blocks).toHaveLength(1);
  expect(dated[4].blocks).toHaveLength(1);
  expect(dated[4].blocks[0].cancelled).toBe(true);
  expect(dated[4].noClasses).toBe(true);
  expect(occurrencesOnDate(data,data.selectedTerm,"2026-10-09")).toHaveLength(0);
  expect(dated[4].calendarNotice).toBe("Campus Festival");
  expect(dated[1].blocks).toHaveLength(1);
});

it.each(["holiday", "no-class", "break"] as const)("retains grey course context on %s without cancelling personal events", type => {
  const data = fixture();
  const date = "2026-04-13";
  data.academicCalendar.push(day(date, type));
  const event = { ...meta, id: "personal", kind: "event" as const, termId: data.selectedTerm, date, title: "Personal", room: "", startTime: "09:00", endTime: "10:00" };
  const shown = occurrencesOnDate(data, data.selectedTerm, date, { events: [event], includeCancelledCourses: true });
  expect(shown.some(row => row.kind === "course" && row.cancelled)).toBe(true);
  expect(shown.find(row => row.id === "personal")?.cancelled).toBeUndefined();
  expect(shown.every(row => !row.conflict)).toBe(true);
  const actual = occurrencesOnDate(data, data.selectedTerm, date, { events: [event] });
  expect(actual.map(row => row.id)).toEqual(["personal"]);
});
