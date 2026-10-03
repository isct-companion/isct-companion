import { t } from "../../i18n";
import type { ReactNode } from "react";
import { detectConflicts } from "../../core/academic";
import { Badge, Button, Menu, Paper } from "@mantine/core";
import { useLocalStorage } from "@mantine/hooks";

import {
  WeekTimetable,
  type Block,
} from "../../components/timetable/timetable";

import type {
  PersonalEvent,
  TermId,
  Workspace,
} from "../../core/model";

import {
  timetableCourses,
  allTerms,
  weekColumns,
} from "../../components/timetable/workspace";
import { UiButton } from "../../components/mantine";

/** The term the Courses page shows: the selected one when it exists, else the current one. */
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
  pulling,
  register,
  academic,
}: {
  data: Workspace;
  currentTerm: TermId;
  readOnly: boolean;
  update: (f: (d: Workspace) => Workspace) => void;
  /** Shows a course's details over the search panel. */
  open: (courseId: string) => void;
  openEvent: (event: PersonalEvent | undefined, term: TermId) => void;
  addTerm: () => void;
  exportIcs: (term: TermId) => void;
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
  pulling?: boolean;
  /** Opens 教務Web registration with this quarter's planned courses queued. */
  register?: (term: TermId) => void;
  /** GPA / credits summary shown in the bar. */
  academic?: ReactNode;
}) {
  const terms = allTerms(data, currentTerm);
  const term = coursesTerm(data, currentTerm);
  const courses = data.courses.filter((c) => c.termId === term && !c.deletedAt);
  const events = data.events.filter((e) => e.termId === term && !e.deletedAt);
  const scheduled = timetableCourses(data, term);
  // Intensive or unpublished times have no grid cell; they get their own lane.
  const unscheduled = scheduled.filter((course) => !course.meetings.length);
  const conflictList = detectConflicts(scheduled, events);
  const conflicts = new Set(conflictList.flatMap((c) => [c.leftId, c.rightId]));
  const [weekLength, setWeekLength] = useLocalStorage<"5" | "7">({
    key: "stws-courses-week-length",
    defaultValue: "5",
  });
  const allColumns = weekColumns(data, scheduled, events, conflicts);
  // Weekdays only, unless a weekend day actually has something on it.
  const columns =
    weekLength === "7"
      ? allColumns
      : allColumns.slice(
          0,
          // Slot clicks use the column position as the weekday, so keep it contiguous.
          allColumns[6]?.blocks.length ? 7 : allColumns[5]?.blocks.length ? 6 : 5,
        );
  const removable =
    !courses.length && !events.length && (data.terms ?? []).includes(term);
  const select = (block: Block) => {
    if (block.kind === "course") open(block.id);
    else {
      const event = events.find((e) => e.id === block.id);
      if (event) openEvent(event, term);
    }
  };
  return (
    <div className="page">
      <header className="pageBar">
        <div className="termTabs" role="tablist" aria-label={t("Quarter")}>
          {terms.map((tabTerm) => (
            <UiButton
              type="button"
              role="tab"
              aria-selected={tabTerm === term}
              className={tabTerm === term ? "active" : ""}
              key={tabTerm}
              onClick={() => update((d) => ({ ...d, selectedTerm: tabTerm }))}
            >
              {tabTerm.replace("-", " ")}
              {tabTerm === currentTerm && <i title={t("Current quarter")} />}
            </UiButton>
          ))}
          {!readOnly && (
            <UiButton
              type="button"
              className="addTerm"
              title={t("Add quarter")}
              onClick={addTerm}
            >
              ＋
            </UiButton>
          )}
        </div>
        <div className="pageBarRight">
          {academic}
          {pullLms && (
            <Button.Group>
              <Button
                size="xs"
                variant="default"
                loading={pulling}
                onClick={() => pullLms("term")}
                title={t("Pull this quarter's courses from LMS")}
              >
                {t("Pull from LMS")}
              </Button>
              <Menu position="bottom-end" withinPortal>
                <Menu.Target>
                  <Button
                    size="xs"
                    variant="default"
                    disabled={pulling}
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
          {register && !readOnly && (
            <UiButton
              className="button"
              onClick={() => register(term)}
              title={t("Open 教務Web registration with this quarter's courses queued")}
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
          {removable && !readOnly && (
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
          <UiButton className="button" onClick={() => exportIcs(term)}>
            Export ICS
          </UiButton>
          {!readOnly && (
            <UiButton
              className="button"
              onClick={() => openEvent(undefined, term)}
            >
              {t("＋ Event")}
            </UiButton>
          )}
        </div>
      </header>
      <div className="coursesBody">
        <div className="coursesLeft">
        <Paper
          component="section"
          className="ttCard"
          withBorder
          shadow="xs"
          radius="md"
        >
          <div className="intensiveLane">
            {onIntensive ? (
              <UiButton
                type="button"
                className={`intensiveToggle${intensiveActive ? " active" : ""}`}
                aria-pressed={Boolean(intensiveActive)}
                title={t("Find intensive courses")}
                onClick={(event) => onIntensive(event.metaKey || event.ctrlKey)}
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
                  style={{ "--course": course.color } as React.CSSProperties}
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
          <WeekTimetable
            days={columns}
            onSelect={select}
            emptyLabel="Nothing scheduled this day"
            onSlot={onSlot}
            activeSlots={activeSlots}
          />
        </Paper>
        {leftPanel && <div className="coursesSidePanel">{leftPanel}</div>}
        </div>
        <div className="coursesSide">
          {side}
          {panel && <div className="coursesSidePanel">{panel}</div>}
        </div>
      </div>
    </div>
  );
}
