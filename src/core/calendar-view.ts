import {
  contentLanguage,
  pickLocalized,
  type CalendarException,
  type ImportantDate,
  type Quarter,
  type Workspace,
} from "./model";
import { quarterRange } from "./schedule";

const QUARTERS: Quarter[] = ["1Q", "2Q", "3Q", "4Q"];
const DAY_TYPES = ["holiday", "no-class", "replacement", "break"] as const;
export type CalendarDayType = (typeof DAY_TYPES)[number];

/** Where a calendar value comes from, as shown to the person. */
export type CalendarOrigin = "official" | "own" | "changed" | "estimated";

export type CalendarDayView = {
  date: string;
  type: CalendarDayType;
  replacementDay?: number;
  label: string;
  origin: Exclude<CalendarOrigin, "estimated">;
  /** The published row a personal change replaces. */
  official?: { type: CalendarDayType; replacementDay?: number; label: string };
  exception?: CalendarException;
};

export type QuarterView = {
  quarter: Quarter;
  from: string;
  to: string;
  origin: Exclude<CalendarOrigin, "changed">;
};

export type AcademicYearCalendar = {
  year: number;
  quarters: QuarterView[];
  days: CalendarDayView[];
  importantDates: ImportantDate[];
  /** True when the subscription supplied any row for this year. */
  published: boolean;
};

/** Academic years start in April (Asia/Tokyo civil dates). */
export function academicYearOf(date: string) {
  const year = Number(date.slice(0, 4));
  return Number(date.slice(5, 7)) >= 4 ? year : year - 1;
}

const inYear = (date: string, year: number) =>
  date >= `${year}-04-01` && date <= `${year + 1}-03-31`;

/**
 * One academic year as the person sees it: published quarters and teaching-day
 * changes merged with their own overrides. The same precedence as
 * `calendarState`: a personal exception replaces the published row on its date.
 */
export function academicYearCalendar(
  data: Workspace,
  year: number,
): AcademicYearCalendar {
  const locale = contentLanguage(data.settings);
  const officialRows = data.academicCalendar.filter(
    (row) =>
      !row.deletedAt &&
      !row.missingSince &&
      (row.academicYear ?? academicYearOf(row.date)) === year,
  );
  const official = new Map(
    officialRows
      .filter((row) => (DAY_TYPES as readonly string[]).includes(row.type))
      .map((row) => [
        row.date,
        {
          type: row.type as CalendarDayType,
          replacementDay: row.replacementDay,
          label: pickLocalized(row.label, locale, row.date),
        },
      ]),
  );
  const own = data.exceptions.filter(
    (row) => !row.deletedAt && inYear(row.date, year),
  );
  const days: CalendarDayView[] = own.map((exception) => ({
    date: exception.date,
    type: exception.type,
    replacementDay: exception.replacementDay,
    label: exception.label,
    origin: official.has(exception.date) ? "changed" : "own",
    official: official.get(exception.date),
    exception,
  }));
  const changedDates = new Set(days.map((day) => day.date));
  for (const [date, row] of official)
    if (!changedDates.has(date))
      days.push({ date, ...row, origin: "official" });
  days.sort((a, b) => a.date.localeCompare(b.date));

  const profileYear = data.academicProfile.years.some(
    (row) => row.academicYear === year,
  );
  const publishedQuarters = new Set(
    officialRows
      .filter((row) => row.type === "quarter-start")
      .map((row) => row.quarter ?? row.id.split(":").pop()),
  );
  const quarters = QUARTERS.map((quarter) => {
    const range = quarterRange(data, `${year}-${quarter}`);
    return {
      quarter,
      from: range.from,
      to: range.to,
      origin: profileYear
        ? ("own" as const)
        : publishedQuarters.has(quarter)
          ? ("official" as const)
          : ("estimated" as const),
    };
  });

  const importantDates = data.importantDates
    .filter(
      (row) => !row.deletedAt && !row.missingSince && row.academicYear === year,
    )
    .sort((a, b) => a.startDate.localeCompare(b.startDate));

  return {
    year,
    quarters,
    days,
    importantDates,
    published: officialRows.length > 0,
  };
}
