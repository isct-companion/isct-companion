export const teachingPeriods = [
  { label: "1-2", periods: [1, 2], start: "08:50", end: "10:30" },
  { label: "3-4", periods: [3, 4], start: "10:45", end: "12:25" },
  { label: "Lunch", periods: [], start: "12:25", end: "13:30" },
  { label: "5-6", periods: [5, 6], start: "13:30", end: "15:10" },
  { label: "7-8", periods: [7, 8], start: "15:25", end: "17:05" },
  { label: "9-10", periods: [9, 10], start: "17:15", end: "18:55" },
] as const;

/** Teaching slots only (no lunch); used by the course editor. */
export const classSlots = teachingPeriods.filter(
  (period) => period.periods.length > 0,
);

export const timetableStartMinutes = 8 * 60 + 50;
export const timetableEndMinutes = 18 * 60 + 55;

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
