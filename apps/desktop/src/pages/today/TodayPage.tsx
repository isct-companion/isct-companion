import { TitleRow } from "../../components/TitleRow";
import { ExamCards } from "../../components/exams/ExamCards";
import { useEffect, useRef, type ReactNode } from "react";
import { t } from "../../i18n";
import { useReveal } from "../../components/ui";
import {
  type PersonalEvent,
  type TermId,
  type Workspace,
} from "../../lib/model";
import { civilWeekday, formatJst, tokyoDate } from "../../lib/civil-date";
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
  focusSection,
  selection,
  sidebar,
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
  active?: boolean;
  selection?: { courseId: string; assignmentId?: string };
  focusSection?: { section: "classes" | "assignments"; request: number };
  /** The button that folds Today away beside the calendar. */
  sidebar?: ReactNode;
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
  const viewDate = new Date(`${todayIso}T12:00:00+09:00`);
  // Apart from the rest: the day's classes stay on screen while the answers
  // below re-read each minute.
  const dayQuery = useScheduleQuery<DatedAgenda[]>(
    { kind: "calendarRange", from: todayIso, to: todayIso },
    active,
  );
  const dashboard = useScheduleQueries<[ExamView[], TodayView, TaskView]>(
    [
      { kind: "upcomingExams", from: todayIso, to: null },
      { kind: "today", now: nowIso },
      { kind: "taskView", now: nowIso },
    ],
    active,
  );
  const [exams, today, taskView] = dashboard.value ?? [];
  const scheduleLoading = dayQuery.pending && !dayQuery.value;
  const scheduleReveal = useReveal(scheduleLoading);
  const agenda = dayQuery.value?.find((day) => day.date === todayIso);
  const teachingState = agenda?.calendarState;
  const effectiveDay = civilWeekday(todayIso);
  const importantDates = agenda?.importantDates ?? [];
  const tasksLoading = dashboard.pending && !taskView;
  const tasksReveal = useReveal(tasksLoading);
  const stateLabel =
    teachingState?.label ||
    t(effectiveDay > 5 ? "today.weekend" : "today.teachingDay");

  return (
    <div className="page todayPage" ref={page}>
      <TitleRow nav sidebar={sidebar} className="todayHeader">
        <h1>
          {formatJst(viewDate, data.settings.locale, {
            day: "numeric",
            month: "long",
          })}
          <span className="todayEyebrow">
            {formatJst(viewDate, data.settings.locale, {
              weekday: "long",
            })}
          </span>
        </h1>
        <span className="titleRowSpacer" />
        <span className="teachingState">{stateLabel}</span>
      </TitleRow>

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
              open={(id) => open(id)}
              openExam={openExam}
            />
          </section>
        )}
        <TodayAgenda
          data={data}
          agenda={agenda}
          today={today?.date === todayIso ? today : undefined}
          now={now}
          todayIso={todayIso}
          scheduleLoading={scheduleLoading}
          scheduleReveal={scheduleReveal}
          selection={selection}
          open={open}
          openExam={openExam}
          openEvent={openEvent}
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
