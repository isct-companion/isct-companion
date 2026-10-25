import { useState } from "react";
import { Paper, Input } from "../../components/ui";
import { UiButton } from "../../components/ui";
import {
  WeekTimetable,
  type Block,
  type BlockAction,
} from "../../components/timetable/timetable";
import { agendaColumns } from "../../components/timetable/workspace";
import { useScheduleQuery, type ScheduleView } from "../../platform/core-query";
import { useClock } from "../../platform/use-clock";
import {
  addCivilDays,
  civilWeekday,
  formatCivilDate,
  tokyoDate,
} from "../../lib/civil-date";
import type { Workspace } from "../../lib/model";
import { t } from "../../i18n";

/** Date navigation is independent of the quarter used for course planning. */
export function CalendarView({
  data,
  open,
  mark,
}: {
  data: Workspace;
  open: (block: Block) => void;
  /** Marks one session; absent when the workspace is read-only. */
  mark?: (block: Block, as: "exam" | "quiz" | "cancelled" | "class") => void;
}) {
  // Classes and exams take a mark; personal items open their own editor,
  // exactly as a click does. The marks are exclusive: choosing the checked
  // one again makes the session a plain class, and Core keeps no mark when
  // its sources already say so.
  const actionsFor = (block: Block): BlockAction[] => {
    if (!mark) return [];
    if (block.kind === "event" || block.kind === "plan")
      return [{ label: t("Edit"), run: () => open(block) }];
    const state = block.cancelled
      ? "cancelled"
      : block.kind === "exam"
        ? block.quiz
          ? "quiz"
          : "exam"
        : "class";
    return (
      [
        ["exam", t("Mark as exam")],
        ["quiz", t("Mark as quiz")],
        ["cancelled", t("Mark as cancelled")],
      ] as const
    ).map(([as, label]) => ({
      label,
      current: state === as,
      run: () => mark(block, state === as ? "class" : as),
    }));
  };
  // The today column and the now line come from one clock instant.
  const now = useClock();
  const today = tokyoDate(new Date(now));
  const [anchor, setAnchor] = useState<string>();
  const date = anchor ?? today;
  const from = addCivilDays(date, 1 - civilWeekday(date));
  const to = addCivilDays(from, 6);
  const view = useScheduleQuery<ScheduleView>({
    kind: "scheduleView",
    term: data.currentTerm,
    from,
    to,
    includePersonal: true,
  });
  const visibleFrom = view.value?.agendas[0]?.date ?? from;
  const visibleTo = view.value?.agendas.at(-1)?.date ?? to;
  return (
    <section className="calendarWorkspace" aria-label={t("Calendar")}>
      <header className="scheduleTitleRow">
        <h1>{t("Calendar")}</h1>
      </header>
      <Paper className="ttCard" withBorder radius="md">
        <header className="timetableViewBar">
          <b>
            {formatCivilDate(visibleFrom, data.settings.locale, {
              month: "short",
              day: "numeric",
            })}{" "}
            –{" "}
            {formatCivilDate(visibleTo, data.settings.locale, {
              month: "short",
              day: "numeric",
            })}
          </b>
          <div className="scheduleControls">
            <Input
              type="date"
              aria-label={t("Date")}
              value={date}
              onChange={(event) =>
                event.target.value && setAnchor(event.target.value)
              }
            />
            <UiButton
              aria-label={t("Previous week")}
              onClick={() => setAnchor(addCivilDays(date, -7))}
            >
              ←
            </UiButton>
            <UiButton onClick={() => setAnchor(undefined)}>
              {t("This week")}
            </UiButton>
            <UiButton
              aria-label={t("Next week")}
              onClick={() => setAnchor(addCivilDays(date, 7))}
            >
              →
            </UiButton>
          </div>
        </header>
        <WeekTimetable
          days={agendaColumns(view.value?.agendas ?? [], today)}
          onSelect={open}
          actionsFor={actionsFor}
          now={now}
        />
      </Paper>
    </section>
  );
}
