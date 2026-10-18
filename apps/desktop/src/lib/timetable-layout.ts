import {
  DEFAULT_TEACHING_PERIODS,
  type TeachingPeriod,
} from "../generated/persistence";

export type ClassSlot = {
  label: string;
  periods: readonly [number, number];
  start: string;
  end: string;
};

export type TimetableBounds = { start: number; end: number };

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

/** Axis rows, including the long midday gap as layout-only information. */
export const teachingRows = (periods: readonly TeachingPeriod[]) => {
  const slots = classSlots(periods);
  return slots.flatMap((slot, index) => {
    const previous = slots[index - 1];
    const gap = previous
      ? clockMinutes(slot.start) - clockMinutes(previous.end)
      : 0;
    return gap >= 30
      ? [
          {
            label: "Class break",
            periods: [] as const,
            start: previous.end,
            end: slot.start,
            isBreak: true,
          },
          { ...slot, isBreak: false },
        ]
      : [{ ...slot, isBreak: false }];
  });
};

export function clockMinutes(value: string): number {
  const match = /^(\d{1,2}):(\d{2})$/.exec(value);
  if (!match) throw new Error(`Invalid clock time: ${value}`);
  return Number(match[1]) * 60 + Number(match[2]);
}

export function timetableBounds(
  periods: readonly TeachingPeriod[],
  blocks: readonly { startTime: string; endTime: string }[] = [],
): TimetableBounds {
  const clocks = [
    ...periods.flatMap((period) => [period.startTime, period.endTime]),
    ...blocks.flatMap((block) => [block.startTime, block.endTime]),
  ].flatMap((clock) => {
    try {
      return [clockMinutes(clock)];
    } catch {
      return [];
    }
  });
  return { start: Math.min(...clocks), end: Math.max(...clocks) };
}

export function timetablePosition(
  start: string,
  end: string,
  bounds: TimetableBounds,
) {
  const range = bounds.end - bounds.start;
  const from = clockMinutes(start);
  const to = clockMinutes(end);
  return {
    topPercent: ((from - bounds.start) / range) * 100,
    heightPercent: (Math.max(20, to - from) / range) * 100,
  };
}

/** Grid cells run from one row rule to the next, so a class ending at its
 * period's end leaves the short break below it inside its own cell. A block
 * edge within that break snaps to the cell edge; exact times further from a
 * rule (an exam at 09:30–10:20) keep their place. */
const CELL_SNAP_MINUTES = 15;

export function blockPosition(
  start: string,
  end: string,
  rows: readonly { start: string; end: string }[],
  bounds: TimetableBounds,
) {
  const edges = [
    ...rows.map((row) => clockMinutes(row.start)),
    ...rows.slice(-1).map((row) => clockMinutes(row.end)),
    bounds.end,
  ];
  const from = clockMinutes(start);
  const to = clockMinutes(end);
  const top = Math.max(
    ...edges.filter((edge) => edge <= from && from - edge <= CELL_SNAP_MINUTES),
    -Infinity,
  );
  const bottom = Math.min(
    ...edges.filter((edge) => edge >= to && edge - to <= CELL_SNAP_MINUTES),
    Infinity,
  );
  const clock = (minutes: number) =>
    `${String(Math.floor(minutes / 60)).padStart(2, "0")}:${String(minutes % 60).padStart(2, "0")}`;
  return timetablePosition(
    Number.isFinite(top) ? clock(top) : start,
    Number.isFinite(bottom) && bottom > from ? clock(bottom) : end,
    bounds,
  );
}

/** Percent offset of a clock time inside the rendered day. */
export function timetableOffset(
  minutes: number,
  bounds: TimetableBounds,
): number | undefined {
  if (minutes < bounds.start || minutes > bounds.end) return undefined;
  return ((minutes - bounds.start) / (bounds.end - bounds.start)) * 100;
}
