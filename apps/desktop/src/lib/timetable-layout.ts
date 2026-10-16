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

/** Percent offset of a clock time inside the rendered day. */
export function timetableOffset(
  minutes: number,
  bounds: TimetableBounds,
): number | undefined {
  if (minutes < bounds.start || minutes > bounds.end) return undefined;
  return ((minutes - bounds.start) / (bounds.end - bounds.start)) * 100;
}
