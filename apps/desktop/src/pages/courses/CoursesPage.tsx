import { ExamEditor } from "../../components/exams/ExamEditor";
import { ResizableWorkspace } from "../../components/ResizableWorkspace";
import { t } from "../../i18n";
import { useRef, useState, type ReactNode } from "react";
import { ActionIcon, Badge, Button, Menu, Paper, Modal } from "@mantine/core";
import {
  useMediaQuery,
  useLocalStorage,
  useDebouncedCallback,
} from "@mantine/hooks";

import {
  dayNames,
  WeekTimetable,
  type Block,
} from "../../components/timetable/timetable";

import type { PersonalEvent, Plan, TermId, Workspace } from "../../lib/model";

import { planningColumns } from "../../components/timetable/workspace";
import { tokyoDate } from "../../lib/civil-date";
import type { CalendarExportScope } from "../../generated/persistence";
import { UiButton } from "../../components/mantine";
import { YearQuarterPicker } from "./YearQuarterPicker";
import { PullRing } from "../../components/PullRing";
import type { WorkspaceIntent } from "../../lib/workspace-intents";
import {
  useScheduleQueries,
  useScheduleQuery,
  type CourseDetail,
  type PlanningWeek,
  type WorkspaceOverview,
} from "../../platform/core-query";

/**
 * Timetable on the left, syllabus search on the right. Clicking a course opens
 * its details over the search (`panel`); removing happens there, so a stray
 * click on the timetable never deletes anything.
 */
