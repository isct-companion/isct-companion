import { zoomFrom } from "../../components/timetable/course-zoom";
import { CalendarCheck } from "lucide-react";
import { useState } from "react";
import { t } from "../../i18n";
import { UiButton, EmptyState, Skeleton } from "../../components/ui";
import { type PersonalEvent, type Workspace } from "../../lib/model";
import { addCivilDays as addDays, formatJst } from "../../lib/civil-date";
import type {
  AgendaRow,
  AgendaStatus,
  Occurrence,
} from "../../generated/persistence";

const agendaLabel = {
  earlier: "common.earlier",
  now: "common.now",
  next: "today.next",
  later: "common.later",
} as const satisfies Record<AgendaStatus, Parameters<typeof t>[0]>;

const timeLabel = (value: string) => value.replace(/^0/, "");
/** Rows shown before a list folds behind "+N more". */
const CLASS_ROWS = 6;

export function TodayAgenda({
  data,
  rows = [],
  nextClass,
  todayIso,
  scheduleLoading,
  scheduleReveal,
  selection,
  open,
  openExam,
  openEvent,
}: {
  data: Workspace;
  rows?: AgendaRow[];
  nextClass?: Occurrence | null;
  todayIso: string;
  scheduleLoading: boolean;
  scheduleReveal?: string;
  selection?: { courseId: string; assignmentId?: string };
  open: (
    courseId: string,
    tab?: "assignments",
    context?: { date: string; startTime: string; endTime: string },
  ) => void;
  openExam?: (key: string, term: string) => void;
  openEvent: (event: PersonalEvent) => void;
}) {
  const [allClasses, setAllClasses] = useState(false);
  const openBlock = (block: Occurrence, button: HTMLButtonElement) => {
    if (block.kind === "event") {
      const event = data.events.find((item) => item.id === block.id);
      if (event) openEvent(event);
    } else if (block.kind === "exam") {
      // An exam opens on Lecture Desk's exam view; without its term it
      // opens its course, never a lecture under the exam's own id.
      if (block.termId && openExam) openExam(block.key, block.termId);
      else if (block.courseId)
        open(block.courseId, undefined, {
          date: block.date,
          startTime: block.startTime,
          endTime: block.endTime,
        });
    } else {
      if (block.kind === "course") zoomFrom(button);
      open(block.id, undefined, {
        date: block.date,
        startTime: block.startTime,
        endTime: block.endTime,
      });
    }
  };
  const more = (hidden: number, expanded: boolean, toggle: () => void) =>
    hidden > 0 || expanded ? (
      <UiButton className="textButton moreToggle" onClick={toggle}>
        {expanded
          ? t("common.showLess")
          : t("assignments.countMore", { count: hidden })}
      </UiButton>
    ) : null;
  return (
    <section className="todaySection" data-today-section="classes">
      <header>
        <h2>{t("common.schedule", { context: "today" })}</h2>
      </header>
      {scheduleLoading ? (
        <Skeleton count={3} />
      ) : (
        <div
          className={`dayAgenda${scheduleReveal ? ` ${scheduleReveal}` : ""}`}
        >
          {(allClasses ? rows : rows.slice(0, CLASS_ROWS)).map(
            ({ occurrence: block, status, highlighted }) => {
              return (
                <UiButton
                  className={`dayAgendaRow ${block.kind}${highlighted ? " focus" : ""}${selection?.courseId === block.id && !selection.assignmentId ? " selected" : ""}${block.conflict ? " conflict" : ""}`}
                  aria-current={
                    selection?.courseId === block.id && !selection.assignmentId
                      ? "true"
                      : undefined
                  }
                  style={{ "--course": block.color } as React.CSSProperties}
                  onClick={(event) => openBlock(block, event.currentTarget)}
                  key={block.key}
                >
                  <span className="agendaState">
                    {status ? t(agendaLabel[status]) : ""}
                  </span>
                  <time>
                    {timeLabel(block.startTime)}
                    {block.kind === "exam" && (
                      <small>
                        {t(block.quiz ? "common.quiz" : "common.exam")}
                        {block.timeLabel ? ` · ${block.timeLabel}` : ""}
                      </small>
                    )}
                  </time>
                  <span className="agendaLine" aria-hidden="true" />
                  <span className="agendaCourse">
                    <b>{block.title}</b>
                    <small>
                      {[
                        block.room || block.subtitle,
                        block.conflictWith?.length
                          ? t("today.overlapsCourses", {
                              courses: block.conflictWith.join(", "),
                            })
                          : "",
                      ]
                        .filter(Boolean)
                        .join(" · ") || "—"}
                    </small>
                  </span>
                </UiButton>
              );
            },
          )}
          {rows.length > CLASS_ROWS &&
            more(allClasses ? 0 : rows.length - CLASS_ROWS, allClasses, () =>
              setAllClasses((value) => !value),
            )}
          {!rows.length && (
            <EmptyState compact icon={CalendarCheck}>
              {t("today.nothingScheduledToday")}
            </EmptyState>
          )}
          {nextClass && (
            <UiButton
              className={`dayAgendaRow ${nextClass.kind} focus nextClass${selection?.courseId === nextClass.id && !selection.assignmentId ? " selected" : ""}`}
              style={{ "--course": nextClass.color } as React.CSSProperties}
              onClick={(event) => openBlock(nextClass, event.currentTarget)}
            >
              <span className="agendaState">{t("today.next")}</span>
              <time>
                <small>
                  {nextClass.date === addDays(todayIso, 1)
                    ? t("common.tomorrow")
                    : formatJst(nextClass.date, data.settings.locale, {
                        weekday: "short",
                        month: "numeric",
                        day: "numeric",
                      })}
                  {nextClass.kind === "exam" &&
                    ` · ${t(nextClass.quiz ? "common.quiz" : "common.exam")}`}
                </small>
                {timeLabel(nextClass.startTime)}
              </time>
              <span className="agendaLine" aria-hidden="true" />
              <span className="agendaCourse">
                <b>{nextClass.title}</b>
                <small>{nextClass.room || nextClass.subtitle || "—"}</small>
              </span>
            </UiButton>
          )}
        </div>
      )}
    </section>
  );
}
