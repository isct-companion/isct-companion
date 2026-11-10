import { dayNames, type Block, type DayColumn } from "./timetable";
import type { DatedAgenda, PlanningWeek } from "../../platform/core-query";
import { civilWeekday } from "../../lib/civil-date";

/**
 * Adapt Core's dated schedule read model to the timetable's display shape.
 * The column for `today` (the Tokyo civil date) is marked; it carries the now
 * line and the today tint.
 */
export function agendaColumns(
  agendas: readonly DatedAgenda[],
  today: string,
): DayColumn[] {
  return agendas.map((agenda, index) => ({
    label: dayNames[index % dayNames.length],
    date: agenda.date,
    calendarNotice: agenda.calendarNotice,
    noClasses: agenda.noClasses,
    isToday: agenda.date === today,
    blocks: agenda.occurrences as Block[],
  }));
}

/**
 * Adapt Core's recurring planning read model to the timetable display. Pass
 * `today` (a Tokyo civil date) only while the planned quarter is running; its
 * weekday is marked as today.
 */
export function planningColumns(
  view?: PlanningWeek,
  today?: string,
): DayColumn[] {
  const weekday = today ? civilWeekday(today) : undefined;
  return (view?.days ?? []).map((day) => ({
    label: dayNames[day.day - 1] ?? "",
    isToday: day.day === weekday,
    blocks: day.blocks as Block[],
  }));
}
