import { deadlineState, effectiveWeekday } from "../../core/academic";
import type { PersonalEvent, TermId, Workspace } from "../../core/model";
import {
  dayBlocks,
  timetableCourses,
} from "../../components/timetable/workspace";
import { jstMinutes } from "../../components/timetable/timetable";
import { jstDate } from "../../app/shared";

const timeLabel = (value: string) => value.replace(/^0/, "");

export function Today({
  data,
  term,
  open,
  openEvent,
  openTask,
  openSchedule,
  connectAccount,
}: {
  data: Workspace;
  term: TermId;
  open: (courseId: string, tab?: "assignments") => void;
  openEvent: (event: PersonalEvent) => void;
  openTask: (assignmentId: string) => void;
  openSchedule: () => void;
  connectAccount: () => void;
}) {
  const now = new Date();
  const todayIso = jstDate(now);
  const day = effectiveWeekday(now, data.exceptions);
  const calendarDay = Number(
    new Intl.DateTimeFormat("en-US", {
      weekday: "short",
      timeZone: "Asia/Tokyo",
    })
      .formatToParts(now)
      .find((part) => part.type === "weekday")?.value === "Sun"
      ? 7
      : day,
  );
  const exception = data.exceptions.find(
    (item) => item.date === todayIso && !item.deletedAt,
  );
  const courses = timetableCourses(data, term);
  const events = data.events.filter(
    (event) => event.termId === term && !event.deletedAt,
  );
  const blocks = day ? dayBlocks(courses, events, day, todayIso) : [];
  const minutes = jstMinutes(now);
  const clock = `${String(Math.floor(minutes / 60)).padStart(2, "0")}:${String(minutes % 60).padStart(2, "0")}`;
  const activeIndex = blocks.findIndex(
    (block) => block.startTime <= clock && block.endTime > clock,
  );
  const nextIndex = blocks.findIndex((block) => block.startTime > clock);
  const focusIndex = activeIndex >= 0 ? activeIndex : nextIndex;
  const assignments = data.assignments
    .filter(
      (assignment) =>
        !assignment.deletedAt &&
        !assignment.hidden &&
        assignment.submissionStatus !== "submitted" &&
        !assignment.completedAt,
    )
    .sort((left, right) => left.dueAt.localeCompare(right.dueAt));
  const urgent = assignments.filter(
    (assignment) =>
      new Date(assignment.dueAt).getTime() - now.getTime() < 5 * 864e5,
  );
  const connected = data.importStatus.school !== "not-configured";
  const selectBlock = (id: string, kind: "course" | "event") => {
    if (kind === "course") open(id);
    else {
      const event = events.find((item) => item.id === id);
      if (event) openEvent(event);
    }
  };

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
        <span className="teachingState">
          {exception?.label ||
            (calendarDay > 5 ? "Weekend" : day ? "Teaching day" : "No classes")}
        </span>
      </header>

      {!connected && !courses.length && !assignments.length ? (
        <section className="todayEmpty">
          <h2>Your day starts here.</h2>
          <UiButton
            type="button"
            className="button primary"
            onClick={connectAccount}
          >
            Connect account
          </UiButton>
        </section>
      ) : (
        <div className="todayFlow">
          <section className="todaySection">
            <header>
              <h2>Classes</h2>
              <UiButton
                type="button"
                className="textButton"
                onClick={openSchedule}
              >
                Week
              </UiButton>
            </header>
            <div className="dayAgenda">
              {blocks.map((block, index) => {
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
                    type="button"
                    className={`dayAgendaRow${index === focusIndex ? " focus" : ""}`}
                    style={{ "--course": block.color } as React.CSSProperties}
                    onClick={() => selectBlock(block.id, block.kind)}
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
              {!blocks.length && (
                <p className="quietEmpty">No classes today.</p>
              )}
            </div>
          </section>

          <section className="todaySection dueSection">
            <header>
              <h2>Assignments</h2>
              <span>
                {urgent.length ? `${urgent.length} due soon` : "Clear"}
              </span>
            </header>
            <div className="dueList">
              {urgent.slice(0, 6).map((assignment) => {
                const course = data.courses.find(
                  (item) => item.id === assignment.courseId && !item.deletedAt,
                );
                const due = deadlineState(assignment, now);
                return (
                  <UiButton
                    type="button"
                    className="dueRow"
                    onClick={() => openTask(assignment.id)}
                    key={assignment.id}
                  >
                    <i style={{ background: course?.color ?? "#888" }} />
                    <span>
                      <b>{assignment.title}</b>
                      <small>{course?.title ?? "LMS"}</small>
                    </span>
                    <time className={due.ms < 0 ? "overdue" : due.tone}>
                      {due.label}
                    </time>
                  </UiButton>
                );
              })}
              {!urgent.length && (
                <p className="quietEmpty">Nothing due soon.</p>
              )}
            </div>
          </section>
        </div>
      )}
    </div>
  );
}
import { UiButton } from "../../components/mantine";
