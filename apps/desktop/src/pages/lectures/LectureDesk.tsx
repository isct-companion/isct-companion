import { deadlineText } from "../../lib/deadline-text";
import "./lectures.css";
import { ResizableWorkspace } from "../../components/ResizableWorkspace";
import { usePanelRef } from "react-resizable-panels";
import { useMediaQuery } from "@mantine/hooks";
import {
  Activity,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import type { TermId, Workspace } from "../../lib/model";
import { addCivilDays as addDays, civilWeekday } from "../../lib/civil-date";
import { UiInput, UiSelect } from "../../components/mantine";
import { jstMinutes } from "../../components/timetable/timetable";
import { jstDate } from "../../app/shared";
import { t } from "../../i18n";
import type { CourseEntry } from "../../features/lms/LmsWorkspace";
import { useFormFactor } from "../../app/form-factor";
import { useClock } from "../../platform/use-clock";
import {
  useScheduleQueries,
  type ScheduleView,
  type TaskView,
} from "../../platform/core-query";

const DAY_NAMES = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
type Lecture = {
  id: string;
  title: string;
  code: string;
  color: string;
  /** Minutes from now to the next meeting within a week; 0 while in class. */
  rank: number;
  when?: string;
  room?: string;
  due?: { label: string; tone: string };
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
  railHovered = false,
}: {
  railHovered?: boolean;
  data: Workspace;
  term: TermId;
  entry: CourseEntry | null;
  select: (courseId: string, assignmentId?: string) => void;
  clear: () => void;
  active: boolean;
  renderWorkspace: (
    entry: CourseEntry,
    active: boolean,
    sidebarToggle?: ReactNode,
  ) => ReactNode;
  /** Brings a hidden assignment back to Today and this list. */
  restoreAssignment?: (assignmentId: string) => void;
}) {
  // Single-column layouts open the lecture as a dialog over the list.
  const wide = useMediaQuery("(min-width: 769px)");
  const stacked = useFormFactor() === "mobile";
  const sidebar = usePanelRef();
  const [collapsed, setCollapsed] = useState(false);
  const [floating, setFloating] = useState(false);
  const [listHovered, setListHovered] = useState(false);
  const [listFocused, setListFocused] = useState(false);
  useEffect(() => {
    if (!wide || !active || !collapsed) {
      setFloating(false);
      return;
    }
    if (railHovered || listHovered || listFocused) {
      setFloating(true);
      return;
    }
    // Bridge the gap between the rail and the floating list.
    const timer = window.setTimeout(() => setFloating(false), 200);
    return () => window.clearTimeout(timer);
  }, [wide, active, collapsed, railHovered, listHovered, listFocused]);
  const choose = (courseId: string, assignmentId?: string) => {
    setFloating(false);
    select(courseId, assignmentId);
  };

  // The quarter shown here is browsed on its own; it starts at the current one.
  const [shownTerm, setShownTerm] = useState(term);
  const lastTerm = useRef(term);
  useEffect(() => {
    if (lastTerm.current !== term) {
      lastTerm.current = term;
      setShownTerm(term);
    }
  }, [term]);
  const [query, setQuery] = useState("");
  // The list shows the quarter's lectures, or its open assignments.
  const [mode, setMode] = useState<"lectures" | "assignments">("lectures");
  const [showHidden, setShowHidden] = useState(false);
  const nowIso = useClock(active);
  const now = useMemo(() => new Date(nowIso), [nowIso]);
  const today = jstDate(now);
  const snapshot = useScheduleQueries<[ScheduleView, TaskView]>([
    {
      kind: "scheduleView",
      term: shownTerm,
      from: today,
      to: addDays(today, 7),
      includePersonal: true,
    },
    { kind: "taskView", now: nowIso },
  ]).value;
  const schedule = snapshot?.[0];
  const taskView = snapshot?.[1];
  const terms = schedule?.allTerms.length ? schedule.allTerms : [term];
  const courses = schedule?.courses ?? [];
  const taskTerm = taskView?.byTerm?.[shownTerm];
  const byAssignment = new Map(data.assignments.map((item) => [item.id, item]));

  const lectures = useMemo(() => {
    const minutes = jstMinutes(now);
    const clock = `${String(Math.floor(minutes / 60)).padStart(2, "0")}:${String(minutes % 60).padStart(2, "0")}`;
    const next = new Map<
      string,
      { rank: number; when: string; room?: string }
    >();
    for (const [offset, agenda] of (schedule?.agendas ?? []).entries()) {
      const date = agenda.date;
      for (const block of agenda.occurrences) {
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
    return courses
      .map((course): Lecture => {
        const meeting = next.get(course.id);
        const workId = taskView?.nearestByCourse[course.id];
        const due = workId ? taskView?.deadlines?.[workId] : undefined;
        return {
          id: course.id,
          title: course.title,
          code: course.code,
          color: course.color,
          rank: meeting?.rank ?? Infinity,
          when: meeting?.when,
          room: meeting?.room,
          due: due
            ? {
                label: deadlineText(due, data.settings.locale).countdownLabel,
                tone: due.countdownTone,
              }
            : undefined,
        };
      })
      .sort(
        (left, right) =>
          left.rank - right.rank || left.title.localeCompare(right.title),
      );
  }, [courses, now, schedule?.agendas, taskView, today]);
  // The task view answers for a new quarter before that quarter's courses
  // load, so work whose course is not on screen yet waits for it.
  const work = useMemo(() => {
    const byId = new Map(courses.map((course) => [course.id, course]));
    return (taskTerm?.lectureOpenIds ?? [])
      .flatMap((id) => byAssignment.get(id) ?? [])
      .flatMap((a) => {
        const course = byId.get(a.courseId);
        if (!course) return [];
        const deadline = taskView?.deadlines?.[a.id];
        return {
          assignment: a,
          course,
          due: deadline
            ? {
                label: deadlineText(deadline, data.settings.locale)
                  .countdownLabel,
                tone: deadline.countdownTone,
              }
            : undefined,
        };
      });
  }, [byAssignment, courses, taskTerm, taskView]);
  // Hidden assignments stay with their quarter, folded under the open ones.
  const hiddenWork = useMemo(() => {
    const byId = new Map(courses.map((course) => [course.id, course]));
    return (taskTerm?.lectureHiddenIds ?? [])
      .flatMap((id) => byAssignment.get(id) ?? [])
      .flatMap((a) => {
        const course = byId.get(a.courseId);
        return course ? [{ assignment: a, course }] : [];
      });
  }, [byAssignment, courses, taskTerm]);
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

  const sidebarToggle = wide ? (
    <button
      type="button"
      className="lectureSidebarToggle"
      aria-label={t("Toggle sidebar")}
      aria-expanded={!collapsed}
      aria-controls="lecture-sidebar"
      title={t("Toggle sidebar")}
      onClick={() => {
        setFloating(false);
        if (sidebar.current?.isCollapsed()) sidebar.current.expand();
        else sidebar.current?.collapse();
      }}
    >
      <svg
        width="18"
        height="18"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        aria-hidden="true"
      >
        <rect x="3" y="4" width="18" height="16" rx="3" />
        <path d="M9 4v16" />
      </svg>
    </button>
  ) : undefined;

  return (
    <div
      className={`page lectureDesk${entry ? " open" : ""}`}
      data-sidebar-collapsed={wide && collapsed}
      data-sidebar-floating={wide && collapsed && floating}
      onKeyDown={(event) => {
        if (event.key === "Escape" && floating) {
          setFloating(false);
          setListFocused(false);
        }
      }}
    >
      <ResizableWorkspace
        id="lectures"
        enabled={wide}
        firstSize={220}
        firstCollapsible
        firstPanelRef={sidebar}
        onFirstCollapse={setCollapsed}
        firstMin={170}
        firstMax="45%"
        secondMin="45%"
      >
        <nav
          id="lecture-sidebar"
          className="lectureList"
          aria-label={t("Lectures")}
          inert={wide && collapsed && !floating}
          onPointerEnter={() => setListHovered(true)}
          onPointerLeave={() => setListHovered(false)}
          onFocusCapture={() => setListFocused(true)}
          onBlurCapture={(event) => {
            if (!event.currentTarget.contains(event.relatedTarget))
              setListFocused(false);
          }}
        >
          <header>
            <div className="lectureListTitle">
              {wide && collapsed && floating && sidebarToggle}
              <h1>{t("Lecture Desk")}</h1>
            </div>
            <UiSelect
              className="lectureTerm"
              aria-label={t("Quarter")}
              value={shownTerm}
              onChange={(value) => setShownTerm(value as TermId)}
              data={[...new Set(terms.map((item) => item.slice(0, 4)))].map(
                (year) => ({
                  group: year,
                  items: terms
                    .filter((item) => item.startsWith(`${year}-`))
                    .map((item) => ({
                      value: item,
                      label: item.replace("-", " "),
                    })),
                }),
              )}
            />
          </header>
          <div
            className="dueScope lectureMode"
            role="group"
            aria-label={t("Show")}
          >
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
                aria-current={
                  entry?.assignmentId === assignment.id ? "true" : undefined
                }
                style={{ "--course": course.color } as React.CSSProperties}
                onClick={() => choose(course.id, assignment.id)}
              >
                <b>{assignment.title}</b>
                <small>{course.title}</small>
                {due && (
                  <span className={`dueChip ${due.tone}`}>{due.label}</span>
                )}
              </button>
            ))}
          {mode === "assignments" && !shownWork.length && (
            <p className="quietEmpty compact">
              {t(
                work.length
                  ? "No matching assignments."
                  : "No open assignments.",
              )}
            </p>
          )}
          {mode === "assignments" &&
            restoreAssignment &&
            shownHidden.length > 0 && (
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
                      style={
                        { "--course": course.color } as React.CSSProperties
                      }
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
          {mode === "lectures" &&
            shown.map((lecture) => (
              <button
                type="button"
                key={lecture.id}
                className={`lectureRow${entry?.courseId === lecture.id ? " selected" : ""}${lecture.rank === 0 ? " now" : ""}`}
                aria-current={
                  entry?.courseId === lecture.id ? "true" : undefined
                }
                style={{ "--course": lecture.color } as React.CSSProperties}
                onClick={() => choose(lecture.id)}
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
        <div
          className="lectureContent"
          role={stacked && entry ? "dialog" : undefined}
          aria-modal={stacked && entry ? true : undefined}
        >
          {!entry && wide && (
            <header className="lectureEmptyHeader">{sidebarToggle}</header>
          )}
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
              <Activity
                mode={
                  active && entry?.courseId === item.courseId
                    ? "visible"
                    : "hidden"
                }
              >
                {renderWorkspace(
                  item,
                  active && entry?.courseId === item.courseId,
                  entry?.courseId === item.courseId ? sidebarToggle : undefined,
                )}
              </Activity>
            </div>
          ))}
        </div>
      </ResizableWorkspace>
    </div>
  );
}
