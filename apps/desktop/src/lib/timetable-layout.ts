import {
  DEFAULT_TEACHING_PERIODS,
  type TeachingPeriod,
  type TimetableAxis,
} from "../generated/persistence";

export type ClassSlot = {
  label: string;
  periods: readonly [number, number];
  start: string;
  end: string;
};

export const resolvedTeachingPeriods = (
  periods: readonly TeachingPeriod[] | null | undefined,
): readonly TeachingPeriod[] => periods ?? DEFAULT_TEACHING_PERIODS;

export const classSlots = (periods: readonly TeachingPeriod[]): ClassSlot[] =>
  periods.map((period) => ({
    label: `${period.periods[0]}-${period.periods[1]}`,
    periods: period.periods,
    start: period.startTime,
    end: period.endTime,
  }));

export function clockMinutes(value: string): number {
  const match = /^(\d{1,2}):(\d{2})$/.exec(value);
  if (!match) throw new Error(`Invalid clock time: ${value}`);
  return Number(match[1]) * 60 + Number(match[2]);
}

/** Core's timetable axis (`timetable_axis`): every period one row, lunch
 * half a row, short breaks folded into the rules. Shells only interpolate
 * between its stops, so desktop and mobile place times identically. */
export function axisOffset(axis: TimetableAxis, minutes: number): number {
  const { stops } = axis;
  if (!stops.length) return 0;
  if (minutes <= stops[0].minutes) return stops[0].offset;
  const last = stops[stops.length - 1];
  if (minutes >= last.minutes) return last.offset;
  const index = stops.findIndex((stop) => minutes <= stop.minutes);
  const [from, to] = [stops[index - 1], stops[index]];
  const span = Math.max(to.minutes - from.minutes, 1);
  return (
    from.offset + ((to.offset - from.offset) * (minutes - from.minutes)) / span
  );
}

const percentOf = (axis: TimetableAxis, rows: number) =>
  axis.length > 0 ? (rows / axis.length) * 100 : 0;

/** A span of the day as top and height percentages of the grid. */
export function axisPosition(axis: TimetableAxis, start: string, end: string) {
  const top = axisOffset(axis, clockMinutes(start));
  const bottom = axisOffset(axis, clockMinutes(end));
  return {
    topPercent: percentOf(axis, top),
    // A block of a few minutes still shows its title.
    heightPercent: percentOf(axis, Math.max(bottom - top, 0.4)),
  };
}

/** The axis rows to draw: teaching blocks and the lunch band. */
export const axisRows = (axis: TimetableAxis) =>
  axis.rows.map((row) => ({
    label: row.periods
      ? `${row.periods[0]}-${row.periods[1]}`
      : "timetable.breakBand",
    periods: row.periods ?? null,
    start: row.startTime,
    end: row.endTime,
    isBreak: !row.periods,
    topPercent: percentOf(axis, row.top),
    heightPercent: percentOf(axis, row.bottom - row.top),
  }));

/** The now line's place, while now falls inside the drawn day. */
export function nowPercent(
  axis: TimetableAxis,
  minutes: number,
): number | undefined {
  const first = axis.stops[0];
  const last = axis.stops[axis.stops.length - 1];
  if (!first || !last || minutes < first.minutes || minutes > last.minutes)
    return undefined;
  return percentOf(axis, axisOffset(axis, minutes));
}
