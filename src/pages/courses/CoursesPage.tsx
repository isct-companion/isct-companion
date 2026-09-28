import { t } from "../../i18n";
import { useState } from "react";
import { detectConflicts } from "../../core/academic";
import { Badge, Group, Modal, Paper, Text } from "@mantine/core";

import {
  deleteLocalCourseBatch,
  editCourseBatch,
  filterCourseCatalog,
  visibleSelection,
} from "../../core/course-workflows";

import {
  WeekTimetable,
  dayNames,
  type Block,
} from "../../components/timetable/timetable";

import type {
  Course,
  PersonalEvent,
  TermId,
  Workspace,
} from "../../core/model";

import {
  onTimetable,
  setTimetable,
  timetableCourses,
  allTerms,
  weekColumns,
} from "../../components/timetable/workspace";

export function Courses({
  catalog,
  data,
  currentTerm,
  readOnly,
  update,
  open,
  openEvent,
  openCourse,
  addTerm,
  toggleTimetable,
  notify,
  openCatalog,
  closeCatalog,
}: {
  data: Workspace;
  currentTerm: TermId;
  readOnly: boolean;
  update: (f: (d: Workspace) => Workspace) => void;
  open: (courseId: string) => void;
  openEvent: (event: PersonalEvent | undefined, term: TermId) => void;
  openCourse: (course: Course | undefined, term: TermId) => void;
  addTerm: () => void;
  toggleTimetable: (course: Course) => void;
  notify: (message: string) => void;
  openCatalog: (term: TermId) => void;
  closeCatalog?: () => void;
  catalog?: import("react").ReactNode;
}) {
  const [query, setQuery] = useState("");
  const [language, setLanguage] = useState("all");
  const [weekday, setWeekday] = useState(0);
  const [period, setPeriod] = useState(0);
  const [status, setStatus] = useState<
    "all" | "planned" | "available" | "conflict"
  >("all");
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [showFilters, setShowFilters] = useState(false);
  const [selectionMode, setSelectionMode] = useState(false);
  const [batchEdit, setBatchEdit] = useState(false);
  const terms = allTerms(data, currentTerm);
  const term = terms.includes(data.selectedTerm)
    ? data.selectedTerm
    : currentTerm;
  const courses = data.courses.filter((c) => c.termId === term && !c.deletedAt);
  const events = data.events.filter((e) => e.termId === term && !e.deletedAt);
  const scheduled = timetableCourses(data, term);
  const conflictList = detectConflicts(scheduled, events);
  const conflicts = new Set(conflictList.flatMap((c) => [c.leftId, c.rightId]));
  const q = query.trim().toLocaleLowerCase();
  const languages = [
    ...new Set(courses.map((course) => course.language).filter(Boolean)),
  ].sort();
  const plannedIds = new Set(scheduled.map((course) => course.id));
  const filters = { query, language, weekday, period, status };
  const filtered = filterCourseCatalog(courses, filters, plannedIds, conflicts);
  const matches = (course: Course) =>
    filtered.some((item) => item.id === course.id);
  const visibleIds = new Set(filtered.map((course) => course.id));
  const selectedVisible = visibleSelection(filtered, selected);
  const hiddenSelectionCount = [...selected].filter(
    (id) => !visibleIds.has(id),
  ).length;
  const selectedAdd = selectedVisible.filter(
    (course) => !onTimetable(data, course),
  );
  const selectedRemove = selectedVisible.filter((course) =>
    onTimetable(data, course),
  );
  const toggleSelected = (id: string) =>
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const selectVisible = () =>
    setSelected((current) => {
      const next = new Set(current);
      const allSelected =
        filtered.length > 0 && filtered.every((course) => next.has(course.id));
      for (const course of filtered)
        allSelected ? next.delete(course.id) : next.add(course.id);
      return next;
    });
  const applyBatch = (coursesToChange: Course[], on: boolean) => {
    if (!coursesToChange.length) return;
    if (
      !on &&
      !confirm(
        `Remove ${coursesToChange.length} selected course${coursesToChange.length === 1 ? "" : "s"} from this local timetable? This does not withdraw school registration.`,
      )
    )
      return;
    update((draft) => {
      for (const chosen of coursesToChange) {
        const course = draft.courses.find(
          (item) => item.id === chosen.id && !item.deletedAt,
        );
        if (course) setTimetable(draft, course, on);
      }
      return draft;
    });
    setSelected((current) => {
      const next = new Set(current);
      for (const course of coursesToChange) next.delete(course.id);
      return next;
    });
    notify(
      on
        ? `${coursesToChange.length} course${coursesToChange.length === 1 ? "" : "s"} added to your timetable`
        : `${coursesToChange.length} course${coursesToChange.length === 1 ? "" : "s"} removed from this local timetable`,
    );
  };
  const deleteSelected = () => {
    if (
      !selectedVisible.length ||
      !confirm(
        `Delete ${selectedVisible.length} selected local course record${selectedVisible.length === 1 ? "" : "s"}? This does not withdraw school registration.`,
      )
    )
      return;
    update((draft) => {
      const ids = new Set(selectedVisible.map((course) => course.id));
      deleteLocalCourseBatch(draft, ids);
      return draft;
    });
    notify(
      `${selectedVisible.length} local course record${selectedVisible.length === 1 ? "" : "s"} deleted`,
    );
    setSelected(new Set());
  };
  const columns = weekColumns(
    data,
    scheduled,
    events,
    conflicts,
    q || language !== "all" || weekday || period || status !== "all"
      ? matches
      : undefined,
  );
  const credits = scheduled.reduce((sum, c) => sum + c.credits, 0);
  const removable =
    !courses.length && !events.length && (data.terms ?? []).includes(term);
  const select = (block: Block) => {
    if (block.kind === "course") open(block.id);
    else {
      const event = events.find((e) => e.id === block.id);
      if (event) openEvent(event, term);
    }
  };
  const scheduleLabel = (course: Course) =>
    course.meetings.length
      ? course.meetings
          .map(
            (meeting) =>
              `${meeting.days.map((day) => dayNames[day - 1].slice(0, 3)).join("/")} ${meeting.startTime}–${meeting.endTime}`,
          )
          .join(" · ")
      : "Time not scheduled";
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
              onClick={() => {
                update((d) => ({ ...d, selectedTerm: tabTerm }));
                if (catalog) openCatalog(tabTerm);
              }}
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
          {!catalog && (
            <UiInput
              className="search"
              type="search"
              placeholder={t("Search courses")}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          )}
          {!readOnly && (
            <>
              <UiButton
                className="button"
                onClick={() => openEvent(undefined, term)}
              >
                {t("＋ Event")}
              </UiButton>
              <UiButton
                className="button primary"
                onClick={() => (catalog ? closeCatalog?.() : openCatalog(term))}
              >
                {t(catalog ? "Done" : "＋ Course")}
              </UiButton>
            </>
          )}
        </div>
      </header>
      <div className="coursesBody">
        {catalog}
        <Paper
          component="section"
          className="ttCard"
          withBorder
          shadow="xs"
          radius="md"
        >
          {catalog && scheduled.some((course) => !course.meetings.length) && (
            <div className="unscheduledDrafts">
              <b>
                Unscheduled drafts ·{" "}
                {scheduled.filter((course) => !course.meetings.length).length}
              </b>
              <span>Times not published in the syllabus</span>
              <div>
                {scheduled
                  .filter((course) => !course.meetings.length)
                  .map((course) => (
                    <UiButton
                      key={course.id}
                      type="button"
                      onClick={() => openCourse(course, term)}
                    >
                      {course.title}
                    </UiButton>
                  ))}
              </div>
            </div>
          )}
          <WeekTimetable
            days={columns}
            onSelect={select}
            emptyLabel="Nothing scheduled this day"
          />
        </Paper>
        {!catalog && (
          <Paper
            component="aside"
            className="listCard courseList"
            withBorder
            shadow="xs"
            radius="md"
          >
            <header>
              <div>
                <b>{term.replace("-", " ")}</b>
                <span>
                  {scheduled.length} on timetable · {credits} credits
                </span>
              </div>
              <div className="courseListTools">
                <UiButton
                  type="button"
                  className={showFilters ? "active" : ""}
                  aria-expanded={showFilters}
                  onClick={() => setShowFilters((value) => !value)}
                >
                  Filters
                  {(language !== "all" ||
                    weekday ||
                    period ||
                    status !== "all") &&
                    " · Active"}
                </UiButton>
                {!readOnly && (
                  <UiButton
                    type="button"
                    className={selectionMode ? "active" : ""}
                    aria-pressed={selectionMode}
                    onClick={() => {
                      setSelectionMode((value) => !value);
                      setSelected(new Set());
                    }}
                  >
                    {selectionMode ? "Done" : "Select"}
                  </UiButton>
                )}
              </div>
            </header>
            {showFilters && (
              <div className="courseFilters" aria-label={t("Course filters")}>
                <UiSelect
                  aria-label={t("Language")}
                  value={language}
                  onChange={(event) => setLanguage(event.target.value)}
                >
                  <option value="all">{t("All languages")}</option>
                  {languages.map((value) => (
                    <option value={value} key={value}>
                      {value}
                    </option>
                  ))}
                </UiSelect>
                <UiSelect
                  aria-label={t("Weekday")}
                  value={weekday}
                  onChange={(event) => setWeekday(Number(event.target.value))}
                >
                  <option value={0}>{t("Any day")}</option>
                  {dayNames.map((day, index) => (
                    <option value={index + 1} key={t(day)}>
                      {t(day)}
                    </option>
                  ))}
                </UiSelect>
                <UiSelect
                  aria-label={t("Period")}
                  value={period}
                  onChange={(event) => setPeriod(Number(event.target.value))}
                >
                  <option value={0}>{t("Any period")}</option>
                  {[1, 2, 3, 4, 5, 6, 7].map((value) => (
                    <option value={value} key={value}>
                      Period {value}
                    </option>
                  ))}
                </UiSelect>
                <UiSelect
                  aria-label={t("Plan status")}
                  value={status}
                  onChange={(event) =>
                    setStatus(event.target.value as typeof status)
                  }
                >
                  <option value="all">{t("All courses")}</option>
                  <option value="available">{t("Available to add")}</option>
                  <option value="planned">{t("On timetable")}</option>
                  <option value="conflict">{t("Scheduled conflict")}</option>
                </UiSelect>
                {(q ||
                  language !== "all" ||
                  weekday ||
                  period ||
                  status !== "all") && (
                  <UiButton
                    type="button"
                    className="filterClear"
                    onClick={() => {
                      setQuery("");
                      setLanguage("all");
                      setWeekday(0);
                      setPeriod(0);
                      setStatus("all");
                    }}
                  >
                    {t("Clear filters")}
                  </UiButton>
                )}
              </div>
            )}
            {selectionMode && (
              <div className="courseBatchBar">
                {!readOnly && (
                  <UiButton
                    type="button"
                    className="textButton"
                    onClick={selectVisible}
                    disabled={!filtered.length}
                  >
                    {filtered.length > 0 &&
                    filtered.every((course) => selected.has(course.id))
                      ? "Clear visible"
                      : `Select visible (${filtered.length})`}
                  </UiButton>
                )}
                <span aria-live="polite">
                  {filtered.length} result{filtered.length === 1 ? "" : "s"}
                  {selectedVisible.length
                    ? ` · ${selectedVisible.length} selected`
                    : ""}
                  {hiddenSelectionCount
                    ? ` · ${hiddenSelectionCount} hidden`
                    : ""}
                </span>
                {!readOnly && selectedAdd.length > 0 && (
                  <UiButton
                    type="button"
                    className="button primary compactButton"
                    onClick={() => applyBatch(selectedAdd, true)}
                  >
                    Add {selectedAdd.length}
                  </UiButton>
                )}
                {!readOnly && selectedRemove.length > 0 && (
                  <UiButton
                    type="button"
                    className="button compactButton"
                    onClick={() => applyBatch(selectedRemove, false)}
                  >
                    Remove {selectedRemove.length}
                  </UiButton>
                )}
                {!readOnly && selectedVisible.length > 0 && (
                  <UiButton
                    type="button"
                    className="button compactButton"
                    onClick={() => setBatchEdit(true)}
                  >
                    Edit {selectedVisible.length}
                  </UiButton>
                )}
                {!readOnly && selectedVisible.length > 0 && (
                  <UiButton
                    type="button"
                    className="button danger compactButton"
                    onClick={deleteSelected}
                  >
                    Delete {selectedVisible.length}
                  </UiButton>
                )}
              </div>
            )}
            <div className="scroll">
              {filtered.map((course) => {
                const on = onTimetable(data, course);
                return (
                  <div
                    className={`courseRow${on ? "" : " off"}${conflicts.has(course.id) ? " conflict" : ""}`}
                    key={course.id}
                  >
                    {!readOnly && selectionMode && (
                      <label
                        className="courseSelect"
                        title="Select for a batch action"
                      >
                        <UiInput
                          type="checkbox"
                          checked={selected.has(course.id)}
                          onChange={() => toggleSelected(course.id)}
                          aria-label={`Select ${course.title}`}
                        />
                      </label>
                    )}
                    <UiButton
                      type="button"
                      className="courseRowMain"
                      onClick={() => open(course.id)}
                    >
                      <i style={{ background: course.color }} />
                      <span>
                        <b>{course.title}</b>
                        <small>
                          {course.code} · {scheduleLabel(course)}
                          {course.instructor ? ` · ${course.instructor}` : ""}
                        </small>
                      </span>
                    </UiButton>
                    {!readOnly && (
                      <UiButton
                        type="button"
                        className={`switch${on ? " on" : ""}`}
                        role="switch"
                        aria-checked={on}
                        aria-label={`${course.title} on timetable`}
                        title={
                          on ? "Remove from timetable" : "Add to timetable"
                        }
                        onClick={() => toggleTimetable(course)}
                      >
                        <i />
                      </UiButton>
                    )}
                  </div>
                );
              })}
              {!filtered.length && (
                <div className="empty small">
                  {courses.length
                    ? "No courses match."
                    : "No courses in this quarter yet."}
                </div>
              )}
              {events.length > 0 &&
                !q &&
                language === "all" &&
                !weekday &&
                !period &&
                status === "all" && (
                  <>
                    <div className="listSub">Events</div>
                    {events.map((event) => (
                      <UiButton
                        type="button"
                        className="courseRow courseRowMain event"
                        key={event.id}
                        onClick={() => openEvent(event, term)}
                      >
                        <i />
                        <span>
                          <b>{event.title}</b>
                          <small>
                            {event.date
                              ? event.date
                              : event.days
                                  ?.map((d) => dayNames[d - 1].slice(0, 3))
                                  .join(" ")}
                            {` · ${event.startTime}–${event.endTime}`}
                            {event.room ? ` · ${event.room}` : ""}
                          </small>
                        </span>
                      </UiButton>
                    ))}
                  </>
                )}
              {removable && !readOnly && (
                <UiButton
                  type="button"
                  className="textButton"
                  onClick={() =>
                    update((d) => ({
                      ...d,
                      terms: (d.terms ?? []).filter((t) => t !== term),
                      selectedTerm: currentTerm,
                    }))
                  }
                >
                  Remove this empty quarter
                </UiButton>
              )}
            </div>
          </Paper>
        )}
      </div>
      {batchEdit && (
        <BatchCourseEditDialog
          count={selectedVisible.length}
          close={() => setBatchEdit(false)}
          save={(changes) => {
            update((draft) => {
              const ids = new Set(selectedVisible.map((course) => course.id));
              editCourseBatch(draft, ids, changes);
              return draft;
            });
            notify(`${selectedVisible.length} courses updated`);
            setBatchEdit(false);
          }}
        />
      )}
    </div>
  );
}

