import { ChevronLeft, ChevronRight } from "lucide-react";
import { TitleRow } from "../../components/TitleRow";
import { useState } from "react";
import { Paper, Input, Skeleton } from "../../components/ui";
import { UiButton } from "../../components/ui";
import {
  WeekTimetable,
  type Block,
  type BlockAction,
} from "../../components/timetable/timetable";
import { agendaColumns } from "../../components/timetable/workspace";
import { useScheduleQuery, type DatedAgenda } from "../../platform/core-query";
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
  // Personal items open their own editor, exactly as a click does; a session
  // Core gives a mark takes one. The marks are exclusive: choosing the checked
  // one again makes the session a plain class, and Core keeps no mark when
  // its sources already say so.
  const actionsFor = (block: Block): BlockAction[] => {
    if (!mark) return [];
    if (block.kind === "event")
      return [{ label: t("common.edit"), run: () => open(block) }];
    const state = block.mark;
    if (!state) return [];
    return (
      [
        ["exam", t("common.markAsExam")],
        ["quiz", t("common.markAsQuiz")],
        ["cancelled", t("common.markAsCancelled")],
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
  // Which way the last step went: the grid slides in from that side.
  const [slide, setSlide] = useState(0);
  const step = (next: string | undefined, direction: number) => {
    setSlide(direction);
    setAnchor(next);
  };
  const date = anchor ?? today;
  const from = addCivilDays(date, 1 - civilWeekday(date));
  const to = addCivilDays(from, 6);
  const view = useScheduleQuery<DatedAgenda[]>({
    kind: "calendarRange",
    from,
    to,
  });
  const visibleFrom = view.value?.[0]?.date ?? from;
  const visibleTo = view.value?.at(-1)?.date ?? to;
  return (
    <section className="calendarWorkspace" aria-label={t("today.calendar")}>
      <TitleRow className="calendarTitle">
        <h1>{t("today.calendar")}</h1>
      </TitleRow>
      {/* Stepping through weeks is the calendar's own tool: it sits on the
          calendar, under the title row. */}
      <div className="calendarBar paneToolbar">
        <div className="toolPill">
          <UiButton
            aria-label={t("today.previousWeek")}
            onClick={() => step(addCivilDays(date, -7), -1)}
          >
            <ChevronLeft size={14} strokeWidth={1.75} aria-hidden />
          </UiButton>
          <span className="calendarRange toolPillLabel">
            {formatCivilDate(visibleFrom, data.settings.locale, {
              month: "short",
              day: "numeric",
            })}{" "}
            –{" "}
            {formatCivilDate(visibleTo, data.settings.locale, {
              month: "short",
              day: "numeric",
            })}
          </span>
          <UiButton
            aria-label={t("today.nextWeek")}
            onClick={() => step(addCivilDays(date, 7), 1)}
          >
            <ChevronRight size={14} strokeWidth={1.75} aria-hidden />
          </UiButton>
        </div>
        <UiButton
          className="toolPill toolPillButton"
          onClick={() => step(undefined, Math.sign(today.localeCompare(date)))}
        >
          {t("common.thisWeek")}
        </UiButton>
        <Input
          type="date"
          aria-label={t("common.date")}
          value={date}
          onChange={(event) =>
            event.target.value &&
            step(
              event.target.value,
              Math.sign(event.target.value.localeCompare(date)),
            )
          }
        />
      </div>
      <Paper className="ttCard">
        {view.pending && !view.value ? (
          <Skeleton shape="week" />
        ) : (
          <div
            key={from}
            className={`weekPane${slide ? " weekSlide" : ""}`}
            style={{ "--slide": slide } as React.CSSProperties}
          >
            <WeekTimetable
              days={agendaColumns(view.value ?? [], today)}
              onSelect={open}
              actionsFor={actionsFor}
              now={now}
              zoom
            />
          </div>
        )}
      </Paper>
    </section>
  );
}
