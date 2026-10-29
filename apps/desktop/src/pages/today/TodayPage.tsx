import { dayShort } from "../../components/timetable/timetable";
import { zoomFrom } from "../../app/course-zoom";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { CalendarCheck } from "lucide-react";
import { ExamCards } from "../../components/exams/ExamCards";
import { DueChip } from "../../components/DueChip";
import { submissionLabel } from "../../lib/academic";
import { useEffect, useRef, useState } from "react";
import { t } from "../../i18n";
import { UiButton, EmptyState, Skeleton, useReveal } from "../../components/ui";
import {
  contentLanguage,
  pickLocalized,
  type Assignment,
  type PersonalEvent,
  type TermId,
  type Workspace,
} from "../../lib/model";
import { CourseFinder } from "./CourseFinder";
import { AssignmentGroupList } from "../../components/assignments/AssignmentGroupList";
import {
  addCivilDays as addDays,
  civilWeekday,
  formatJst,
  jstClock,
  tokyoDate,
} from "../../lib/civil-date";
import {
  useScheduleQuery,
  type ScheduleView,
  type TaskView,
} from "../../platform/core-query";
import { useClock } from "../../platform/use-clock";
import { withRecords } from "../../platform/records";

const timeLabel = (value: string) => value.replace(/^0/, "");
/** Rows shown before a list folds behind "+N more". */
const CLASS_ROWS = 6;
const DUE_ROWS = 8;
export function TodayBody({
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
  const [allClasses, setAllClasses] = useState(false);
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
  const scheduleQuery = useScheduleQuery<ScheduleView>({
    kind: "scheduleView",
    term,
    from: weekStart,
    to: addDays(viewIso, 7),
    includePersonal: true,
  });
  const schedule = scheduleQuery.value;
  // Nothing to draw yet (no cached read): placeholders instead of "nothing".
  const scheduleLoading = scheduleQuery.pending && !schedule;
  const scheduleReveal = useReveal(scheduleLoading);
  const agenda = schedule?.agendas.find((day) => day.date === viewIso);
  const teachingState = agenda?.calendarState;
  const effectiveDay = civilWeekday(viewIso);
  const blocks = agenda?.occurrences ?? [];
  const importantDates = agenda?.importantDates ?? [];
  const week = Array.from({ length: 7 }, (_, index) => {
    const date = addDays(weekStart, index);
    return {
      date,
      busy: Boolean(
        schedule?.agendas.find((day) => day.date === date)?.occurrences.length,
      ),
    };
  });
  const clock = jstClock(now);
  const activeIndex = isToday
    ? blocks.findIndex(
        (block) => block.startTime <= clock && block.endTime > clock,
      )
    : -1;
  const nextIndex = isToday
    ? blocks.findIndex((block) => block.startTime > clock)
    : -1;
  const focusIndex = activeIndex >= 0 ? activeIndex : nextIndex;

  const taskQuery = useScheduleQuery<TaskView>({
    kind: "taskView",
    now: nowIso,
  });
  const taskView = taskQuery.value;
  const tasksLoading = taskQuery.pending && !taskView;
  const tasksReveal = useReveal(tasksLoading);
  const taskTerm = taskView?.byTerm[term];
  const submittedIds = new Set(taskView?.submittedIds);
  const byAssignment = new Map(data.assignments.map((item) => [item.id, item]));
  const pendingCount = taskTerm?.pendingIds.length ?? 0;
  const row = (assignment: Assignment) => {
    const course = data.courses.find(
      (item) => item.id === assignment.courseId && !item.deletedAt,
    );
    const due = taskView?.deadlines?.[assignment.id];
    const submitted = submittedIds.has(assignment.id);
    // Past and submitted: gray; past and never submitted: red; submitted
    // ahead of the deadline: green.
    const state = due?.overdue
      ? submitted
        ? " closed"
        : " missed"
      : submitted
        ? " done"
        : "";
    return (
      <div className={`dueRow${state}`} key={assignment.id}>
        <i style={{ background: course?.color ?? "#888" }} />
        <UiButton
          className={`dueAssignment${selection?.assignmentId === assignment.id ? " selected" : ""}`}
          aria-current={
            selection?.assignmentId === assignment.id ? "true" : undefined
          }
          onClick={() => openTask(assignment.id)}
        >
          <span>
            <b>{assignment.title}</b>
            <small>
              {course?.title ?? "LMS"} ·{" "}
              {submissionLabel(assignment, t, submitted)}
            </small>
          </span>
          {due && (
            <DueChip
              due={due}
              submitted={submitted}
              locale={data.settings.locale}
            />
          )}
        </UiButton>
        <UiButton
          className="textButton"
          onClick={() => hideAssignment(assignment.id)}
        >
          {t("common.hide")}
        </UiButton>
      </div>
    );
  };
  // With no agenda today, show the next actual occurrence within a week.
  const nextClass = (() => {
    if (blocks.length) return undefined;
    for (const dayAgenda of schedule?.agendas ?? []) {
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
  const rawStateLabel = teachingState?.label;
  const stateLabel =
    typeof rawStateLabel === "string"
      ? rawStateLabel
      : rawStateLabel
        ? (rawStateLabel[contentLanguage(data.settings)] ??
          rawStateLabel.ja ??
          rawStateLabel.en)
        : t(effectiveDay > 5 ? "today.weekend" : "today.teachingDay");

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
                <b>
                  {pickLocalized(
                    item.label,
                    contentLanguage(data.settings),
                    t("today.importantDate"),
                  )}
                </b>
                {item.startDate !== item.endDate && (
                  <small>
                    {item.startDate}–{item.endDate}
                  </small>
                )}
              </span>
            ))}
          </aside>
        )}
        {(schedule?.upcomingExams ?? []).some(
          (exam) =>
            exam.courseId &&
            exam.status === "scheduled" &&
            exam.date >= viewIso,
        ) && (
          <section
            className="todaySection"
            aria-label={t("today.upcomingExams")}
          >
            <header>
              <h2>{t("today.upcomingExams")}</h2>
            </header>
            <ExamCards
              exams={(schedule?.upcomingExams ?? [])
                .filter(
                  (exam) =>
                    exam.courseId &&
                    exam.status === "scheduled" &&
                    exam.date >= viewIso,
                )
                .slice(0, 3)}
              courses={schedule?.courses ?? []}
              open={(id) => open(id)}
              openExam={openExam}
            />
          </section>
        )}
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

        <section
          className="todaySection dueSection"
          data-today-section="assignments"
        >
          <header>
            <h2>{t("common.assignments")}</h2>
            <span>
              {pendingCount
                ? t("today.countActive", { count: pendingCount })
                : t("today.allClear")}
            </span>
            {openAssignments && (
              <UiButton className="textButton" onClick={openAssignments}>
                {t("common.allAssignments")} →
              </UiButton>
            )}
          </header>
          {tasksLoading ? (
            <Skeleton count={3} />
          ) : (
            <div className={`dueList${tasksReveal ? ` ${tasksReveal}` : ""}`}>
              <AssignmentGroupList
                groups={taskTerm?.today}
                assignments={byAssignment}
                row={row}
                thisWeekLimit={DUE_ROWS}
                pastLabel={t("today.missed")}
                empty={t("today.nothingDueThisWeek")}
              />
            </div>
          )}
        </section>
      </div>
    </div>
  );
}

/** Today with the records it shows (`platform/records`). */
export const Today = withRecords(
  [
    "assignments",
    "courses",
    "materials",
    "events",
    "memberships",
    "planItems",
    "exceptions",
    "scheduleOverrides",
    "intensiveSchedules",
    "academicProfile",
  ],
  TodayBody,
);
