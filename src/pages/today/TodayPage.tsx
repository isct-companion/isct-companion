import { useEffect, useRef, useState } from "react";
import { UiButton } from "../../components/mantine";
import { deadlineState } from "../../core/academic";
import { contentLanguage, pickLocalized, type PersonalEvent, type Plan, type TermId, type Workspace } from "../../core/model";
import { timetableCourses } from "../../components/timetable/workspace";
import { jstMinutes } from "../../components/timetable/timetable";
import { jstDate } from "../../app/shared";
import { CourseFinder } from "./CourseFinder";
import {
  addDays,
  calendarState,
  civilWeekday,
  importantDatesOnDate,
  resolveDayAgenda,
} from "../../core/schedule";

const timeLabel = (value: string) => value.replace(/^0/, "");
const DAY_NAMES = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
/** Rows shown before a list folds behind "+N more". */
const CLASS_ROWS = 6;
const DUE_ROWS = 8;
export function Today({
  data,
  term,
  open,
  openEvent,
  openPlan,
  openTask,
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
  openEvent: (event: PersonalEvent) => void;
  openPlan: (plan: Plan) => void;
  openTask: (assignmentId: string) => void;
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
  const [allDue, setAllDue] = useState(false);
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    if (!active) return;
    setNow(new Date());
    const timer = setInterval(() => setNow(new Date()), 30000);
    return () => clearInterval(timer);
  }, [active]);
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
  const teachingState = calendarState(data, viewIso);
  const effectiveDay = civilWeekday(viewIso);
  const courses = timetableCourses(data, term);
  const events = data.events.filter(
    (event) => event.termId === term && !event.deletedAt,
  );
  const blocks = resolveDayAgenda(data, term, viewIso, { courses, events });
  const importantDates = importantDatesOnDate(data, viewIso);
  const week = Array.from({ length: 7 }, (_, index) => {
    const date = addDays(weekStart, index);
    return {
      date,
      busy:
        date === viewIso
          ? blocks.length > 0
          : resolveDayAgenda(data, term, date, { courses, events }).length > 0,
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

  const assignments = data.assignments
    .filter((assignment) => {
      if (
        assignment.deletedAt ||
        assignment.hidden ||
        assignment.submission === "submitted" ||
        assignment.submission === "graded"
      )
        return false;
      return (
        assignment.dueAt !== null &&
        Number.isFinite(new Date(assignment.dueAt).getTime())
      );
    })
    .sort((left, right) =>
      (left.dueAt ?? "").localeCompare(right.dueAt ?? ""),
    );
  // Every open assignment of this quarter, however far its deadline; work
  // from earlier quarters (never-submitted reports of finished courses)
  // stays folded.
  const inThisQuarter = (assignment: (typeof assignments)[number]) => {
    const course = data.courses.find(
      (item) => item.id === assignment.courseId && !item.deletedAt,
    );
    return course
      ? course.termId === term
      : assignment.dueAt !== null &&
        new Date(assignment.dueAt).getTime() >= now.getTime();
  };
  const current = assignments.filter(inThisQuarter);
  // This week (overdue included) first; later deadlines in their own group.
  const weekEnd = now.getTime() + 7 * 86_400_000;
  const soon = current.filter(
    (item) => new Date(item.dueAt!).getTime() <= weekEnd,
  );
  const later = current.filter(
    (item) => new Date(item.dueAt!).getTime() > weekEnd,
  );
  const earlier = assignments.filter(
    (assignment) => !inThisQuarter(assignment),
  );
  const row = (assignment: (typeof assignments)[number]) => {
    const course = data.courses.find(
      (item) => item.id === assignment.courseId && !item.deletedAt,
    );
    const due = deadlineState(assignment, now);
    return (
      <div className="dueRow" key={assignment.id}>
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
              {course?.title ?? "LMS"} · {assignment.submission ?? "unknown"}
            </small>
          </span>
          <time className={due.ms < 0 ? "overdue" : due.tone}>{due.label}</time>
        </UiButton>
        <UiButton
          className="textButton"
          onClick={() => hideAssignment(assignment.id)}
        >
          Hide
        </UiButton>
      </div>
    );
  };
  // With no agenda today, show the next actual occurrence within a week.
  const nextClass = (() => {
    if (blocks.length) return undefined;
    for (let offset = 1; offset <= 7; offset++) {
      const date = addDays(viewIso, offset);
      const day = civilWeekday(date);
      const first = resolveDayAgenda(data, term, date, { courses, events })[0];
      if (first)
        return {
          ...first,
          date: date,
          day: offset === 1 && isToday ? "Tomorrow" : DAY_NAMES[day - 1],
        };
    }
    return undefined;
  })();
  const more = (hidden: number, expanded: boolean, toggle: () => void) =>
    hidden > 0 || expanded ? (
      <UiButton className="textButton moreToggle" onClick={toggle}>
        {expanded ? "Show less" : `+${hidden} more`}
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
        : effectiveDay > 5
          ? "Weekend"
          : "Teaching day";

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
            {new Intl.DateTimeFormat("en-GB", {
              weekday: "long",
              timeZone: "Asia/Tokyo",
            }).format(viewDate)}
          </p>
          <h1>
            {new Intl.DateTimeFormat("en-GB", {
              day: "numeric",
              month: "long",
              timeZone: "Asia/Tokyo",
            }).format(viewDate)}
          </h1>
        </div>
        {!compact && (
          <div className="todayWeek">
            <div
              className="weekStrip"
              role="group"
              aria-label="Week"
              style={
                {
                  "--day": effectiveDay - 1,
                  "--swap": direction,
                } as React.CSSProperties
              }
            >
              <UiButton
                className="weekShift"
                aria-label="Previous week"
                onClick={() => pick(addDays(viewIso, -7))}
              >
                ‹
              </UiButton>
              {week.map(({ date, busy }, index) => (
                <UiButton
                  key={date}
                  className={`weekDay${date === viewIso ? " active" : ""}${date === todayIso ? " today" : ""}${busy ? " busy" : ""}`}
                  aria-pressed={date === viewIso}
                  aria-label={date}
                  onClick={() => pick(date === todayIso ? null : date)}
                >
                  <small>{DAY_NAMES[index].slice(0, 1)}</small>
                  <b>{Number(date.slice(8))}</b>
                </UiButton>
              ))}
              <UiButton
                className="weekShift"
                aria-label="Next week"
                onClick={() => pick(addDays(viewIso, 7))}
              >
                ›
              </UiButton>
            </div>
            <span className="teachingState">
              {!isToday && (
                <UiButton
                  className="textButton"
                  onClick={() => pick(null)}
                >
                  Today
                </UiButton>
              )}
              {stateLabel}
            </span>
          </div>
        )}
      </header>

      <div className="todayFlow">
        {importantDates.length > 0 && (
          <aside className="importantDates" aria-label="Important dates">
            {importantDates.map((item) => (
              <span key={item.id}>
                <b>{pickLocalized(item.label, contentLanguage(data.settings), "Important date")}</b>
                {item.startDate !== item.endDate && (
                  <small>{item.startDate}–{item.endDate}</small>
                )}
              </span>
            ))}
          </aside>
        )}
        <section className="todaySection" data-today-section="classes">
          <header>
            <h2>Schedule</h2>
            <UiButton className="textButton" onClick={openSchedule}>
              Week
            </UiButton>
          </header>
          <div className="dayAgenda daySwap" key={viewIso} style={swap}>
            {(allClasses ? blocks : blocks.slice(0, CLASS_ROWS)).map(
              (block, index) => {
                const state = !isToday
                  ? ""
                  : index === activeIndex
                    ? "Now"
                    : index === nextIndex
                      ? "Next"
                      : block.endTime <= clock
                        ? "Earlier"
                        : "Later";
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
                      if (block.kind === "course" || block.kind === "exam")
                        open(block.id, undefined, {
                          date: viewIso,
                          startTime: block.startTime,
                          endTime: block.endTime,
                        });
                      else if (block.kind === "event") {
                        const event = events.find(
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
                      {block.kind === "exam" && <small>EXAM · {block.timeLabel}</small>}
                    </time>
                    <span className="agendaLine" aria-hidden="true" />
                    <span className="agendaCourse">
                      <b>{block.title}</b>
                      <small>
                        {[block.room || block.subtitle, block.conflictWith?.length ? `Overlaps ${block.conflictWith.join(", ")}` : ""]
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
                  {isToday ? "Nothing scheduled today" : "Nothing scheduled"}
                </p>
                {nextClass && (
                  <UiButton
                    className={`dayAgendaRow focus nextClass${selection?.courseId === nextClass.id && !selection.assignmentId ? " selected" : ""}`}
                    style={
                      { "--course": nextClass.color } as React.CSSProperties
                    }
                    onClick={() => {
                      if (nextClass.kind === "event") {
                        const event = events.find(
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
                    <span className="agendaState">Next</span>
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
            <h2>Assignments</h2>
            <span>{current.length ? `${current.length} active` : "Clear"}</span>
          </header>
          <div className="dueList">
            {(allDue ? soon : soon.slice(0, DUE_ROWS)).map(row)}
            {soon.length > DUE_ROWS &&
              more(allDue ? 0 : soon.length - DUE_ROWS, allDue, () =>
                setAllDue((value) => !value),
              )}
            {!soon.length && (
              <p className="quietEmpty compact">Nothing due this week.</p>
            )}
            {later.length > 0 && (
              <div className="dueLater">
                <small>Later · {later.length}</small>
                {later.map(row)}
              </div>
            )}
            {earlier.length > 0 && (
              <details className="dueEarlier">
                <summary>Earlier quarters · {earlier.length}</summary>
                {earlier.map(row)}
              </details>
            )}
          </div>
        </section>
      </div>
    </div>
  );
}