function BatchCourseEditDialog({
  count,
  close,
  save,
}: {
  count: number;
  close: () => void;
  save: (changes: {
    language?: string;
    instructor?: string;
    credits?: number;
  }) => void;
}) {
  const [language, setLanguage] = useState("");
  const [instructor, setInstructor] = useState("");
  const [credits, setCredits] = useState("");
  const parsedCredits = credits === "" ? undefined : Number(credits);
  const changed = Boolean(
    language.trim() || instructor.trim() || credits !== "",
  );
  const valid =
    changed &&
    (parsedCredits === undefined ||
      (Number.isFinite(parsedCredits) &&
        parsedCredits >= 0 &&
        parsedCredits <= 20));
  return (
    <Modal
      opened
      onClose={close}
      centered
      size="lg"
      title={
        <div>
          <Text fw={700}>Edit {count} selected courses</Text>
          <Text c="dimmed" size="xs">
            Only filled fields change. Titles, codes, and meeting times stay
            course-specific.
          </Text>
        </div>
      }
      overlayProps={{ backgroundOpacity: 0.28, blur: 1 }}
    >
      <div className="batchCourseDialog">
        <div className="batchEditFields">
          <label>
            <span>{t("Language")}</span>
            <UiInput
              value={language}
              onChange={(event) => setLanguage(event.target.value)}
              placeholder={t("Leave unchanged")}
            />
          </label>
          <label>
            <span>{t("Instructor")}</span>
            <UiInput
              value={instructor}
              onChange={(event) => setInstructor(event.target.value)}
              placeholder={t("Leave unchanged")}
            />
          </label>
          <label>
            <span>{t("Credits")}</span>
            <UiInput
              type="number"
              min="0"
              max="20"
              step="0.5"
              value={credits}
              onChange={(event) => setCredits(event.target.value)}
              placeholder={t("Leave unchanged")}
            />
          </label>
        </div>
        <div className="batchPreview">
          Changes apply to {count} selected course{count === 1 ? "" : "s"}.
        </div>
        <Group justify="flex-end" className="batchCourseActions">
          <UiButton className="button" onClick={close}>
            {t("Cancel")}
          </UiButton>
          <UiButton
            className="button primary"
            disabled={!valid}
            onClick={() =>
              save({
                language: language.trim() || undefined,
                instructor: instructor.trim() || undefined,
                credits: parsedCredits,
              })
            }
          >
            Apply changes
          </UiButton>
        </Group>
      </div>
    </Modal>
  );
}

/* ---------- Course drawer ---------- */
import { UiButton, UiInput, UiSelect } from "../../components/mantine";
