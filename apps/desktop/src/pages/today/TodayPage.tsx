import { ExamCards } from "../../components/exams/ExamCards";
import { deadlineText } from "../../lib/deadline-text";
import { formatJst } from "../../lib/civil-date";
import { submissionLabel } from "../../lib/academic";
import { useEffect, useRef, useState } from "react";
import { SwipeActions } from "../../components/SwipeActions";
import { useFormFactor } from "../../app/form-factor";
import { t } from "../../i18n";
import { UiButton } from "../../components/mantine";
import {
  contentLanguage,
  pickLocalized,
  type Assignment,
  type PersonalEvent,
  type Plan,
  type TermId,
  type Workspace,
} from "../../lib/model";
import { jstMinutes } from "../../components/timetable/timetable";
import { jstDate } from "../../app/shared";
import { CourseFinder } from "./CourseFinder";
import { AssignmentGroupList } from "../../components/assignments/AssignmentGroupList";
import { addCivilDays as addDays, civilWeekday } from "../../lib/civil-date";
import {
  useScheduleQuery,
  type ScheduleView,
  type TaskView,
} from "../../platform/core-query";
import { useClock } from "../../platform/use-clock";

const timeLabel = (value: string) => value.replace(/^0/, "");
const DAY_NAMES = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
/** Rows shown before a list folds behind "+N more". */
const CLASS_ROWS = 6;
const DUE_ROWS = 8;
export function Today({
  data,
  term,
  open,
  openExam,
  openEvent,
  openPlan,
  openTask,
  openAssignments,
  hideAssignment,
  pinAssignment,
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
  openPlan: (plan: Plan) => void;
  openTask: (assignmentId: string) => void;
  /** Lecture Desk's full list of the quarter's assignments. */
  openAssignments?: () => void;
  hideAssignment: (assignmentId: string) => void;
  pinAssignment?: (assignmentId: string) => void;
  openSchedule: () => void;
  compact?: boolean;
  active?: boolean;
  selection?: { courseId: string; assignmentId?: string };
  focusSection?: { section: "classes" | "assignments"; request: number };
}) {
  const mobile = useFormFactor() === "mobile";
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
  const todayIso = jstDate(now);
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
  const schedule = useScheduleQuery<ScheduleView>({
    kind: "scheduleView",
    term,
    from: weekStart,
    to: addDays(viewIso, 7),
    includePersonal: true,
  }).value;
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
  const minutes = jstMinutes(now);
  const clock = `${String(Math.floor(minutes / 60)).padStart(2, "0")}:${String(minutes % 60).padStart(2, "0")}`;
  const activeIndex = isToday
    ? blocks.findIndex(
        (block) => block.startTime <= clock && block.endTime > clock,
      )
    : -1;
  const nextIndex = isToday
    ? blocks.findIndex((block) => block.startTime > clock)
    : -1;
  const focusIndex = activeIndex >= 0 ? activeIndex : nextIndex;

  const taskView = useScheduleQuery<TaskView>({
    kind: "taskView",
    now: nowIso,
  }).value;
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
    const content = (
      <div
        className={`dueRow${state}${mobile ? " touchRow" : ""}`}
        key={assignment.id}
      >
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
          {due &&
            (!due.overdue && !submitted ? (
              <time
                className={`dueChip ${due.countdownTone}`}
                dateTime={assignment.dueAt!}
                title={deadlineText(due, data.settings.locale).label}
              >
                {deadlineText(due, data.settings.locale).countdownLabel}
              </time>
            ) : (
              <time className={due.overdue ? "overdue" : due.tone}>
                {deadlineText(due, data.settings.locale).label}
              </time>
            ))}
        </UiButton>
        {!mobile && (
          <UiButton
            className="textButton"
            onClick={() => hideAssignment(assignment.id)}
          >
            {t("Hide")}
          </UiButton>
        )}
      </div>
    );
    return mobile ? (
      <SwipeActions
        key={assignment.id}
        leading={
          pinAssignment
            ? {
                label: assignment.pinned ? "Unpin" : "Pin",
                run: () => pinAssignment(assignment.id),
              }
            : undefined
        }
        trailing={{ label: "Hide", run: () => hideAssignment(assignment.id) }}
      >
        {content}
      </SwipeActions>
    ) : (
      content
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
          day: t(offset === 1 && isToday ? "Tomorrow" : DAY_NAMES[day - 1]),
        };
    }
    return undefined;
  })();
  const more = (hidden: number, expanded: boolean, toggle: () => void) =>
    hidden > 0 || expanded ? (
      <UiButton className="textButton moreToggle" onClick={toggle}>
        {expanded ? t("Show less") : t("+{{count}} more", { count: hidden })}
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
        : t(effectiveDay > 5 ? "Weekend" : "Teaching day");

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
              timeZone: "Asia/Tokyo",
            })}
          </p>
          <h1>
            {formatJst(viewDate, data.settings.locale, {
              day: "numeric",
              month: "long",
              timeZone: "Asia/Tokyo",
            })}
          </h1>
        </div>
        {!compact && (
          <div className="todayWeek">
            <div
              className="weekStrip"
              role="group"
              aria-label={t("Week")}
              style={
                {
                  "--day": effectiveDay - 1,
                  "--swap": direction,
                } as React.CSSProperties
              }
            >
              <UiButton
                className="weekShift"
                aria-label={t("Previous week")}
                onClick={() => pick(addDays(viewIso, -7))}
              >
                ‹
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
                aria-label={t("Next week")}
                onClick={() => pick(addDays(viewIso, 7))}
              >
                ›
              </UiButton>
            </div>
            <span className="teachingState">
              {!isToday && (
                <UiButton className="textButton" onClick={() => pick(null)}>
                  {t("Today")}
                </UiButton>
              )}
              {stateLabel}
            </span>
          </div>
        )}
      </header>

      <div className="todayFlow">
        {importantDates.length > 0 && (
          <aside className="importantDates" aria-label={t("Important dates")}>
            {importantDates.map((item) => (
              <span key={item.id}>
                <b>
                  {pickLocalized(
                    item.label,
                    contentLanguage(data.settings),
                    t("Important date"),
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
            !exam.hidden &&
            exam.status === "scheduled" &&
            exam.date >= viewIso,
        ) && (
          <section className="todaySection" aria-label={t("Upcoming exams")}>
            <header>
              <h2>{t("Upcoming exams")}</h2>
            </header>
            <ExamCards
              exams={(schedule?.upcomingExams ?? [])
                .filter(
                  (exam) =>
                    exam.courseId &&
                    !exam.hidden &&
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
            <h2>{t("Schedule", { context: "today" })}</h2>
            <UiButton className="textButton" onClick={openSchedule}>
              {t("Week")}
            </UiButton>
          </header>
          <div className="dayAgenda daySwap" key={viewIso} style={swap}>
            {(allClasses ? blocks : blocks.slice(0, CLASS_ROWS)).map(
              (block, index) => {
                const state = !isToday
                  ? ""
                  : t(
                      index === activeIndex
                        ? "Now"
                        : index === nextIndex
                          ? "Next"
                          : block.endTime <= clock
                            ? "Earlier"
                            : "Later",
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
                    onClick={() => {
                      if (block.kind === "exam" && block.termId && openExam)
                        openExam(block.key, block.termId);
                      else if (block.kind === "course" || block.kind === "exam")
                        open(block.id, undefined, {
                          date: viewIso,
                          startTime: block.startTime,
                          endTime: block.endTime,
                        });
                      else if (block.kind === "event") {
                        const event = data.events.find(
                          (item) => item.id === block.id,
                        );
                        if (event) openEvent(event);
                      } else {
                        const plan = data.plans.find(
                          (item) => item.id === block.id,
                        );
                        if (plan) openPlan(plan);
                      }
                    }}
                    key={block.key}
                  >
                    <span className="agendaState">{state}</span>
                    <time>
                      {timeLabel(block.startTime)}
                      {block.kind === "exam" && (
                        <small>
                          {t("Exam")} · {block.timeLabel}
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
                            ? t("Overlaps {{courses}}", {
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
                <p className="quietEmpty compact">
                  {t(isToday ? "Nothing scheduled today" : "Nothing scheduled")}
                </p>
                {nextClass && (
                  <UiButton
                    className={`dayAgendaRow focus nextClass${selection?.courseId === nextClass.id && !selection.assignmentId ? " selected" : ""}`}
                    style={
                      { "--course": nextClass.color } as React.CSSProperties
                    }
                    onClick={() => {
                      if (nextClass.kind === "event") {
                        const event = data.events.find(
                          (item) => item.id === nextClass.id,
                        );
                        if (event) openEvent(event);
                      } else if (nextClass.kind === "plan") {
                        const plan = data.plans.find(
                          (item) => item.id === nextClass.id,
                        );
                        if (plan) openPlan(plan);
                      } else
                        open(nextClass.id, undefined, {
                          date: nextClass.date,
                          startTime: nextClass.startTime,
                          endTime: nextClass.endTime,
                        });
                    }}
                  >
                    <span className="agendaState">{t("Next")}</span>
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
        </section>

        <section
          className="todaySection dueSection"
          data-today-section="assignments"
        >
          <header>
            <h2>{t("Assignments")}</h2>
            <span>
              {pendingCount
                ? t("{{count}} active", { count: pendingCount })
                : t("All clear")}
            </span>
            {openAssignments && (
              <UiButton className="textButton" onClick={openAssignments}>
                {t("All assignments")} →
              </UiButton>
            )}
          </header>
          <div className="dueList">
            <AssignmentGroupList
              groups={taskTerm?.today}
              assignments={byAssignment}
              row={row}
              thisWeekLimit={DUE_ROWS}
              pastLabel={t("Missed")}
              empty={t("Nothing due this week")}
            />
          </div>
        </section>
      </div>
    </div>
  );
}
