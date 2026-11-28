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
import { formatJst, tokyoDate } from "../../lib/civil-date";
import { wallNow } from "../../platform/use-clock";
import { AnnouncementBanners } from "../../components/notifications/AnnouncementBanners";
import type {
  AgendaDayKind,
  AnnouncementView,
} from "../../generated/persistence";
import type { ImportantDateView } from "../../platform/core-query";
import { useTodayAgenda } from "./use-today-agenda";
import { TodayAssignments } from "./TodayAssignments";
import { TodayAgenda } from "./TodayAgenda";

const teachingLabel = {
  teachingDay: "today.teachingDay",
  weekend: "today.weekend",
  examPeriod: "today.examPeriod",
  noClasses: "today.noClasses",
} as const satisfies Record<AgendaDayKind, Parameters<typeof t>[0]>;

export function Today({
  data,
  term,
  announcements,
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
  announcements?: readonly AnnouncementView[];
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
  const dashboard = useTodayAgenda(null, active);
  const todayIso = tokyoDate(new Date(wallNow()));
  const agenda =
    dashboard.value?.today === todayIso ? dashboard.value : undefined;
  const viewDate = new Date(`${todayIso}T12:00:00+09:00`);
  const importantDates = (agenda?.importantDates ?? []) as ImportantDateView[];
  const scheduleLoading = dashboard.pending && !agenda;
  const reveal = useReveal(scheduleLoading);
  const tasks = agenda?.tasks;
  const tasksLoading = dashboard.pending && !tasks;
  const tasksReveal = useReveal(tasksLoading);
  const stateLabel =
    agenda?.calendarNotice || (agenda ? t(teachingLabel[agenda.dayKind]) : "");

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
        <AnnouncementBanners items={announcements} />
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
        {agenda && agenda.exams.length > 0 && (
          <section
            className="todaySection"
            aria-label={t("today.upcomingExams")}
          >
            <header>
              <h2>{t("today.upcomingExams")}</h2>
            </header>
            <ExamCards
              exams={agenda.exams}
              open={(id) => open(id)}
              openExam={openExam}
            />
          </section>
        )}
        <TodayAgenda
          data={data}
          rows={agenda?.rows}
          nextClass={agenda?.nextClass}
          todayIso={todayIso}
          scheduleLoading={scheduleLoading}
          scheduleReveal={reveal}
          selection={selection}
          open={open}
          openExam={openExam}
          openEvent={openEvent}
        />
        <TodayAssignments
          data={data}
          term={term}
          taskView={tasks}
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