export function Courses({
  data,
  currentTerm,
  term,
  overview,
  cachedPlanning,
  onSelectTerm,
  readOnly,
  mutate,
  open,
  openEvent,
  addTerm,
  exportIcs,
  side,
  panel,
  leftPanel,
  onSlot,
  activeSlots,
  onIntensive,
  intensiveActive,
  refreshSchedule,
  pullProgress = null,
  register,
  academic,
  editing = false,
  onEditingChange,
  openPlan,
  newPlan,
  compact = false,
}: {
  editing?: boolean;
  onEditingChange?: (value: boolean) => void;
  openPlan?: (plan: Plan) => void;
  newPlan?: () => void;
  data: Workspace;
  currentTerm: TermId;
  term: TermId;
  overview: WorkspaceOverview;
  cachedPlanning?: PlanningWeek;
  onSelectTerm: (term: TermId) => void;
  readOnly: boolean;
  mutate: (intent: WorkspaceIntent) => Promise<unknown>;
  /** Shows a course's details over the search panel. */
  open: (courseId: string) => void;
  openEvent: (event: PersonalEvent | undefined, term: TermId) => void;
  addTerm: () => void;
  exportIcs: (term: TermId, scope?: CalendarExportScope) => void;
  /** The syllabus search panel. */
  side?: ReactNode;
  /** Course details covering the search panel while open. */
  panel?: ReactNode;
  /** A search result's details covering the timetable while open. */
  leftPanel?: ReactNode;
  /** A free slot was clicked; `append` when ⌘ (macOS) or Ctrl was held. */
  onSlot?: (day: number, from: number, to: number, append: boolean) => void;
  activeSlots?: ReadonlySet<string>;
  /** The 集中 lane toggles the intensive condition. */
  onIntensive?: (append: boolean) => void;
  intensiveActive?: boolean;
  /** Refreshes official enrollment and times from 教務Web. */
  refreshSchedule?: () => void;
  /** 0–100 while an official pull runs, else null. */
  pullProgress?: number | null;
  /** Opens 教務Web registration with this quarter's planned courses queued. */
  register?: (term: TermId) => void;
  /** GPA / credits summary opened from the top toolbar. */
  academic?: ReactNode;
  /** Mobile: one header row, viewing only; extras move into a menu. */
  compact?: boolean;
}) {
  const lastQuarterClick = useRef(-Infinity);
  const commitTerm = useDebouncedCallback(
    (termId: TermId) => {
      void mutate({ kind: "selectTerm", termId }).catch(() => undefined);
    },
    { delay: 100, flushOnUnmount: true },
  );
  const selectTerm = (termId: TermId) => {
    onSelectTerm(termId);
    const now = performance.now();
    const first = now - lastQuarterClick.current >= 100;
    lastQuarterClick.current = now;
    commitTerm(termId);
    // Mantine's leading mode drops the trailing click; flush only the first
    // click and let its normal debounce retain the final click in a burst.
    if (first) commitTerm.flush();
  };
  const canEdit = editing && !readOnly;
  const wide = useMediaQuery("(min-width: 761px)");
  const terms = overview?.allTerms ?? [currentTerm];
  const planningQuery = useScheduleQueries<[PlanningWeek]>(
    [{ kind: "planningWeek", term }],
    !cachedPlanning,
  );
  const planning =
    cachedPlanning ??
    (planningQuery.pending ? undefined : planningQuery.value?.[0]);
  // Courses of this quarter, a 3Q–4Q course in each of its quarters.
  const courseCount = overview?.termCourseIds.length ?? 0;
  const events = data.events.filter((e) => e.termId === term && !e.deletedAt);
  const scheduled = planning?.courses ?? [];
  // Intensive or unpublished times have no grid cell; they get their own lane.
  const unscheduled = scheduled.filter((course) => !course.meetings.length);
  const conflictCount = editing ? (planning?.conflictCount ?? 0) : 0;
  const [weekLength, setWeekLength] = useLocalStorage<"5" | "7">({
    key: "isct-companion-courses-week-length",
    defaultValue: "5",
  });
  // The quarter running now marks today's weekday, which shows the now line.
  const today =
    term === currentTerm
      ? dayNames[
          (new Date(`${tokyoDate(new Date())}T00:00:00Z`).getUTCDay() + 6) % 7
        ]
      : undefined;
  const allColumns = planningColumns(planning).map((column) => ({
    ...column,
    isToday: column.label === today,
  }));
  const columns =
    weekLength === "7"
      ? allColumns
      : allColumns.slice(
          0,
          allColumns[6]?.blocks.length
            ? 7
            : allColumns[5]?.blocks.length
              ? 6
              : 5,
        );
  const removable =
    !courseCount && !events.length && (data.terms ?? []).includes(term);
  const [preview, setPreview] = useState<Block | null>(null);
  const examDetail = useScheduleQuery<CourseDetail>(
    { kind: "courseDetail", courseId: preview?.id ?? "" },
    preview?.kind === "exam",
  );
  const previewExam = examDetail.value?.exams?.find(
    (exam) => exam.key === preview?.key,
  );

  const select = (block: Block) => {
    if (!editing && block.kind !== "course") {
      setPreview(block);
      return;
    }
    if (block.kind === "course") open(block.id);
    else if (block.kind === "plan") {
      const plan = data.plans.find(
        (item) => item.id === block.id && !item.deletedAt,
      );
      if (plan) openPlan?.(plan);
    } else {
      const event = data.events.find((e) => !e.deletedAt && e.id === block.id);
      if (event) openEvent(event, event.termId);
    }
  };
  const exportMenu = (target: ReactNode) => (
    <Menu position="bottom-end" withinPortal>
      <Menu.Target>{target}</Menu.Target>
      <Menu.Dropdown>
        <Menu.Item onClick={() => exportIcs(term, "courses")}>
          {t("Courses only")}
        </Menu.Item>
        <Menu.Item onClick={() => exportIcs(term, "courses-and-plans")}>
          {t("Courses + personal plans")}
        </Menu.Item>
      </Menu.Dropdown>
    </Menu>
  );
  const header = compact ? (
    <header className="scheduleCompactBar">
      <h1>{t("Timetable")}</h1>
      <YearQuarterPicker
        terms={terms}
        selected={term}
        current={currentTerm}
        onSelect={selectTerm}
      />
      {exportMenu(
        <ActionIcon
          variant="subtle"
          color="gray"
          size={36}
          aria-label={t("Calendar export options")}
        >
          ⋯
        </ActionIcon>,
      )}
    </header>
  ) : null;
  return (
    <div
      className={`page unifiedSchedule${editing ? " editing" : ""}${compact ? " compact" : ""}`}
    >
      {header}
      <div className="scheduleTitleRow" hidden={compact}>
        <div>
          <h1>{t("Timetable")}</h1>
          <span>{t("Plan courses for this quarter")}</span>
        </div>
      </div>
      <header className="pageBar" hidden={compact}>
        <YearQuarterPicker
          terms={terms}
          selected={term}
          current={currentTerm}
          onSelect={selectTerm}
          onAdd={canEdit ? addTerm : undefined}
        />
        <div className="pageBarRight">
          {academic}
          {refreshSchedule && (
            <Button
              size="xs"
              variant="default"
              disabled={pullProgress !== null}
              leftSection={
                pullProgress !== null ? (
                  <PullRing value={pullProgress} />
                ) : undefined
              }
              onClick={refreshSchedule}
              title={t("Refresh timetable")}
            >
              {pullProgress !== null
                ? `${Math.round(pullProgress)}%`
                : t("Refresh timetable")}
            </Button>
          )}
          {register && canEdit && (
            <UiButton
              className="button"
              onClick={() => register(term)}
              title={t(
                "Open 教務Web registration with this quarter's courses queued",
              )}
            >
              {t("Register ↗")}
            </UiButton>
          )}
          {readOnly && (
            <Badge
              variant="light"
              color="gray"
              title={t("Editing is available in the desktop app")}
            >
              {t("Read only")}
            </Badge>
          )}
          {conflictCount > 0 && (
            <Badge
              component="button"
              type="button"
              variant="light"
              color="red"
              className="conflictBadge"
              title={t("Show the first conflict")}
              onClick={() => {
                const first = planning?.conflicts?.[0];
                if (first) open(first.courseId);
              }}
            >
              {t("Conflicts: {{count}}", { count: conflictCount })}
            </Badge>
          )}

          {removable && canEdit && (
            <UiButton
              className="button"
              onClick={() =>
                void mutate({ kind: "removeTerm", termId: term }).catch(
                  () => undefined,
                )
              }
            >
              {t("Remove empty quarter")}
            </UiButton>
          )}
          <Button.Group className="calendarExportActions">
            <Button
              size="xs"
              variant="default"
              onClick={() => exportIcs(term, "courses")}
            >
              {t("Export ICS")}
            </Button>
            <Menu position="bottom-end" withinPortal>
              <Menu.Target>
                <Button
                  size="xs"
                  variant="default"
                  px={6}
                  aria-label={t("Calendar export options")}
                >
                  ▾
                </Button>
              </Menu.Target>
              <Menu.Dropdown>
                <Menu.Item onClick={() => exportIcs(term, "courses")}>
                  {t("Courses only")}
                </Menu.Item>
                <Menu.Item onClick={() => exportIcs(term, "courses-and-plans")}>
                  {t("Courses + personal plans")}
                </Menu.Item>
              </Menu.Dropdown>
            </Menu>
          </Button.Group>
          {canEdit && newPlan && (
            <Button size="xs" variant="default" onClick={newPlan}>
              {t("＋ Plan")}
            </Button>
          )}
          {canEdit && (
            <UiButton
              className="button"
              onClick={() => openEvent(undefined, term)}
            >
              {t("＋ Event")}
            </UiButton>
          )}
        </div>
      </header>
      <div className={`coursesBody${editing ? "" : " viewing"}`}>
        <ResizableWorkspace
          id="planner"
          enabled={wide && editing}
          secondSize={320}
          firstMin="40%"
          secondMin={240}
          secondMax="55%"
        >
          <div className="coursesLeft">
            <Paper
              component="section"
              className="ttCard"
              withBorder
              shadow="xs"
              radius="md"
            >
              <div className="timetableViewBar">
                <b>{t("Quarter template")}</b>
              </div>
              {
                <div className="intensiveLane">
                  {canEdit && onIntensive ? (
                    <UiButton
                      type="button"
                      className={`intensiveToggle${intensiveActive ? " active" : ""}`}
                      aria-pressed={Boolean(intensiveActive)}
                      title={t("Find intensive courses")}
                      onClick={(event) =>
                        onIntensive(event.metaKey || event.ctrlKey)
                      }
                    >
                      {t("集中・時間未定")}
                    </UiButton>
                  ) : (
                    <b>{t("集中・時間未定")}</b>
                  )}
                  <div>
                    {unscheduled.map((course) => (
                      <UiButton
                        key={course.id}
                        type="button"
                        style={
                          { "--course": course.color } as React.CSSProperties
                        }
                        onClick={() => open(course.id)}
                      >
                        {course.title}
                      </UiButton>
                    ))}
                    {!unscheduled.length && <small>{t("None")}</small>}
                  </div>
                  <UiButton
                    type="button"
                    className="weekLength"
                    aria-pressed={weekLength === "7"}
                    title={t("Show the weekend")}
                    onClick={() =>
                      setWeekLength(weekLength === "7" ? "5" : "7")
                    }
                  >
                    {weekLength === "7" ? t("7 days") : t("5 days")}
                  </UiButton>
                </div>
              }
              <WeekTimetable
                days={columns}
                onSelect={select}
                emptyLabel={t("Nothing scheduled this day")}
                showNow
                onSlot={canEdit ? onSlot : undefined}
                activeSlots={editing ? activeSlots : undefined}
              />
            </Paper>
            {editing && leftPanel && (
              <div className="coursesSidePanel">{leftPanel}</div>
            )}
          </div>
          {editing && (
            <div className="coursesSide">
              <div className="coursesInspector">
                {editing && side}
                {editing && panel && (
                  <div className="coursesSidePanel">{panel}</div>
                )}
              </div>
            </div>
          )}
        </ResizableWorkspace>
      </div>
      <Modal
        opened={Boolean(preview)}
        onClose={() => setPreview(null)}
        title={preview?.title}
      >
        {preview && (
          <div className="scheduleItemPreview">
            {preview.kind === "exam" && (
              <Badge color="red" variant="light">
                {t("Exam")} · {preview.timeLabel}
              </Badge>
            )}
            <p>
              {preview.startTime}–{preview.endTime}
            </p>
            {preview.room && <p>{preview.room}</p>}
            {preview.conflictWith?.length ? (
              <p className="scheduleConflict">
                {t("Overlaps {{courses}}", {
                  courses: preview.conflictWith.join(", "),
                })}
              </p>
            ) : null}
            {!readOnly && preview.kind === "exam" && previewExam && (
              <ExamEditor
                key={previewExam.key}
                exam={previewExam}
                courses={overview.courses}
                onSaved={() => setPreview(null)}
              />
            )}
            {!readOnly && (
              <Button
                variant="default"
                onClick={() => {
                  const block = preview;
                  setPreview(null);
                  if (block.kind === "exam") {
                    open(block.id);
                    return;
                  }
                  onEditingChange?.(true);
                  if (block.kind === "plan") {
                    const plan = data.plans.find((row) => row.id === block.id);
                    if (plan) openPlan?.(plan);
                  } else {
                    const event = events.find((row) => row.id === block.id);
                    if (event) openEvent(event, term);
                  }
                }}
              >
                {t("Edit")}
              </Button>
            )}
          </div>
        )}
      </Modal>
    </div>
  );
}
