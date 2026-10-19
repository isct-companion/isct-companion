import { useEffect, useState } from "react";
import { Paper, Input } from "@mantine/core";
import { UiButton } from "../../components/mantine";
import {
  WeekTimetable,
  type Block,
  type BlockAction,
} from "../../components/timetable/timetable";
import { agendaColumns } from "../../components/timetable/workspace";
import { useScheduleQuery, type ScheduleView } from "../../platform/core-query";
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
  mark?: (block: Block, as: "exam" | "cancelled" | "class") => void;
}) {
  // Classes and exams take a mark; personal items open their own editor,
  // exactly as a click does.
  const actionsFor = (block: Block): BlockAction[] => {
    if (!mark) return [];
    if (block.kind === "event" || block.kind === "plan")
      return [{ label: t("Edit"), run: () => open(block) }];
    return [
      {
        label: t("Mark as exam"),
        current: block.kind === "exam" && !block.cancelled,
        run: () => mark(block, "exam"),
      },
      {
        label: t("Mark as cancelled"),
        current: Boolean(block.cancelled),
        run: () => mark(block, "cancelled"),
      },
      {
        label: t("Mark as a normal class"),
        current: block.kind === "course" && !block.cancelled,
        run: () => mark(block, "class"),
      },
    ];
  };
  // Re-render each minute so the now line moves and today follows midnight.
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), 60_000);
    return () => window.clearInterval(timer);
  }, []);
  const today = tokyoDate(now);
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
          showNow
          emptyLabel="Nothing scheduled this day"
        />
      </Paper>
    </section>
  );
}
