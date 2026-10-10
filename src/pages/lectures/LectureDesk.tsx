import "./lectures.css";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { dueCountdown } from "../../core/academic";
import type { Assignment, TermId, Workspace } from "../../core/model";
import { addDays, civilWeekday, resolveDayAgenda } from "../../core/schedule";
import {
  allTerms,
  timetableCourses,
} from "../../components/timetable/workspace";
import { UiInput, UiSelect } from "../../components/mantine";
import { jstMinutes } from "../../components/timetable/timetable";
import { jstDate } from "../../app/shared";
import { t } from "../../i18n";
import type { CourseEntry } from "../today/lms/LmsWorkspace";

const DAY_NAMES = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const narrow = () =>
  typeof matchMedia === "function" && matchMedia("(max-width: 48em)").matches;
const submitted = (a: Assignment) =>
  a.submission === "submitted" ||
  a.submission === "graded" ||
  a.submissionStatus === "submitted";

type Lecture = {
  id: string;
  title: string;
  code: string;
  color: string;
  /** Minutes from now to the next meeting within a week; 0 while in class. */
  rank: number;
  when?: string;
  room?: string;
  due?: ReturnType<typeof dueCountdown>;
};

/** This quarter's lectures, in class order, and the open lecture's workspace. */
export function LectureDesk({
  data,
  term,
  entry,
  select,
  clear,
  active,
  renderWorkspace,
  restoreAssignment,
}: {
  data: Workspace;
  term: TermId;
  entry: CourseEntry | null;
  select: (courseId: string, assignmentId?: string) => void;
  clear: () => void;
  active: boolean;
  renderWorkspace: (entry: CourseEntry, active: boolean) => ReactNode;
  /** Brings a hidden assignment back to Today and this list. */
  restoreAssignment?: (assignmentId: string) => void;
}) {
  // The quarter shown here is browsed on its own; it starts at the current one.
  const [shownTerm, setShownTerm] = useState(term);
  useEffect(() => setShownTerm(term), [term]);
  const [query, setQuery] = useState("");
  // The list shows the quarter's lectures, or its open assignments.
  const [mode, setMode] = useState<"lectures" | "assignments">("lectures");
  const [showHidden, setShowHidden] = useState(false);
  const terms = allTerms(data, term);
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    if (!active) return;
    setNow(new Date());
    const timer = setInterval(() => setNow(new Date()), 60000);
    return () => clearInterval(timer);
  }, [active]);

  const lectures = useMemo(() => {
    const courses = timetableCourses(data, shownTerm);
    const events = data.events.filter(
      (event) => event.termId === shownTerm && !event.deletedAt,
    );
    const today = jstDate(now);
    const minutes = jstMinutes(now);
    const clock = `${String(Math.floor(minutes / 60)).padStart(2, "0")}:${String(minutes % 60).padStart(2, "0")}`;
    const next = new Map<
      string,
      { rank: number; when: string; room?: string }
    >();
    for (let offset = 0; offset <= 7; offset++) {
      const date = addDays(today, offset);
      for (const block of resolveDayAgenda(data, shownTerm, date, {
        courses,
        events,
      })) {
        if (block.kind !== "course" || next.has(block.id)) continue;
        if (offset === 0 && block.endTime <= clock) continue;
        const [h, m] = block.startTime.split(":").map(Number);
        const inClass = offset === 0 && block.startTime <= clock;
        next.set(block.id, {
          rank: inClass ? 0 : offset * 1440 + h * 60 + m - minutes,
          when: inClass
            ? t("Now")
            : `${offset === 0 ? t("Today") : offset === 1 ? t("Tomorrow") : t(DAY_NAMES[civilWeekday(date) - 1])} ${block.startTime.replace(/^0/, "")}`,
          room: block.room || undefined,
        });
      }
    }
    const due = new Map<string, Assignment>();
    for (const assignment of data.assignments) {
      if (
        assignment.deletedAt ||
        assignment.hidden ||
        !assignment.dueAt ||
        submitted(assignment) ||
        new Date(assignment.dueAt).getTime() < now.getTime()
      )
        continue;
      const nearest = due.get(assignment.courseId);
      if (!nearest || assignment.dueAt < nearest.dueAt!)
        due.set(assignment.courseId, assignment);
    }
    return courses
      .map((course): Lecture => {
        const meeting = next.get(course.id);
        const work = due.get(course.id);
        return {
          id: course.id,
          title: course.title,
          code: course.code,
          color: course.color,
          rank: meeting?.rank ?? Infinity,
          when: meeting?.when,
          room: meeting?.room,
          due: work ? dueCountdown(work.dueAt!, now) : undefined,
        };
      })
      .sort(
        (left, right) =>
          left.rank - right.rank || left.title.localeCompare(right.title),
      );
  }, [data, shownTerm, now]);
  const work = useMemo(() => {
    const courses = new Map(
      timetableCourses(data, shownTerm).map((course) => [course.id, course]),
    );
    return data.assignments
      .filter(
        (a) =>
          !a.deletedAt &&
          !a.hidden &&
          courses.has(a.courseId) &&
          !submitted(a) &&
          (!a.dueAt || new Date(a.dueAt).getTime() >= now.getTime()),
      )
      .sort((left, right) =>
        (left.dueAt ?? "9999").localeCompare(right.dueAt ?? "9999"),
      )
      .map((a) => ({
        assignment: a,
        course: courses.get(a.courseId)!,
        due: a.dueAt ? dueCountdown(a.dueAt, now) : undefined,
      }));
  }, [data, shownTerm, now]);
  // Hidden assignments stay with their quarter, folded under the open ones.
  const hiddenWork = useMemo(() => {
    const courses = new Map(
      timetableCourses(data, shownTerm).map((course) => [course.id, course]),
    );
    return data.assignments
      .filter((a) => !a.deletedAt && a.hidden && courses.has(a.courseId))
      .sort((left, right) =>
        (left.dueAt ?? "9999").localeCompare(right.dueAt ?? "9999"),
      )
      .map((a) => ({ assignment: a, course: courses.get(a.courseId)! }));
  }, [data, shownTerm]);
  const needle = query.trim().toLowerCase();
  const shownWork = needle
    ? work.filter(({ assignment, course }) =>
        `${assignment.title} ${course.title} ${course.code}`
          .toLowerCase()
          .includes(needle),
      )
    : work;
  const shownHidden = needle
    ? hiddenWork.filter(({ assignment, course }) =>
        `${assignment.title} ${course.title} ${course.code}`
          .toLowerCase()
          .includes(needle),
      )
    : hiddenWork;
  const shown = needle
    ? lectures.filter((lecture) =>
        `${lecture.title} ${lecture.code}`.toLowerCase().includes(needle),
      )
    : lectures;

  // Arriving with nothing open shows the class happening now or next.
  const first = lectures[0]?.id;
  useEffect(() => {
    if (active && !entry && first && !narrow()) select(first);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, Boolean(entry), first]);

  // The two most recent lectures stay mounted so switching back is instant.
  const [recent, setRecent] = useState<CourseEntry[]>([]);
  useEffect(() => {
    if (entry)
      setRecent((previous) =>
        [
          ...previous.filter((item) => item.courseId !== entry.courseId),
          entry,
        ].slice(-2),
      );
  }, [entry]);
  const retained = entry
    ? [
        ...recent.filter((item) => item.courseId !== entry.courseId),
        entry,
      ].slice(-2)
    : recent;

  return (
    <div className={`page lectureDesk${entry ? " open" : ""}`}>
      <nav className="lectureList" aria-label={t("Lectures")}>
        <header>
          <h1>{t("Lecture Desk")}</h1>
          <UiSelect
            className="lectureTerm"
            aria-label={t("Quarter")}
            value={shownTerm}
            onChange={(event) => setShownTerm(event.target.value as TermId)}
          >
            {[...new Set(terms.map((item) => item.slice(0, 4)))].map((year) => (
              <optgroup key={year} label={year}>
                {terms
                  .filter((item) => item.startsWith(`${year}-`))
                  .map((item) => (
                    <option key={item} value={item}>
                      {item.replace("-", " ")}
                    </option>
                  ))}
              </optgroup>
            ))}
          </UiSelect>
        </header>
        <div className="dueScope lectureMode" role="group" aria-label={t("Show")}>
          {(["lectures", "assignments"] as const).map((item) => (
            <button
              type="button"
              key={item}
              aria-pressed={mode === item}
              onClick={() => setMode(item)}
            >
              {t(item === "lectures" ? "Lectures" : "Assignments")}
            </button>
          ))}
        </div>
        <UiInput
          className="lectureFilter"
          type="search"
          aria-label={t("Filter")}
          placeholder={t("Filter…")}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => event.key === "Escape" && setQuery("")}
        />
        {mode === "assignments" &&
          shownWork.map(({ assignment, course, due }) => (
            <button
              type="button"
              key={assignment.id}
              className={`lectureRow${entry?.assignmentId === assignment.id ? " selected" : ""}`}
              aria-current={entry?.assignmentId === assignment.id ? "true" : undefined}
              style={{ "--course": course.color } as React.CSSProperties}
              onClick={() => select(course.id, assignment.id)}
            >
              <b>{assignment.title}</b>
              <small>{course.title}</small>
              {due && <span className={`dueChip ${due.tone}`}>{due.label}</span>}
            </button>
          ))}
        {mode === "assignments" && !shownWork.length && (
          <p className="quietEmpty compact">{t(work.length ? "No matching assignments." : "No open assignments.")}</p>
        )}
        {mode === "assignments" && restoreAssignment && shownHidden.length > 0 && (
          <>
            <button
              type="button"
              className="lectureHiddenToggle"
              aria-expanded={showHidden}
              onClick={() => setShowHidden((value) => !value)}
            >
              {showHidden ? "▾" : "▸"} {t("Hidden assignments")}
            </button>
            {showHidden &&
              shownHidden.map(({ assignment, course }) => (
                <div
                  key={assignment.id}
                  className="lectureRow hidden"
                  style={{ "--course": course.color } as React.CSSProperties}
                >
                  <b>{assignment.title}</b>
                  <small>{course.title}</small>
                  <button
                    type="button"
                    className="textButton"
                    onClick={() => restoreAssignment(assignment.id)}
                  >
                    {t("Restore")}
                  </button>
                </div>
              ))}
          </>
        )}
        {mode === "lectures" && shown.map((lecture) => (
          <button
            type="button"
            key={lecture.id}
            className={`lectureRow${entry?.courseId === lecture.id ? " selected" : ""}${lecture.rank === 0 ? " now" : ""}`}
            aria-current={entry?.courseId === lecture.id ? "true" : undefined}
            style={{ "--course": lecture.color } as React.CSSProperties}
            onClick={() => select(lecture.id)}
          >
            <b>{lecture.title}</b>
            <small>
              {[lecture.when, lecture.room].filter(Boolean).join(" · ") ||
                lecture.code}
            </small>
            {lecture.due && (
              <span className={`dueChip ${lecture.due.tone}`}>
                {lecture.due.label}
              </span>
            )}
          </button>
        ))}
        {mode === "lectures" && !shown.length && (
          <p className="quietEmpty compact">
            {t(
              lectures.length
                ? "No matching lectures."
                : "No lectures on this quarter's timetable.",
            )}
          </p>
        )}
      </nav>
      <div className="lectureContent">
        {entry && (
          <button type="button" className="lectureBack" onClick={clear}>
            ‹ {t("Lectures")}
          </button>
        )}
        {retained.map((item) => (
          <div
            className="lectureSlot"
            key={item.courseId}
            hidden={entry?.courseId !== item.courseId}
          >
            {renderWorkspace(item, active && entry?.courseId === item.courseId)}
          </div>
        ))}
      </div>
    </div>
  );
}
