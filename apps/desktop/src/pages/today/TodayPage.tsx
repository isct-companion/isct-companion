import { ChevronLeft, ChevronRight } from "lucide-react";
import { ExamCards } from "../../components/exams/ExamCards";
import { useEffect, useRef, useState } from "react";
import { t } from "../../i18n";
import { UiButton, useReveal } from "../../components/ui";
import {
  type PersonalEvent,
  type TermId,
  type Workspace,
} from "../../lib/model";
import { CourseFinder } from "./CourseFinder";
import {
  addCivilDays as addDays,
  civilWeekday,
  formatJst,
  tokyoDate,
} from "../../lib/civil-date";
import {
  useScheduleQueries,
  useScheduleQuery,
  type DatedAgenda,
  type TaskView,
} from "../../platform/core-query";
import { useClock } from "../../platform/use-clock";
import type { ExamView, TodayView } from "../../generated/persistence";
import { TodayAssignments } from "./TodayAssignments";
import { TodayAgenda } from "./TodayAgenda";

export function Today({
  data,
  term,
  open,
  openExam,
  openEvent,
  openTask,
  openAssignments,
  hideAssignment,
  openSchedule,
  focusSection,
  compact = false,
  selection,
  active = true,
}: {
  data: Workspace;
  term: TermId;
  open: (
    courseId: string,
    tab?: "assignments",
    context?: { date: string; startTime: string; endTime: string },
  ) => void;
  openExam?: (key: string, term: string) => void;
  openEvent: (event: PersonalEvent) => void;
  openTask: (assignmentId: string) => void;
  /** Lecture Desk's full list of the quarter's assignments. */
  openAssignments?: () => void;
  hideAssignment: (assignmentId: string) => void;
  openSchedule: () => void;
  compact?: boolean;
  active?: boolean;
  selection?: { courseId: string; assignmentId?: string };
  focusSection?: { section: "classes" | "assignments"; request: number };
}) {
  const page = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (focusSection)
      page.current
        ?.querySelector(`[data-today-section="${focusSection.section}"]`)
        ?.scrollIntoView?.({ block: "start" });
  }, [focusSection]);
  const nowIso = useClock(active);
  const now = new Date(nowIso);
  const todayIso = tokyoDate(now);
  // Schedule can show another day of the week; assignments stay relative to now.
  const [pickedDate, setPickedDate] = useState<string | null>(null);
  // -1 / 1 slides the swapped day in from the side it came from.
  const [direction, setDirection] = useState(0);
  const viewIso = pickedDate ?? todayIso;
  const pick = (date: string | null) => {
    const target = date ?? todayIso;
    if (target === viewIso) return;
    setDirection(target > viewIso ? 1 : -1);
    setPickedDate(date);
  };
  const swap = { "--swap": direction } as React.CSSProperties;
  const isToday = viewIso === todayIso;
  const viewDate = new Date(`${viewIso}T12:00:00+09:00`);
  const weekStart = addDays(viewIso, 1 - civilWeekday(viewIso));
  // Apart from the rest: another week's days must not draw under this one,
  // while the answers below stay on screen when only the minute changes.
  const weekQuery = useScheduleQuery<DatedAgenda[]>(
    { kind: "calendarRange", from: weekStart, to: addDays(viewIso, 7) },
    active,
  );
  const dashboard = useScheduleQueries<[ExamView[], TodayView, TaskView]>(
    [
      { kind: "upcomingExams", from: viewIso, to: null },
      { kind: "today", now: nowIso },
      { kind: "taskView", now: nowIso },
    ],
    active,
  );
  const agendas = weekQuery.value;
  const [exams, today, taskView] = dashboard.value ?? [];
  const scheduleLoading = weekQuery.pending && !agendas;
  const scheduleReveal = useReveal(scheduleLoading);
  const agenda = agendas?.find((day) => day.date === viewIso);
  const teachingState = agenda?.calendarState;
  const effectiveDay = civilWeekday(viewIso);
  const importantDates = agenda?.importantDates ?? [];
  const week = Array.from({ length: 7 }, (_, index) => {
    const date = addDays(weekStart, index);
    return {
      date,
      busy: Boolean(
        agendas?.find((day) => day.date === date)?.occurrences.length,
      ),
    };
  });
  const tasksLoading = dashboard.pending && !taskView;
  const tasksReveal = useReveal(tasksLoading);
  const stateLabel =
    teachingState?.label ||
    t(effectiveDay > 5 ? "today.weekend" : "today.teachingDay");

  return (
    <div
      className={`page todayPage${compact ? " todayNavigation" : ""}`}
      ref={page}
    >
      {compact && (
        <CourseFinder data={data} term={term} open={(id) => open(id)} />
      )}
      <header className="todayHeader">
        <div className="daySwap" key={viewIso} style={swap}>
          <p className="todayEyebrow">
            {formatJst(viewDate, data.settings.locale, {
              weekday: "long",
            })}
          </p>
          <h1>
            {formatJst(viewDate, data.settings.locale, {
              day: "numeric",
              month: "long",
            })}
          </h1>
        </div>
        {!compact && (
          <div className="todayWeek">
            <div
              className="weekStrip"
              role="group"
              aria-label={t("today.week")}
              style={
                {
                  "--day": effectiveDay - 1,
                  "--swap": direction,
                } as React.CSSProperties
              }
            >
              <UiButton
                className="weekShift"
                aria-label={t("today.previousWeek")}
                onClick={() => pick(addDays(viewIso, -7))}
              >
                <ChevronLeft size={14} strokeWidth={1.75} aria-hidden />
              </UiButton>
              {week.map(({ date, busy }) => (
                <UiButton
                  key={date}
                  className={`weekDay${date === viewIso ? " active" : ""}${date === todayIso ? " today" : ""}${busy ? " busy" : ""}`}
                  aria-pressed={date === viewIso}
                  aria-label={date}
                  onClick={() => pick(date === todayIso ? null : date)}
                >
                  <small>
                    {formatJst(date, data.settings.locale, {
                      weekday: "narrow",
                    })}
                  </small>
                  <b>{Number(date.slice(8))}</b>
                </UiButton>
              ))}
              <UiButton
                className="weekShift"
                aria-label={t("today.nextWeek")}
                onClick={() => pick(addDays(viewIso, 7))}
              >
                <ChevronRight size={14} strokeWidth={1.75} aria-hidden />
              </UiButton>
            </div>
            <span className="teachingState">
              {!isToday && (
                <UiButton className="textButton" onClick={() => pick(null)}>
                  {t("common.today")}
                </UiButton>
              )}
              {stateLabel}
            </span>
          </div>
        )}
      </header>

      <div className="todayFlow">
        {importantDates.length > 0 && (
          <aside
            className="importantDates"
            aria-label={t("today.importantDates")}
          >
            {importantDates.map((item) => (
              <span key={item.id}>
                <b>{item.label || t("today.importantDate")}</b>
                {item.startDate !== item.endDate && (
                  <small>
                    {item.startDate}–{item.endDate}
                  </small>
                )}
              </span>
            ))}
          </aside>
        )}
        {Boolean(exams?.length) && (
          <section
            className="todaySection"
            aria-label={t("today.upcomingExams")}
          >
            <header>
              <h2>{t("today.upcomingExams")}</h2>
            </header>
            <ExamCards
              exams={exams?.slice(0, 3) ?? []}
              courses={data.courses}
              open={(id) => open(id)}
              openExam={openExam}
            />
          </section>
        )}
        <TodayAgenda
          data={data}
          agendas={agendas}
          agenda={agenda}
          today={today}
          now={now}
          viewIso={viewIso}
          isToday={isToday}
          swap={swap}
          scheduleLoading={scheduleLoading}
          scheduleReveal={scheduleReveal}
          selection={selection}
          open={open}
          openExam={openExam}
          openEvent={openEvent}
          openSchedule={openSchedule}
        />
        <TodayAssignments
          data={data}
          term={term}
          taskView={taskView}
          tasksLoading={tasksLoading}
          tasksReveal={tasksReveal}
          selection={selection}
          openTask={openTask}
          hideAssignment={hideAssignment}
          openAssignments={openAssignments}
        />
      </div>
    </div>
  );
}
