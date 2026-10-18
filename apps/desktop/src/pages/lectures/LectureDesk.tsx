import { ExamEditor } from "../../components/exams/ExamEditor";
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
import type { Assignment, TermId, Workspace } from "../../lib/model";
import { submissionLabel } from "../../lib/academic";
import { AssignmentGroupList } from "../../components/assignments/AssignmentGroupList";
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
  assignmentsRequest,
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
  /** Changes when Today asks for the current quarter's assignment list. */
  assignmentsRequest?: number;
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
  const [examKey, setExamKey] = useState<string | null>(null);
  // The list shows the quarter's lectures, or all of its assignments.
  const [mode, setMode] = useState<"lectures" | "assignments" | "exams">(
    "lectures",
  );
  useEffect(() => {
    if (!assignmentsRequest) return;
    setMode("assignments");
    setShownTerm(term);
    // Only a new request switches; a quarter change keeps the chosen mode.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [assignmentsRequest]);
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
  const selectedExam =
    mode === "exams"
      ? schedule?.exams?.find((exam) => exam.key === examKey)
      : undefined;
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
  const needle = query.trim().toLowerCase();
  const courseById = new Map(data.courses.map((course) => [course.id, course]));
  const submittedIds = new Set(taskView?.submittedIds);
  const matchesWork = (assignment: Assignment) => {
    const course = courseById.get(assignment.courseId);
    return `${assignment.title} ${course?.title ?? ""} ${course?.code ?? ""}`
      .toLowerCase()
      .includes(needle);
  };
  const workRow = (assignment: Assignment) => {
    const course = courseById.get(assignment.courseId);
    const due = taskView?.deadlines?.[assignment.id];
    const submitted = submittedIds.has(assignment.id);
    // The same states as Today: submitted early, missed, or closed.
    const state = due?.overdue
      ? submitted
        ? " closed"
        : " missed"
      : submitted
        ? " done"
        : "";
    return (
      <button
        type="button"
        key={assignment.id}
        className={`lectureRow${state}${entry?.assignmentId === assignment.id ? " selected" : ""}`}
        aria-current={
          entry?.assignmentId === assignment.id ? "true" : undefined
        }
        style={{ "--course": course?.color } as React.CSSProperties}
        onClick={() => choose(assignment.courseId, assignment.id)}
      >
        <b>{assignment.title}</b>
        <small>
          {course?.title ?? "LMS"} · {submissionLabel(assignment, t, submitted)}
        </small>
        {due &&
          (!due.overdue && !submitted ? (
            <span className={`dueChip ${due.countdownTone}`}>
              {deadlineText(due, data.settings.locale).countdownLabel}
            </span>
          ) : (
            <span className="dueChip past">
              {deadlineText(due, data.settings.locale).label}
            </span>
          ))}
      </button>
    );
  };
  const hiddenRow = (assignment: Assignment) => {
    const course = courseById.get(assignment.courseId);
    return (
      <div
        key={assignment.id}
        className="lectureRow hidden"
        style={{ "--course": course?.color } as React.CSSProperties}
      >
        <b>{assignment.title}</b>
        <small>{course?.title ?? "LMS"}</small>
        <button
          type="button"
          className="textButton"
          onClick={() => restoreAssignment?.(assignment.id)}
        >
          {t("Restore")}
        </button>
      </div>
    );
  };
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
      className={`page lectureDesk${(mode === "exams" ? selectedExam : entry) ? " open" : ""}`}
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
        firstMin={210}
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
            {(["lectures", "assignments", "exams"] as const).map((item) => (
              <button
                type="button"
                key={item}
                aria-pressed={mode === item}
                onClick={() => setMode(item)}
              >
                {t(
                  item === "lectures"
                    ? "Lectures"
                    : item === "exams"
                      ? "Exams"
                      : "Assignments",
                )}
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
          {mode === "exams" &&
            (() => {
              const exams = schedule?.exams ?? [];
              const needle = query.trim().toLowerCase();
              const shown = exams.filter((exam) =>
                needle
                  ? `${exam.title} ${exam.code} ${exam.room}`
                      .toLowerCase()
                      .includes(needle)
                  : Boolean(exam.courseId),
              );
              return (
                <>
                  <div className="lectureExamScope">
                    <small>
                      {needle ? t("Search published exams") : t("My exams")}
                    </small>
                    {!shown.length && (
                      <p>
                        {t(
                          needle
                            ? "No matching exams."
                            : "No linked exams this quarter. Search to find a published exam.",
                        )}
                      </p>
                    )}
                  </div>
                  {shown.map((exam) => (
                    <button
                      type="button"
                      key={exam.key}
                      className={`lectureRow exam${examKey === exam.key ? " selected" : ""}`}
                      aria-current={examKey === exam.key ? "true" : undefined}
                      style={{ "--course": exam.color } as React.CSSProperties}
                      onClick={() => {
                        setExamKey(exam.key);
                        setFloating(false);
                      }}
                    >
                      <b>{exam.title || exam.code}</b>
                      <small>
                        {exam.code} · {exam.date}
                      </small>
                      <small>
                        {exam.courseId ? t("Linked") : t("Not linked")}
                        {exam.status === "cancelled"
                          ? ` · ${t("Cancelled")}`
                          : ""}
                        {exam.hidden ? ` · ${t("Hidden")}` : ""}
                      </small>
                    </button>
                  ))}
                </>
              );
            })()}
          {mode === "assignments" && (
            <AssignmentGroupList
              key={shownTerm}
              groups={taskTerm?.record}
              assignments={byAssignment}
              row={workRow}
              hiddenRow={restoreAssignment ? hiddenRow : undefined}
              matches={needle ? matchesWork : undefined}
              pastLabel={t("Past deadline")}
              empty={t(
                needle
                  ? "No matching assignments."
                  : "No assignments this quarter.",
              )}
            />
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
          role={
            stacked && (mode === "exams" ? selectedExam : entry)
              ? "dialog"
              : undefined
          }
          aria-modal={
            stacked && (mode === "exams" ? selectedExam : entry)
              ? true
              : undefined
          }
        >
          {mode === "exams" && (
            <div className="lectureExamDetails">
              {sidebarToggle}
              <button
                type="button"
                className="lectureBack"
                onClick={() => setExamKey(null)}
              >
                ‹ {t("Exams")}
              </button>
              {selectedExam ? (
                <>
                  <header>
                    <h1>{selectedExam.title || selectedExam.code}</h1>
                    <p>
                      {selectedExam.code} ·{" "}
                      {selectedExam.term.replace("-", " ")} · {t("Exam")}
                    </p>
                  </header>
                  <ExamEditor
                    key={`${selectedExam.key}:${data.scheduleOverrides.find((row) => !row.deletedAt && row.targetKey === selectedExam.key)?.rev ?? 0}`}
                    exam={selectedExam}
                    data={data}
                    courses={courses}
                  />
                </>
              ) : (
                <p className="emptyHint">
                  {t("Select an exam to edit its schedule or link a course.")}
                </p>
              )}
            </div>
          )}
          <div className="lectureCourseContent" hidden={mode === "exams"}>
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
                    active &&
                    mode !== "exams" &&
                    entry?.courseId === item.courseId
                      ? "visible"
                      : "hidden"
                  }
                >
                  {renderWorkspace(
                    item,
                    active &&
                      mode !== "exams" &&
                      entry?.courseId === item.courseId,
                    entry?.courseId === item.courseId
                      ? sidebarToggle
                      : undefined,
                  )}
                </Activity>
              </div>
            ))}
          </div>
        </div>
      </ResizableWorkspace>
    </div>
  );
}
