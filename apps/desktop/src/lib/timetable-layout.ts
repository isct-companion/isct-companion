import { PERIOD_CLOCKS } from "../generated/persistence";

type PeriodRow = {
  label: string;
  periods: readonly number[];
  start: string;
  end: string;
};

/** Teaching slots only (no lunch), from Core's period clock; used by the
 * course editor. */
export const classSlots: readonly PeriodRow[] = PERIOD_CLOCKS.map(
  ([start, end], index) => ({
    label: `${index * 2 + 1}-${index * 2 + 2}`,
    periods: [index * 2 + 1, index * 2 + 2],
    start,
    end,
  }),
);

/** Timetable rows: the slots with the lunch break after 3-4. */
export const teachingPeriods: readonly PeriodRow[] = [
  ...classSlots.slice(0, 2),
  {
    label: "Lunch",
    periods: [],
    start: classSlots[1].end,
    end: classSlots[2].start,
  },
  ...classSlots.slice(2),
];

export const timetableStartMinutes = clockMinutes(classSlots[0].start);
export const timetableEndMinutes = clockMinutes(
  classSlots[classSlots.length - 1].end,
);

export function clockMinutes(value: string): number {
  const match = /^(\d{1,2}):(\d{2})$/.exec(value);
  if (!match) throw new Error(`Invalid clock time: ${value}`);
  return Number(match[1]) * 60 + Number(match[2]);
}

export function timetablePosition(start: string, end: string) {
  const range = timetableEndMinutes - timetableStartMinutes;
  const from = Math.max(timetableStartMinutes, clockMinutes(start));
  const to = Math.min(timetableEndMinutes, clockMinutes(end));
  return {
    topPercent: ((from - timetableStartMinutes) / range) * 100,
    heightPercent: (Math.max(20, to - from) / range) * 100,
  };
}

/** Percent offset of a clock time inside the teaching day, or undefined outside it. */
export function timetableOffset(minutes: number): number | undefined {
  if (minutes < timetableStartMinutes || minutes > timetableEndMinutes)
    return undefined;
  return (
    ((minutes - timetableStartMinutes) /
      (timetableEndMinutes - timetableStartMinutes)) *
    100
  );
}
