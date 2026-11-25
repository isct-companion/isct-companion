import { zoomFrom } from "../../components/timetable/course-zoom";
import { CalendarCheck } from "lucide-react";
import { useState } from "react";
import { t } from "../../i18n";
import { UiButton, EmptyState, Skeleton } from "../../components/ui";
import { type PersonalEvent, type Workspace } from "../../lib/model";
import {
  addCivilDays as addDays,
  formatJst,
  jstClock,
} from "../../lib/civil-date";
import {
  type DatedAgenda,
  type ScheduleOccurrence,
} from "../../platform/core-query";
import type { Occurrence, TodayView } from "../../generated/persistence";

const timeLabel = (value: string) => value.replace(/^0/, "");
/** Rows shown before a list folds behind "+N more". */
const CLASS_ROWS = 6;

export function TodayAgenda({
  data,
  agenda,
  today,
  now,
  todayIso,
  scheduleLoading,
  scheduleReveal,
  selection,
  open,
  openExam,
  openEvent,
}: {
  data: Workspace;
  agenda?: DatedAgenda;
  today?: TodayView;
  now: Date;
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
  const blocks = agenda?.occurrences ?? [];
  const clock = jstClock(now);
  const currentKey = today?.hero?.occurrence.key;
  const nextClass = today?.upcoming[0]?.occurrence;
  const nextKey = nextClass?.key;
  const focusKey = currentKey ?? nextKey;
  const openBlock = (
    block: Occurrence | ScheduleOccurrence,
    button: HTMLButtonElement,
  ) => {
    if (block.kind === "event") {
      const event = data.events.find((item) => item.id === block.id);
      if (event) openEvent(event);
    } else if (block.kind === "exam" && block.termId && openExam) {
      openExam(block.key, block.termId);
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
          {(allClasses ? blocks : blocks.slice(0, CLASS_ROWS)).map((block) => {
            const state = block.cancelled
              ? t("common.noClass")
              : t(
                  block.key === currentKey
                    ? "common.now"
                    : block.key === nextKey
                      ? "today.next"
                      : block.endTime <= clock
                        ? "common.earlier"
                        : // Core's moment covers classes; a personal
                          // event is now while its own time runs.
                          block.kind === "event" && block.startTime <= clock
                          ? "common.now"
                          : "common.later",
                );
            return (
              <UiButton
                className={`dayAgendaRow ${block.kind}${block.key === focusKey ? " focus" : ""}${selection?.courseId === block.id && !selection.assignmentId ? " selected" : ""}${block.conflict ? " conflict" : ""}`}
                aria-current={
                  selection?.courseId === block.id && !selection.assignmentId
                    ? "true"
                    : undefined
                }
                style={{ "--course": block.color } as React.CSSProperties}
                onClick={(event) => openBlock(block, event.currentTarget)}
                key={block.key}
              >
                <span className="agendaState">{state}</span>
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
          })}
          {blocks.length > CLASS_ROWS &&
            more(allClasses ? 0 : blocks.length - CLASS_ROWS, allClasses, () =>
              setAllClasses((value) => !value),
            )}
          {!blocks.length && (
            <EmptyState compact icon={CalendarCheck}>
              {t("today.nothingScheduledToday")}
            </EmptyState>
          )}
          {nextClass && nextClass.date > todayIso && (
            <UiButton
              className={`dayAgendaRow focus nextClass${selection?.courseId === nextClass.id && !selection.assignmentId ? " selected" : ""}`}
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
