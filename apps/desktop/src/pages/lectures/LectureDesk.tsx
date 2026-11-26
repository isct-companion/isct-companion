import { dayShort } from "../../components/timetable/timetable";
import { TitleRow, TitlebarNav } from "../../components/TitleRow";
import { BookOpen } from "lucide-react";
import { DueChip } from "../../components/DueChip";
import { CloseButton, Input, EmptyState, Skeleton } from "../../components/ui";
import "./lectures.css";
import {
  ResizableWorkspace,
  WorkspaceSidebarToggle,
} from "../../components/ResizableWorkspace";
import { KeptPage } from "../../app/KeptPage";
import { usePanelRef } from "react-resizable-panels";
import { useEffect, useRef, useState, type ReactNode } from "react";
import type { TermId, Workspace } from "../../lib/model";
import {
  addCivilDays as addDays,
  civilWeekday,
  tokyoDate,
} from "../../lib/civil-date";
import { UiSelect } from "../../components/ui";
import { t } from "../../i18n";
import type { CourseEntry } from "../../features/lms";
import { useClock } from "../../platform/use-clock";
import {
  useScheduleQueries,
  useScheduleQuery,
  type LectureList,
  type TaskView,
} from "../../platform/core-query";
import { withRecords } from "../../platform/records";
import { LectureAssignments } from "./LectureAssignments";
import { LectureExamList, LectureExamDetails } from "./LectureExams";
import type { LectureMeeting } from "../../generated/persistence";

function meetingLabel(
  meeting: LectureMeeting,
  today: string,
  tomorrow: string,
) {
  if (meeting.inClass) return t("common.now");
  const day =
    meeting.date === today
      ? "common.today"
      : meeting.date === tomorrow
        ? "common.tomorrow"
        : dayShort[civilWeekday(meeting.date) - 1];
  return `${t(day)} ${meeting.startTime.replace(/^0/, "")}`;
}

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

