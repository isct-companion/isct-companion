import { useState } from "react";
import { UiButton } from "../../components/mantine";
import { deadlineState } from "../../core/academic";
import type { PersonalEvent, Plan, TermId, Workspace } from "../../core/model";
import {
  dayBlocks,
  timetableCourses,
} from "../../components/timetable/workspace";
import { jstMinutes } from "../../components/timetable/timetable";
import { jstDate } from "../../app/shared";
import { clockForPeriods, planOccursOn } from "../../core/user-schedule";

const timeLabel = (value: string) => value.replace(/^0/, "");
const DAY_NAMES = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
/** Rows shown before a list folds behind "+N more". */
const CLASS_ROWS = 6;
const DUE_ROWS = 8;
const weekday = (date: Date) => {
  const value = date.getDay();
  return value === 0 ? 7 : value;
};

function rangeContains(date: string, from: string, to: string) {
  return date >= from && date <= to;
}

export function Today({
  data,
  term,
  open,
  openEvent,
  openPlan,
  openTask,
  hideAssignment,
  openSchedule,
}: {
  data: Workspace;
  term: TermId;
  open: (courseId: string, tab?: "assignments") => void;
  openEvent: (event: PersonalEvent) => void;
  openPlan: (plan: Plan) => void;
  openTask: (assignmentId: string) => void;
  hideAssignment: (assignmentId: string) => void;
  openSchedule: () => void;
}) {
  const [allClasses, setAllClasses] = useState(false);
  const [allDue, setAllDue] = useState(false);
  const now = new Date();
  const todayIso = jstDate(now);
  const personalException = data.exceptions.find(
    (item) => item.date === todayIso && !item.deletedAt,
  );
  const subscribedDay = data.academicCalendar.find(
    (item) => item.date === todayIso && !item.deletedAt,
  );
  const teachingState = personalException ?? subscribedDay;
  const noClasses =
    teachingState?.type === "no-class" ||
    teachingState?.type === "break" ||
    teachingState?.type === "holiday";
  const effectiveDay =
    teachingState?.type === "replacement" && teachingState.replacementDay
      ? teachingState.replacementDay
      : weekday(now);

  const courses = timetableCourses(data, term);
  const events = data.events.filter(
    (event) => event.termId === term && !event.deletedAt,
  );
  const weekly = noClasses
    ? []
    : dayBlocks(courses, events, effectiveDay, todayIso);
  const intensive = data.intensiveSchedules.flatMap((schedule) => {
    if (
      schedule.deletedAt ||
      !rangeContains(todayIso, schedule.from, schedule.to)
    )
      return [];
    const override = schedule.days?.find((item) => item.date === todayIso);
    if (override?.periods === null) return [];
    const course = courses.find((item) => item.id === schedule.courseId);
    if (!course) return [];
    const periods = override?.periods ?? schedule.periods;
    const clock = clockForPeriods(periods);
    return [
      {
        key: `intensive-${schedule.id}-${todayIso}`,
        id: course.id,
        kind: "course" as const,
        title: course.title,
        subtitle: course.code,
        room: schedule.room ?? "",
        startTime: clock.start,
        endTime: clock.end,
        color: course.color,
        conflict: false,
        dim: false,
      },
    ];
  });
  const plans = data.plans
    .filter((plan) => !plan.deletedAt && planOccursOn(plan, todayIso))
    .map((plan) => {
      const startTime = plan.start.includes("T")
        ? plan.start.slice(11, 16)
        : "08:50";
      const [hour, minute] = startTime.split(":").map(Number);
      const end = new Date(2000, 0, 1, hour, minute + plan.duration);
      return {
        key: `plan-${plan.id}-${todayIso}`,
        id: plan.id,
        kind: "plan" as const,
        title: plan.title,
        subtitle: "Plan",
        room: plan.room ?? "",
        startTime,
        endTime: `${String(end.getHours()).padStart(2, "0")}:${String(end.getMinutes()).padStart(2, "0")}`,
        color: "#5f6b8a",
      };
    });
  const blocks = [...weekly, ...intensive, ...plans].sort((a, b) =>
    a.startTime.localeCompare(b.startTime),
  );
  const minutes = jstMinutes(now);
  const clock = `${String(Math.floor(minutes / 60)).padStart(2, "0")}:${String(minutes % 60).padStart(2, "0")}`;
  const activeIndex = blocks.findIndex(
    (block) => block.startTime <= clock && block.endTime > clock,
  );
  const nextIndex = blocks.findIndex((block) => block.startTime > clock);
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
      return Number.isFinite(new Date(assignment.dueAt).getTime());
    })
    .sort((left, right) => left.dueAt.localeCompare(right.dueAt));
  // Every open assignment of this quarter, however far its deadline; work
  // from earlier quarters (never-submitted reports of finished courses)
  // stays folded.
  const inThisQuarter = (assignment: (typeof assignments)[number]) => {
    const course = data.courses.find(
      (item) => item.id === assignment.courseId && !item.deletedAt,
    );
    return course
      ? course.termId === term
      : new Date(assignment.dueAt).getTime() >= now.getTime();
  };
  const current = assignments.filter(inThisQuarter);
  // This week (overdue included) first; later deadlines in their own group.
  const weekEnd = now.getTime() + 7 * 86_400_000;
  const soon = current.filter((item) => new Date(item.dueAt).getTime() <= weekEnd);
  const later = current.filter((item) => new Date(item.dueAt).getTime() > weekEnd);
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
          className="dueAssignment"
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
  // With no class today, the next weekly class (within a week) takes its place.
  const nextClass = (() => {
    if (blocks.length) return undefined;
    for (let offset = 1; offset <= 7; offset++) {
      const date = new Date(now.getTime() + offset * 86_400_000);
      const day = weekday(date);
      const first = dayBlocks(courses, events, day, jstDate(date))[0];
      if (first) return { ...first, day: offset === 1 ? "Tomorrow" : DAY_NAMES[day - 1] };
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
        ? (rawStateLabel[data.settings.locale === "ja" ? "ja" : "en"] ??
          rawStateLabel.ja ??
          rawStateLabel.en)
        : effectiveDay > 5
          ? "Weekend"
          : "Teaching day";

  return (
    <div className="page todayPage">
      <header className="todayHeader">
        <div>
          <p className="todayEyebrow">
            {new Intl.DateTimeFormat("en-GB", {
              weekday: "long",
              timeZone: "Asia/Tokyo",
            }).format(now)}
          </p>
          <h1>
            {new Intl.DateTimeFormat("en-GB", {
              day: "numeric",
              month: "long",
              timeZone: "Asia/Tokyo",
            }).format(now)}
          </h1>
        </div>
        <span className="teachingState">{stateLabel}</span>
      </header>

      <div className="todayFlow">
        <section className="todaySection">
          <header>
            <h2>Classes</h2>
            <UiButton className="textButton" onClick={openSchedule}>
              Week
            </UiButton>
          </header>
          <div className="dayAgenda">
            {(allClasses ? blocks : blocks.slice(0, CLASS_ROWS)).map((block, index) => {
              const state =
                index === activeIndex
                  ? "Now"
                  : index === nextIndex
                    ? "Next"
                    : block.endTime <= clock
                      ? "Earlier"
                      : "Later";
              return (
                <UiButton
                  className={`dayAgendaRow${index === focusIndex ? " focus" : ""}`}
                  style={{ "--course": block.color } as React.CSSProperties}
                  onClick={() => {
                    if (block.kind === "course") open(block.id);
                    else if (block.kind === "event") {
                      const event = events.find((item) => item.id === block.id);
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
                  <time>{timeLabel(block.startTime)}</time>
                  <span className="agendaLine" aria-hidden="true" />
                  <span className="agendaCourse">
                    <b>{block.title}</b>
                    <small>{block.room || block.subtitle || "—"}</small>
                  </span>
                </UiButton>
              );
            })}
            {blocks.length > CLASS_ROWS &&
              more(allClasses ? 0 : blocks.length - CLASS_ROWS, allClasses, () =>
                setAllClasses((value) => !value),
              )}
            {!blocks.length && (
              <p className="quietEmpty compact">
                No classes today
                {nextClass && (
                  <UiButton className="nextClass" onClick={() => open(nextClass.id)}>
                    Next · {nextClass.day} {timeLabel(nextClass.startTime)}{" "}
                    <b>{nextClass.title}</b>
                    {nextClass.room && <span> · {nextClass.room}</span>}
                  </UiButton>
                )}
              </p>
            )}
          </div>
        </section>

        <section className="todaySection dueSection">
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
            {!soon.length && <p className="quietEmpty compact">Nothing due this week.</p>}
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
