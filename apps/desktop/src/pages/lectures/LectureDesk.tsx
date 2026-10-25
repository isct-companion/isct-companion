import {
  examConflictLabel,
  examKindLabel,
} from "../../components/exams/ExamCards";
import { ExamEditor } from "../../components/exams/ExamEditor";
import { DueChip } from "../../components/DueChip";
import { CloseButton, Input } from "../../components/ui";
import { deadlineText } from "../../lib/deadline-text";
import "./lectures.css";
import { ResizableWorkspace } from "../../components/ResizableWorkspace";
import { KeptPage } from "../../app/KeptPage";
import { usePanelRef } from "react-resizable-panels";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { Assignment, TermId, Workspace } from "../../lib/model";
import { submissionLabel } from "../../lib/academic";
import { AssignmentGroupList } from "../../components/assignments/AssignmentGroupList";
import {
  addCivilDays as addDays,
  civilWeekday,
  jstMinutes,
  tokyoDate,
} from "../../lib/civil-date";
import { UiSelect } from "../../components/ui";
import { t } from "../../i18n";
import type { CourseEntry } from "../../features/lms/LmsWorkspace";
import { useClock } from "../../platform/use-clock";
import {
  useScheduleQueries,
  useScheduleQuery,
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
/**
 * Applies each entrance once. Revealing the retained Activity re-runs effects,
 * which must resume the page as it was rather than replay the entrance.
 */
function useEntrance(request: number | undefined, apply: () => void) {
  const applied = useRef<number | undefined>(undefined);
  useEffect(() => {
    if (request === undefined || applied.current === request) return;
    applied.current = request;
    apply();
    // Only a new request applies; the callback reads that render's props.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [request]);
}

export function LectureDesk({
  data,
  term,
  entry,
  examEntry,
  select,
  selectExam,
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
  examEntry: { key: string; term: TermId; request: number } | null;
  select: (courseId: string, assignmentId?: string, term?: TermId) => void;
  selectExam: (key: string, term: string) => void;
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
  const sidebar = usePanelRef();
  const [collapsed, setCollapsed] = useState(false);
  const [floating, setFloating] = useState(false);
  const [listHovered, setListHovered] = useState(false);
  const [listFocused, setListFocused] = useState(false);
  useEffect(() => {
    if (!active || !collapsed) {
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
  }, [active, collapsed, railHovered, listHovered, listFocused]);
  const choose = (courseId: string, assignmentId?: string) => {
    setFloating(false);
    setQuery("");
    select(courseId, assignmentId, shownTerm);
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
  // Only a new request switches; a quarter change keeps the chosen mode.
  useEntrance(assignmentsRequest || undefined, () => {
    setMode("assignments");
    setQuery("");
    setShownTerm(term);
  });
  // Only a new navigation selects its mode and quarter.
  useEntrance(entry?.request, () => {
    if (!entry) return;
    setMode(entry.assignmentId ? "assignments" : "lectures");
    setQuery("");
    const course = data.courses.find((course) => course.id === entry.courseId);
    const target = entry.term ?? course?.termId;
    if (target) setShownTerm(target as TermId);
  });
  useEntrance(examEntry?.request, () => {
    if (!examEntry) return;
    setMode("exams");
    setQuery("");
    setExamKey(examEntry.key);
    setShownTerm(examEntry.term);
  });
  const nowIso = useClock(active);
  const now = useMemo(() => new Date(nowIso), [nowIso]);
  const today = tokyoDate(now);
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
  const search = useScheduleQuery<
    import("../../generated/persistence").LectureSearch
  >(
    { kind: "lectureSearch", term: shownTerm, query },
    Boolean(query.trim()),
  ).value;
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
    return Boolean(search?.assignmentIds.includes(assignment.id));
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
        {due && (
          <DueChip
            due={due}
            submitted={submitted}
            locale={data.settings.locale}
          />
        )}
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
    ? lectures.filter((lecture) => search?.courseIds.includes(lecture.id))
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

  const sidebarToggle = (
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
  );

  return (
    <div
      className="page lectureDesk"
      data-sidebar-collapsed={collapsed}
      data-sidebar-floating={collapsed && floating}
      onKeyDown={(event) => {
        if (event.key === "Escape" && floating) {
          setFloating(false);
          setListFocused(false);
        }
      }}
    >
      <ResizableWorkspace
        id="lectures"
        enabled
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
          inert={collapsed && !floating}
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
              {collapsed && floating && sidebarToggle}
              <h1>{t("Lecture Desk")}</h1>
            </div>
            <UiSelect
              className="lectureTerm"
              aria-label={t("Quarter")}
              value={shownTerm}
              onChange={(value) => {
                setQuery("");
                setShownTerm(value as TermId);
              }}
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
                onClick={() => {
                  setMode(item);
                  setQuery("");
                }}
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
          <Input
            className="lectureFilter"
            type="search"
            aria-label={t("Filter")}
            placeholder={t("Filter…")}
            value={query}
            rightSectionPointerEvents="all"
            rightSection={
              query ? (
                <CloseButton
                  aria-label={t("Clear")}
                  onClick={() => setQuery("")}
                  size="sm"
                />
              ) : null
            }
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => event.key === "Escape" && setQuery("")}
          />
          {mode === "exams" &&
            (() => {
              const exams = schedule?.exams ?? [];
              const needle = query.trim().toLowerCase();
              const shown = exams.filter((exam) =>
                needle
                  ? Boolean(search?.examKeys.includes(exam.key))
                  : Boolean(exam.courseId) || exam.key === examKey,
              );
              // Classes the timetable lists in exam week are not exams.
              const classes = shown.filter((exam) => exam.kind === "class");
              const row = (exam: (typeof shown)[number]) => (
                <button
                  type="button"
                  key={exam.key}
                  className={`lectureRow exam${exam.kind === "class" ? " examWeekClass" : ""}${examKey === exam.key ? " selected" : ""}`}
                  aria-current={examKey === exam.key ? "true" : undefined}
                  style={{ "--course": exam.color } as React.CSSProperties}
                  onClick={() => {
                    setQuery("");
                    setFloating(false);
                    selectExam(exam.key, exam.term);
                  }}
                >
                  <b>{exam.title || exam.code}</b>
                  <small>
                    {exam.code} · {exam.date}
                    {exam.startTime && exam.endTime
                      ? ` · ${exam.startTime}–${exam.endTime}`
                      : ""}
                  </small>
                  <small>
                    {exam.kind === "quiz" ? `${t("Quiz")} · ` : ""}
                    {exam.origin === "session"
                      ? t("Marked on the calendar")
                      : exam.courseId
                        ? t("Linked")
                        : t("Not linked")}
                    {exam.status === "cancelled" ? ` · ${t("Cancelled")}` : ""}
                  </small>
                  {exam.conflict && (
                    <small className="examConflictTag">
                      {examConflictLabel(exam)}
                    </small>
                  )}
                </button>
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
                  {shown.filter((exam) => exam.kind !== "class").map(row)}
                  {classes.length > 0 && (
                    <div className="lectureExamScope">
                      <small>{t("Classes in exam week")}</small>
                    </div>
                  )}
                  {classes.map(row)}
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
                  {[lecture.code, lecture.when, lecture.room]
                    .filter(Boolean)
                    .join(" · ")}
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
          {mode === "exams" && (
            <div className="lectureExamDetails">
              {sidebarToggle}
              {selectedExam ? (
                <>
                  <header>
                    <h1>{selectedExam.title || selectedExam.code}</h1>
                    <p>
                      {selectedExam.code} ·{" "}
                      {selectedExam.term.replace("-", " ")} ·{" "}
                      {examKindLabel(selectedExam)}
                    </p>
                  </header>
                  <ExamEditor
                    key={selectedExam.key}
                    exam={selectedExam}
                    courses={courses}
                    onRemoved={() => setExamKey(null)}
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
            {!entry && (
              <header className="lectureEmptyHeader">{sidebarToggle}</header>
            )}
            {retained.map((item) => (
              <div
                className="lectureSlot"
                key={item.courseId}
                hidden={entry?.courseId !== item.courseId}
              >
                {/* Its own overlay layer, so a course's popovers hide with it
                    when the exam list or another course takes the pane. */}
                <KeptPage
                  visible={
                    active &&
                    mode !== "exams" &&
                    entry?.courseId === item.courseId
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
                </KeptPage>
              </div>
            ))}
          </div>
        </div>
      </ResizableWorkspace>
    </div>
  );
}
