import { intlLocale } from "../../i18n/locale";
import { t } from "../../i18n";
import { useState, type ReactNode } from "react";
import { detectConflicts } from "../../core/academic";
import {
  ActionIcon,
  Badge,
  Button,
  Group,
  Menu,
  Paper,
  Switch,
  Modal,
  TextInput,
} from "@mantine/core";
import { useLocalStorage } from "@mantine/hooks";

import {
  WeekTimetable,
  type Block,
} from "../../components/timetable/timetable";

import type { PersonalEvent, Plan, TermId, Workspace } from "../../core/model";

import {
  timetableCourses,
  allTerms,
  templateWeekColumns,
  weekColumnsAt,
} from "../../components/timetable/workspace";
import type { CalendarExportScope } from "../../core/calendar-ics";
import { UiButton } from "../../components/mantine";
import {
  addDays,
  courseTerms,
  quarterRange,
  tokyoDate,
} from "../../core/schedule";
import { YearQuarterPicker } from "./YearQuarterPicker";
import { PullRing } from "../../components/PullRing";
import { newUserRecordId } from "../../storage/ids";

/** The term the unified Schedule workspace shows: the selected one when it exists, else the current one. */
export function coursesTerm(data: Workspace, currentTerm: TermId): TermId {
  return allTerms(data, currentTerm).includes(data.selectedTerm)
    ? data.selectedTerm
    : currentTerm;
}

/**
 * Timetable on the left, syllabus search on the right. Clicking a course opens
 * its details over the search (`panel`); removing happens there, so a stray
 * click on the timetable never deletes anything.
 */