function LectureDeskBody({
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
    chosenHere.current = true;
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
  // A course or exam chosen from this list enters its pane; one opened from
  // another page arrives with that page's transition instead.
  const chosenHere = useRef(false);
  const takeChoice = () => {
    const here = chosenHere.current;
    chosenHere.current = false;
    return here;
  };
  const [entering, setEntering] = useState<string | null>(null);
  const endEntrance = (event: React.AnimationEvent) => {
    if (event.target === event.currentTarget) setEntering(null);
  };
  // Only a new navigation selects its mode and quarter.
  useEntrance(entry?.request, () => {
    if (!entry) return;
    if (takeChoice()) setEntering(entry.courseId);
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
    if (takeChoice()) setEntering(`exam:${examEntry.key}`);
  });
  const nowIso = useClock(active);
  const today = tokyoDate(new Date(nowIso));
  const tomorrow = addDays(today, 1);
  const snapshotQuery = useScheduleQueries<[LectureList, TaskView]>([
    { kind: "lectureList", term: shownTerm, now: nowIso },
    { kind: "taskView", now: nowIso },
  ]);
  const snapshot = snapshotQuery.value;
  // Nothing read yet: the list draws placeholders, not "no lectures".
  const listLoading = snapshotQuery.pending && !snapshot;
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

  const needle = query.trim().toLowerCase();
  const shown = needle
    ? courses.filter((course) => search?.courseIds.includes(course.id))
    : courses;

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
    <WorkspaceSidebarToggle
      panel={sidebar}
      collapsed={collapsed}
      controls="lecture-sidebar"
      onToggle={() => setFloating(false)}
    />
  );
  // With the list folded away the content column is first after the rail,
  // so its title row starts with Back, Forward and the sidebar button.
  const contentStart = collapsed ? (
    <TitlebarNav sidebar={sidebarToggle} />
  ) : undefined;

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
          aria-label={t("common.lectures")}
          inert={collapsed && !floating}
          onPointerEnter={() => setListHovered(true)}
          onPointerLeave={() => setListHovered(false)}
          onFocusCapture={() => setListFocused(true)}
          onBlurCapture={(event) => {
            if (!event.currentTarget.contains(event.relatedTarget))
              setListFocused(false);
          }}
        >
          <TitleRow
            nav
            sidebar={!collapsed || floating ? sidebarToggle : undefined}
            className="lectureListHeader"
          >
            <h1>{t("lectures.lectureDesk")}</h1>
          </TitleRow>
          <UiSelect
            className="lectureTerm"
            aria-label={t("common.quarter")}
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
          <div
            className="dueScope lectureMode"
            role="group"
            aria-label={t("lectures.show")}
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
                    ? "common.lectures"
                    : item === "exams"
                      ? "common.exams"
                      : "common.assignments",
                )}
              </button>
            ))}
          </div>
          <Input
            className="lectureFilter"
            type="search"
            aria-label={t("common.filter")}
            placeholder={t("lectures.filter")}
            value={query}
            rightSectionPointerEvents="all"
            rightSection={
              query ? (
                <CloseButton
                  aria-label={t("common.clear")}
                  onClick={() => setQuery("")}
                  size="sm"
                />
              ) : null
            }
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => event.key === "Escape" && setQuery("")}
          />
          {mode === "exams" && (
            <LectureExamList
              schedule={schedule}
              query={query}
              search={search}
              examKey={examKey}
              chooseExam={(key, term) => {
                setQuery("");
                setFloating(false);
                chosenHere.current = true;
                selectExam(key, term);
              }}
            />
          )}
          {mode === "assignments" && (
            <LectureAssignments
              data={data}
              taskView={taskView}
              shownTerm={shownTerm}
              search={search}
              needle={needle}
              entry={entry}
              choose={choose}
              restoreAssignment={restoreAssignment}
            />
          )}
          {mode === "lectures" && listLoading && <Skeleton count={5} />}
          {mode === "lectures" &&
            !listLoading &&
            shown.map((lecture) => {
              const meeting = schedule?.nextByCourse[lecture.id];
              const workId = taskView?.nearestByCourse[lecture.id];
              const due = workId ? taskView?.deadlines[workId] : undefined;
              return (
                <button
                  type="button"
                  key={lecture.id}
                  className={`lectureRow${entry?.courseId === lecture.id ? " selected" : ""}${meeting?.inClass ? " now" : ""}`}
                  aria-current={
                    entry?.courseId === lecture.id ? "true" : undefined
                  }
                  style={{ "--course": lecture.color } as React.CSSProperties}
                  onClick={() => choose(lecture.id)}
                >
                  <b>{lecture.title}</b>
                  <small>
                    {[
                      lecture.code,
                      meeting && meetingLabel(meeting, today, tomorrow),
                      meeting?.room,
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </small>
                  {due && <DueChip due={due} locale={data.settings.locale} />}
                </button>
              );
            })}
          {mode === "lectures" && !listLoading && !shown.length && (
            <EmptyState compact icon={BookOpen}>
              {t(
                courses.length
                  ? "lectures.noMatchingLectures"
                  : "lectures.noLecturesOnThisQuarters",
              )}
            </EmptyState>
          )}
        </nav>
        <div className="lectureContent">
          {mode === "exams" && (
            <LectureExamDetails
              selectedExam={selectedExam}
              entering={entering}
              endEntrance={endEntrance}
              sidebarToggle={contentStart}
              onRemoved={() => setExamKey(null)}
            />
          )}
          <div className="lectureCourseContent" hidden={mode === "exams"}>
            {!entry && (
              <TitleRow className="lectureEmptyHeader">{contentStart}</TitleRow>
            )}
            {/* The list says when there is nothing to choose. */}
            {!entry && shown.length > 0 && (
              <EmptyState icon={BookOpen}>
                {t("common.chooseACourse")}
              </EmptyState>
            )}
            {retained.map((item) => (
              <div
                className={`lectureSlot${entering === item.courseId ? " entering" : ""}`}
                key={item.courseId}
                hidden={entry?.courseId !== item.courseId}
                onAnimationEnd={endEntrance}
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
                      ? contentStart
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

/** LectureDesk with the records it shows (`platform/records`). */
export const LectureDesk = withRecords(
  [
    "assignments",
    "courses",
    "materials",
    "memberships",
    "planItems",
    "academicProfile",
    "lmsCourses",
  ],
  LectureDeskBody,
);
