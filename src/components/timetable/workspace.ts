import { dayNames, type Block, type DayColumn } from "./timetable";
import type { DatedAgenda, PlanningWeek } from "../../platform/core-query";

/** Adapt Core's dated schedule read model to the timetable's display shape. */
export function agendaColumns(agendas: readonly DatedAgenda[]): DayColumn[] {
  return agendas.map((agenda, index) => ({
    label: dayNames[index % dayNames.length],
    date: agenda.date,
    calendarNotice: agenda.calendarNotice,
    noClasses: agenda.noClasses,
    blocks: agenda.occurrences as Block[],
  }));
}

/** Adapt Core's recurring planning read model to the timetable display. */
export function planningColumns(view?: PlanningWeek): DayColumn[] {
  return (view?.days ?? []).map((day) => ({
    label: dayNames[day.day - 1] ?? "",
    blocks: day.blocks as Block[],
  }));
}