export function Courses({
  data,
  currentTerm,
  readOnly,
  update,
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
  pullLms,
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
  readOnly: boolean;
  update: (f: (d: Workspace) => Workspace) => void;
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
  /** Pulls LMS courses for the quarter on screen, or every quarter. */
  pullLms?: (scope: "term" | "all") => void;
  /** 0–100 while an LMS pull runs, else null. */
  pullProgress?: number | null;
  /** Opens 教務Web registration with this quarter's planned courses queued. */
  register?: (term: TermId) => void;
  /** GPA / credits summary opened from the top toolbar. */
  academic?: ReactNode;
  /** Mobile: one header row, viewing only; extras move into a menu. */
  compact?: boolean;
}) {
  const terms = allTerms(data, currentTerm);
  const term = coursesTerm(data, currentTerm);
  const canEdit = editing && !readOnly;
  const [weekAnchor, setWeekAnchor] = useState<{
    term: TermId;
    date: string;
  }>();
  const today = tokyoDate(new Date());
  const range = quarterRange(data, term);
  const anchorDate =
    weekAnchor?.term === term
      ? weekAnchor.date
      : range && today >= range.from && today <= range.to
        ? today
        : (range?.from ?? today);
  const moveWeek = (days: number) =>
    setWeekAnchor({ term, date: addDays(anchorDate, days) });
  const courses = data.courses.filter(
    (c) => !c.deletedAt && courseTerms(data, c).includes(term),
  );
  const events = data.events.filter((e) => e.termId === term && !e.deletedAt);
  const scheduled = timetableCourses(data, term);
  // Intensive or unpublished times have no grid cell; they get their own lane.
  const unscheduled = scheduled.filter((course) => !course.meetings.length);
  const conflictList = editing ? detectConflicts(scheduled, events) : [];
  const conflicts = new Set(conflictList.flatMap((c) => [c.leftId, c.rightId]));
  const [weekLength, setWeekLength] = useLocalStorage<"5" | "7">({
    key: "isct-companion-courses-week-length",
    defaultValue: "5",
  });
  const allColumns = editing
    ? templateWeekColumns(scheduled, events, conflicts)
    : weekColumnsAt(
        data,
        scheduled,
        events,
        new Date(`${anchorDate}T12:00:00+09:00`),
        undefined,
        undefined,
        term,
      );
  const dateFormat = new Intl.DateTimeFormat(intlLocale(data.settings.locale), {
    day: "numeric",
    month: "short",
    timeZone: "Asia/Tokyo",
  });
  const weekLabel = editing
    ? ""
    : `${dateFormat.format(new Date(`${allColumns[0].date}T12:00:00+09:00`))} – ${dateFormat.format(new Date(`${allColumns[6].date}T12:00:00+09:00`))}`;
  // Weekdays only, unless a weekend day actually has something on it.
  const columns =
    !editing || weekLength === "7"
      ? allColumns
      : allColumns.slice(
          0,
          // Slot clicks use the column position as the weekday, so keep it contiguous.
          allColumns[6]?.blocks.length
            ? 7
            : allColumns[5]?.blocks.length
              ? 6
              : 5,
        );
  const removable =
    !courses.length && !events.length && (data.terms ?? []).includes(term);
  const [preview, setPreview] = useState<Block | null>(null);
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
      const event = events.find((e) => e.id === block.id);
      if (event) openEvent(event, term);
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
      <h1>{t("Schedule")}</h1>
      <YearQuarterPicker
        terms={terms}
        selected={term}
        current={currentTerm}
        onSelect={(selectedTerm) => update((d) => ({ ...d, selectedTerm }))}
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
          <h1>{t("Schedule")}</h1>
          <span>
            {editing
              ? t("Plan courses for this quarter")
              : t("Courses and personal plans for the week")}
          </span>
        </div>
        <Switch
          label={t("Edit")}
          checked={editing}
          onChange={(event) => onEditingChange?.(event.currentTarget.checked)}
        />
      </div>
      <header className="pageBar" hidden={compact}>
        <YearQuarterPicker
          terms={terms}
          selected={term}
          current={currentTerm}
          onSelect={(selectedTerm) => update((d) => ({ ...d, selectedTerm }))}
          onAdd={canEdit ? addTerm : undefined}
        />
        <div className="pageBarRight">
          {academic}
          {canEdit && pullLms && (
            <Button.Group>
              <Button
                size="xs"
                variant="default"
                disabled={pullProgress !== null}
                leftSection={
                  pullProgress !== null ? (
                    <PullRing value={pullProgress} />
                  ) : undefined
                }
                onClick={() => pullLms("term")}
                title={t("Pull this quarter's courses from LMS")}
              >
                {pullProgress !== null
                  ? `${Math.round(pullProgress)}%`
                  : t("Pull from LMS")}
              </Button>
              <Menu position="bottom-end" withinPortal>
                <Menu.Target>
                  <Button
                    size="xs"
                    variant="default"
                    disabled={pullProgress !== null}
                    aria-label={t("More pull options")}
                    px={6}
                  >
                    ▾
                  </Button>
                </Menu.Target>
                <Menu.Dropdown>
                  <Menu.Item onClick={() => pullLms("all")}>
                    {t("Pull all quarters")}
                  </Menu.Item>
                </Menu.Dropdown>
              </Menu>
            </Button.Group>
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
              title="Editing is available in the desktop app"
            >
              {t("Read only")}
            </Badge>
          )}
          {conflictList.length > 0 && (
            <Badge variant="light" color="red">
              {conflictList.length} conflict{conflictList.length > 1 ? "s" : ""}
            </Badge>
          )}
          {removable && canEdit && (
            <UiButton
              className="button"
              onClick={() =>
                update((d) => ({
                  ...d,
                  terms: (d.terms ?? []).filter((item) => item !== term),
                  selectedTerm: currentTerm,
                }))
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
        <div className="coursesLeft">
          <Paper
            component="section"
            className="ttCard"
            withBorder
            shadow="xs"
            radius="md"
          >
            <div className="timetableViewBar">
              <b>{editing ? t("Quarter template") : weekLabel}</b>
              {!editing && (
                <div className="scheduleControls">
                  <UiButton
                    type="button"
                    aria-label={t("Previous week")}
                    onClick={() => moveWeek(-7)}
                  >
                    ←
                  </UiButton>
                  <UiButton
                    type="button"
                    onClick={() => {
                      update((d) => ({ ...d, selectedTerm: currentTerm }));
                      setWeekAnchor({ term: currentTerm, date: today });
                    }}
                  >
                    {t("This week")}
                  </UiButton>
                  <UiButton
                    type="button"
                    aria-label={t("Next week")}
                    onClick={() => moveWeek(7)}
                  >
                    →
                  </UiButton>
                </div>
              )}
            </div>
            {editing && (
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
                  onClick={() => setWeekLength(weekLength === "7" ? "5" : "7")}
                >
                  {weekLength === "7" ? t("7 days") : t("5 days")}
                </UiButton>
              </div>
            )}
            <WeekTimetable
              days={columns}
              onSelect={select}
              emptyLabel="Nothing scheduled this day"
              showNow={!editing}
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
                EXAM · {preview.timeLabel}
              </Badge>
            )}
            <p>
              {preview.startTime}–{preview.endTime}
            </p>
            {preview.room && <p>{preview.room}</p>}
            {preview.conflictWith?.length ? (
              <p className="scheduleConflict">
                Overlaps {preview.conflictWith.join(", ")}
              </p>
            ) : null}
            {!readOnly &&
              preview.kind === "exam" &&
              (() => {
                const existing = data.scheduleOverrides.find(
                  (row) => !row.deletedAt && row.targetKey === preview.key,
                );
                return (
                  <form
                    key={`${preview.key}:${existing?.rev ?? 0}`}
                    className="scheduleOverrideForm"
                    onSubmit={async (event) => {
                      event.preventDefault();
                      const values = new FormData(event.currentTarget);
                      const id = existing?.id ?? (await newUserRecordId());
                      const now = new Date().toISOString();
                      update((draft) => {
                        let row = draft.scheduleOverrides.find(
                          (item) => item.id === id,
                        );
                        if (!row) {
                          row = {
                            id,
                            targetKey: preview.key,
                            targetKind: "exam",
                            courseId: preview.id,
                            hidden: false,
                            rev: 0,
                            updatedAt: now,
                            updatedBy: draft.deviceId,
                          };
                          draft.scheduleOverrides.push(row);
                        }
                        row.date = String(
                          values.get("date") || preview.date || "",
                        );
                        row.startTime = String(
                          values.get("startTime") || preview.startTime,
                        );
                        row.endTime = String(
                          values.get("endTime") || preview.endTime,
                        );
                        row.room = String(values.get("room") ?? preview.room);
                        row.hidden = false;
                        row.rev += 1;
                        row.updatedAt = now;
                        row.updatedBy = draft.deviceId;
                        return draft;
                      });
                      setPreview(null);
                    }}
                  >
                    <TextInput
                      label={t("Date")}
                      name="date"
                      type="date"
                      defaultValue={existing?.date ?? preview.date}
                    />
                    <Group grow>
                      <TextInput
                        label={t("Start")}
                        name="startTime"
                        type="time"
                        defaultValue={existing?.startTime ?? preview.startTime}
                      />
                      <TextInput
                        label={t("End")}
                        name="endTime"
                        type="time"
                        defaultValue={existing?.endTime ?? preview.endTime}
                      />
                    </Group>
                    <TextInput
                      label={t("Room")}
                      name="room"
                      defaultValue={existing?.room ?? preview.room}
                    />
                    <Group justify="space-between">
                      <Button
                        color="red"
                        variant="subtle"
                        type="button"
                        onClick={async () => {
                          const id = existing?.id ?? (await newUserRecordId());
                          const now = new Date().toISOString();
                          update((draft) => {
                            let row = draft.scheduleOverrides.find(
                              (item) => item.id === id,
                            );
                            if (!row) {
                              row = {
                                id,
                                targetKey: preview.key,
                                targetKind: "exam",
                                courseId: preview.id,
                                hidden: true,
                                rev: 0,
                                updatedAt: now,
                                updatedBy: draft.deviceId,
                              };
                              draft.scheduleOverrides.push(row);
                            }
                            row.hidden = true;
                            row.rev += 1;
                            row.updatedAt = now;
                            return draft;
                          });
                          setPreview(null);
                        }}
                      >
                        {t("Hide exam")}
                      </Button>
                      <Button type="submit">{t("Save override")}</Button>
                    </Group>
                  </form>
                );
              })()}
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
