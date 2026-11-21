import { dayShort } from "../../components/timetable/timetable";
import { zoomFrom } from "../../components/timetable/course-zoom";
import { CalendarCheck } from "lucide-react";
import { useState } from "react";
import { t } from "../../i18n";
import { UiButton, EmptyState, Skeleton } from "../../components/ui";
import { type PersonalEvent, type Workspace } from "../../lib/model";
import {
  addCivilDays as addDays,
  civilWeekday,
  jstClock,
} from "../../lib/civil-date";
import { type DatedAgenda } from "../../platform/core-query";
import type { TodayView } from "../../generated/persistence";

const timeLabel = (value: string) => value.replace(/^0/, "");
/** Rows shown before a list folds behind "+N more". */
const CLASS_ROWS = 6;

export function TodayAgenda({
  data,
  agendas,
  agenda,
  today,
  now,
  viewIso,
  isToday,
  swap,
  scheduleLoading,
  scheduleReveal,
  selection,
  open,
  openExam,
  openEvent,
  openSchedule,
}: {
  data: Workspace;
  agendas?: DatedAgenda[];
  agenda?: DatedAgenda;
  today?: TodayView;
  now: Date;
  viewIso: string;
  isToday: boolean;
  swap: React.CSSProperties;
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
  openSchedule: () => void;
}) {
  const [allClasses, setAllClasses] = useState(false);
  const blocks = agenda?.occurrences ?? [];
  const clock = jstClock(now);
  const currentKey = isToday ? today?.hero?.occurrence.key : undefined;
  const nextKey = isToday ? today?.upcoming[0]?.occurrence.key : undefined;
  const activeIndex = blocks.findIndex((block) => block.key === currentKey);
  const nextIndex = blocks.findIndex((block) => block.key === nextKey);
  const focusIndex = activeIndex >= 0 ? activeIndex : nextIndex;
  // With no agenda today, show the next actual occurrence within a week.
  const nextClass = (() => {
    if (blocks.length) return undefined;
    for (const dayAgenda of agendas ?? []) {
      const date = dayAgenda.date;
      if (date <= viewIso || date > addDays(viewIso, 7)) continue;
      const offset = Math.round(
        (Date.parse(`${date}T12:00:00Z`) - Date.parse(`${viewIso}T12:00:00Z`)) /
          86_400_000,
      );
      const day = civilWeekday(date);
      const first = dayAgenda.occurrences[0];
      if (first)
        return {
          ...first,
          date: date,
          day: t(
            offset === 1 && isToday ? "common.tomorrow" : dayShort[day - 1],
          ),
        };
    }
    return undefined;
  })();
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
        <UiButton className="textButton" onClick={openSchedule}>
          {t("today.week")}
        </UiButton>
      </header>
      {scheduleLoading ? (
        <Skeleton count={3} />
      ) : (
        <div
          className={`dayAgenda daySwap${scheduleReveal ? ` ${scheduleReveal}` : ""}`}
          key={viewIso}
          style={swap}
        >
          {(allClasses ? blocks : blocks.slice(0, CLASS_ROWS)).map(
            (block, index) => {
              const state = !isToday
                ? ""
                : t(
                    index === activeIndex
                      ? "common.now"
                      : index === nextIndex
                        ? "today.next"
                        : block.endTime <= clock
                          ? "common.earlier"
                          : // Core's moment covers classes; a personal
                            // event is now while its own time runs.
                            block.kind === "event" &&
                              block.startTime <= clock
                            ? "common.now"
                            : "common.later",
                  );
              return (
                <UiButton
                  className={`dayAgendaRow ${block.kind}${index === focusIndex ? " focus" : ""}${selection?.courseId === block.id && !selection.assignmentId ? " selected" : ""}${block.conflict ? " conflict" : ""}`}
                  aria-current={
                    selection?.courseId === block.id &&
                    !selection.assignmentId
                      ? "true"
                      : undefined
                  }
                  style={{ "--course": block.color } as React.CSSProperties}
                  onClick={(event) => {
                    if (block.kind === "exam" && block.termId && openExam)
                      openExam(block.key, block.termId);
                    else if (
                      block.kind === "course" ||
                      block.kind === "exam"
                    ) {
                      if (block.kind === "course")
                        zoomFrom(event.currentTarget);
                      open(block.id, undefined, {
                        date: viewIso,
                        startTime: block.startTime,
                        endTime: block.endTime,
                      });
                    } else {
                      const event = data.events.find(
                        (item) => item.id === block.id,
                      );
                      if (event) openEvent(event);
                    }
                  }}
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
            },
          )}
          {blocks.length > CLASS_ROWS &&
            more(
              allClasses ? 0 : blocks.length - CLASS_ROWS,
              allClasses,
              () => setAllClasses((value) => !value),
            )}
          {!blocks.length && (
            <>
              <EmptyState compact icon={CalendarCheck}>
                {t(
                  isToday
                    ? "today.nothingScheduledToday"
                    : "today.nothingScheduled",
                )}
              </EmptyState>
              {nextClass && (
                <UiButton
                  className={`dayAgendaRow focus nextClass${selection?.courseId === nextClass.id && !selection.assignmentId ? " selected" : ""}`}
                  style={
                    { "--course": nextClass.color } as React.CSSProperties
                  }
                  onClick={(event) => {
                    if (nextClass.kind === "event") {
                      const event = data.events.find(
                        (item) => item.id === nextClass.id,
                      );
                      if (event) openEvent(event);
                    } else {
                      zoomFrom(event.currentTarget);
                      open(nextClass.id, undefined, {
                        date: nextClass.date,
                        startTime: nextClass.startTime,
                        endTime: nextClass.endTime,
                      });
                    }
                  }}
                >
                  <span className="agendaState">{t("today.next")}</span>
                  <time>
                    <small>{nextClass.day}</small>
                    {timeLabel(nextClass.startTime)}
                  </time>
                  <span className="agendaLine" aria-hidden="true" />
                  <span className="agendaCourse">
                    <b>{nextClass.title}</b>
                    <small>
                      {nextClass.room || nextClass.subtitle || "—"}
                    </small>
                  </span>
                </UiButton>
              )}
            </>
          )}
        </div>
      )}
    </section>
  );
}
