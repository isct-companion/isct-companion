import { CalendarPlus } from "lucide-react";
import { COURSE_PALETTE } from "../../generated/persistence";
import { t } from "../../i18n";
import { UiButton, UiInput, UiSelect, EmptyState } from "../../components/ui";
import { confirmAction } from "../../components/confirm";
import { useSubmissionGuard } from "../../components/use-submission-guard";
import { useState, type FormEvent } from "react";
import type {
  Course,
  InstructionLanguage,
  Meeting,
  PersonalEvent,
  TermId,
} from "../../lib/model";
import {
  classSlots as teachingClassSlots,
  type ClassSlot,
} from "../../lib/timetable-layout";
import type { TeachingPeriod } from "../../generated/persistence";
import { dayNames, dayShort } from "../../components/timetable/timetable";
import {
  Checkbox,
  Chip,
  Drawer,
  Group,
  Modal,
  SegmentedControl,
  Text,
} from "../../components/ui";

const palette = COURSE_PALETTE;

type MeetingRow = {
  days: number[];
  periods: number[];
  slot: string;
  startTime: string;
  endTime: string;
  room: string;
};

export type CourseDraft = {
  title: string;
  code: string;
  instructor: string;
  credits: number;
  language: InstructionLanguage;
  color: string;
  url: string;
  meetings: Meeting[];
  onTimetable: boolean;
};

const periodRangeSlot = (periods: readonly number[]) =>
  `periods:${periods.join(",")}`;

const slotFor = (meeting: Meeting, classSlots: readonly ClassSlot[]) =>
  classSlots.find(
    (slot) =>
      meeting.periods.length === slot.periods.length &&
      meeting.periods.every((period, index) => period === slot.periods[index]),
  )?.label ??
  (meeting.periods.length ? periodRangeSlot(meeting.periods) : "custom");

const rowFromMeeting = (
  meeting: Meeting,
  classSlots: readonly ClassSlot[],
): MeetingRow => {
  const slot = slotFor(meeting, classSlots);
  const teachingSlot = classSlots.find((candidate) => candidate.label === slot);
  return {
    days: [...meeting.days],
    periods: [...meeting.periods],
    slot,
    startTime: teachingSlot?.start ?? meeting.startTime,
    endTime: teachingSlot?.end ?? meeting.endTime,
    room: meeting.room,
  };
};

const blankRow = (classSlots: readonly ClassSlot[]): MeetingRow => ({
  days: [],
  periods: [...classSlots[0].periods],
  slot: classSlots[0].label,
  startTime: classSlots[0].start,
  endTime: classSlots[0].end,
  room: "",
});

function DayChips({
  value,
  onChange,
}: {
  value: number[];
  onChange: (days: number[]) => void;
}) {
  return (
    <Chip.Group
      multiple
      value={value.map(String)}
      onChange={(days) => onChange(days.map(Number).sort((a, b) => a - b))}
    >
      <Group gap={4} role="group" aria-label={t("courses.weekdays")}>
        {dayNames.map((name, index) => (
          <Chip key={name} value={String(index + 1)} size="xs">
            {t(dayShort[index])}
          </Chip>
        ))}
      </Group>
    </Chip.Group>
  );
}

export function CourseEditor({
  course,
  term,
  onTimetable,
  onClose,
  onSave,
  onDelete,
  teachingPeriods,
}: {
  course?: Course;
  term: TermId;
  onTimetable: boolean;
  onClose: () => void;
  onSave: (draft: CourseDraft) => Promise<unknown> | void;
  onDelete?: () => Promise<unknown> | void;
  teachingPeriods: readonly TeachingPeriod[];
}) {
  const submission = useSubmissionGuard();
  const classSlots = teachingClassSlots(teachingPeriods);
  const [values, setValues] = useState({
    title: course?.title ?? "",
    code: course?.code ?? "",
    instructor: course?.instructor ?? "",
    credits: course?.credits ?? 2,
    language: course?.instructionLanguage ?? "ja",
    color: course?.color ?? palette[0],
    url: course?.url ?? "",
    onTimetable,
  });
  const [rows, setRows] = useState<MeetingRow[]>(() =>
    course?.meetings.length
      ? course.meetings.map((meeting) => rowFromMeeting(meeting, classSlots))
      : [blankRow(classSlots)],
  );
  const [error, setError] = useState("");
  const setRow = (index: number, patch: Partial<MeetingRow>) =>
    setRows((current) =>
      current.map((row, i) => (i === index ? { ...row, ...patch } : row)),
    );
  const submit = (event: FormEvent) => {
    event.preventDefault();
    const meetings: Meeting[] = [];
    for (const row of rows) {
      if (!row.days.length) continue;
      if (row.startTime >= row.endTime)
        return setError(t("courses.aMeetingMustEndAfter"));
      meetings.push({
        days: row.days,
        periods: [...row.periods],
        startTime: row.startTime,
        endTime: row.endTime,
        room: row.room.trim(),
      });
    }
    void submission.run(() => onSave({ ...values, meetings }));
  };
  return (
    <Drawer
      opened
      onClose={() => !submission.busy && onClose()}
      closeOnClickOutside={!submission.busy}
      closeOnEscape={!submission.busy}
      position="right"
      size="min(560px, 100%)"
      title={
        <div>
          <Text fw={700}>
            {t(course ? "courses.editCourse" : "courses.newCourse")}
          </Text>
          <Text c="dimmed" size="xs">
            {term.replace("-", " ")}
          </Text>
        </div>
      }
      overlayProps={{ backgroundOpacity: 0.28, blur: 1 }}
      styles={{ body: { height: "calc(100% - 64px)", padding: 0 } }}
    >
      <form
        className="sheetInner"
        onSubmit={submit}
        aria-busy={submission.busy}
      >
        <Group className="sheetActions" justify="flex-end" py="sm">
          <UiButton
            type="button"
            className="button"
            disabled={submission.busy}
            onClick={onClose}
          >
            {t("common.cancel")}
          </UiButton>
          <UiButton
            type="submit"
            className="button primary"
            disabled={submission.busy}
          >
            {t("common.save")}
          </UiButton>
        </Group>
        <div className="sheetBody courseEditBody">
          <section className="sheetCol">
            <label className="field wide">
              <span>{t("courses.title")}</span>
              <UiInput
                autoFocus
                required
                value={values.title}
                onChange={(e) =>
                  setValues({ ...values, title: e.target.value })
                }
                placeholder="並列プログラミング"
              />
            </label>
            <label className="field">
              <span>{t("courses.code")}</span>
              <UiInput
                required
                value={values.code}
                onChange={(e) => setValues({ ...values, code: e.target.value })}
                placeholder="CSC.T354"
              />
            </label>
            <label className="field">
              <span>{t("courses.credits")}</span>
              <UiInput
                type="number"
                min="0"
                step="0.5"
                value={values.credits}
                onChange={(e) =>
                  setValues({ ...values, credits: Number(e.target.value) })
                }
              />
            </label>
            <label className="field wide">
              <span>{t("courses.instructor")}</span>
              <UiInput
                value={values.instructor}
                onChange={(e) =>
                  setValues({ ...values, instructor: e.target.value })
                }
              />
            </label>
            <label className="field">
              <span>{t("common.language")}</span>
              <UiSelect
                value={values.language}
                onChange={(value) =>
                  setValues({
                    ...values,
                    language: value as InstructionLanguage,
                  })
                }
                data={[
                  { value: "ja", label: t("courses.japanese") },
                  { value: "en", label: t("courses.english") },
                  { value: "ja-en", label: t("courses.japaneseEnglish") },
                  { value: "other", label: t("courses.other") },
                ]}
              />
            </label>
            <div className="field">
              <span>{t("courses.color")}</span>
              <div className="swatches">
                {palette.map((color) => (
                  <UiButton
                    type="button"
                    key={color}
                    className={values.color === color ? "on" : ""}
                    style={{ background: color }}
                    aria-label={color}
                    onClick={() => setValues({ ...values, color })}
                  />
                ))}
                <UiInput
                  type="color"
                  value={values.color}
                  aria-label={t("courses.customColor")}
                  onChange={(e) =>
                    setValues({ ...values, color: e.target.value })
                  }
                />
              </div>
            </div>
            <Checkbox
              className="wide"
              checked={values.onTimetable}
              label={t("courses.showOnTheTimetable")}
              onCheckedChange={(checked) =>
                setValues({
                  ...values,
                  onTimetable: checked === true,
                })
              }
            />
          </section>
          <section className="sheetCol">
            <div className="sheetColHead">
              <b>{t("courses.meetings")}</b>
              <UiButton
                type="button"
                className="button small"
                onClick={() => setRows([...rows, blankRow(classSlots)])}
              >
                {t("courses.add")}
              </UiButton>
            </div>
            <div className="meetingRows">
              {rows.map((row, index) => (
                <div className="meetingRow" key={index}>
                  <DayChips
                    value={row.days}
                    onChange={(days) => setRow(index, { days })}
                  />
                  <UiSelect
                    aria-label={t("courses.period")}
                    value={row.slot}
                    onChange={(value) => {
                      const slot = classSlots.find((s) => s.label === value);
                      setRow(index, {
                        slot: value,
                        periods: slot
                          ? [...slot.periods]
                          : value === "custom"
                            ? []
                            : row.periods,
                        startTime: slot?.start ?? row.startTime,
                        endTime: slot?.end ?? row.endTime,
                      });
                    }}
                    data={[
                      ...classSlots.map((slot) => ({
                        value: slot.label,
                        label: `${slot.label} · ${slot.start}–${slot.end}`,
                      })),
                      ...(row.periods.length > 0 &&
                      !classSlots.some((slot) => slot.label === row.slot)
                        ? [
                            {
                              value: row.slot,
                              label: `${row.periods[0]}-${row.periods.at(-1)} · ${row.startTime}–${row.endTime}`,
                            },
                          ]
                        : []),
                      { value: "custom", label: t("courses.customTime") },
                    ]}
                  />
                  {row.slot === "custom" && (
                    <div className="timeRange">
                      <UiInput
                        type="time"
                        aria-label={t("courses.starts")}
                        value={row.startTime}
                        onChange={(e) =>
                          setRow(index, { startTime: e.target.value })
                        }
                      />
                      <UiInput
                        type="time"
                        aria-label={t("courses.ends")}
                        value={row.endTime}
                        onChange={(e) =>
                          setRow(index, { endTime: e.target.value })
                        }
                      />
                    </div>
                  )}
                  <UiInput
                    aria-label={t("common.room")}
                    placeholder={t("common.room")}
                    value={row.room}
                    onChange={(e) => setRow(index, { room: e.target.value })}
                  />
                  <UiButton
                    type="button"
                    className="iconButton"
                    aria-label={t("courses.removeMeeting")}
                    onClick={() => setRows(rows.filter((_, i) => i !== index))}
                  >
                    ×
                  </UiButton>
                </div>
              ))}
              {!rows.length && (
                <EmptyState compact icon={CalendarPlus}>
                  {t("courses.noMeetingsAddOneAbove")}
                </EmptyState>
              )}
            </div>
            {error && (
              <p className="formError" role="alert">
                {error}
              </p>
            )}
            {submission.error && (
              <p className="formError" role="alert">
                {submission.error}
              </p>
            )}
          </section>
        </div>
        {course && onDelete && (
          <footer className="sheetDangerZone">
            <div>
              <b>{t("courses.deleteCourse")}</b>
              <span>{t("courses.removeThisLocalCourseAnd")}</span>
            </div>
            <UiButton
              type="button"
              className="button danger"
              disabled={submission.busy}
              onClick={async () => {
                if (
                  await confirmAction(
                    t("courses.deleteTitleThisCannotBe", {
                      title: course.title,
                    }),
                    t("common.delete"),
                  )
                )
                  void submission.run(onDelete);
              }}
            >
              {t("common.delete")}
            </UiButton>
          </footer>
        )}
      </form>
    </Drawer>
  );
}

export type EventDraft = Pick<
  PersonalEvent,
  "title" | "room" | "days" | "startTime" | "endTime" | "color"
>;

export function EventEditor({
  event,
  term,
  onClose,
  onSave,
  onDelete,
}: {
  event?: PersonalEvent;
  term: TermId;
  onClose: () => void;
  onSave: (draft: EventDraft) => Promise<unknown> | void;
  onDelete?: () => Promise<unknown> | void;
}) {
  const submission = useSubmissionGuard();
  const [draft, setDraft] = useState<EventDraft>({
    title: event?.title ?? "",
    room: event?.room ?? "",
    days: event?.days ?? [3],
    startTime: event?.startTime ?? "13:30",
    endTime: event?.endTime ?? "15:10",
    color: event?.color,
  });
  const [error, setError] = useState("");
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (draft.startTime >= draft.endTime)
      return setError(t("courses.theEventMustEndAfter"));
    if (!draft.days.length) return setError(t("courses.pickAtLeastOneWeekday"));
    void submission.run(() =>
      onSave({ ...draft, title: draft.title.trim(), room: draft.room.trim() }),
    );
  };
  return (
    <Drawer
      opened
      onClose={() => !submission.busy && onClose()}
      closeOnClickOutside={!submission.busy}
      closeOnEscape={!submission.busy}
      position="right"
      size="min(440px, 100%)"
      title={
        <div>
          <Text fw={700}>
            {t(event ? "courses.editWeeklyEvent" : "courses.newWeeklyEvent")}
          </Text>
          <Text c="dimmed" size="xs">
            {term.replace("-", " ")}
          </Text>
        </div>
      }
      overlayProps={{ backgroundOpacity: 0.28, blur: 1 }}
      styles={{ body: { height: "calc(100% - 64px)", padding: 0 } }}
    >
      <form
        className="sheetInner narrow"
        onSubmit={submit}
        aria-busy={submission.busy}
      >
        <Group className="sheetActions" justify="flex-end" py="sm">
          <UiButton
            type="button"
            className="button"
            disabled={submission.busy}
            onClick={onClose}
          >
            {t("common.cancel")}
          </UiButton>
          <UiButton
            type="submit"
            className="button primary"
            disabled={submission.busy}
          >
            {t("common.save")}
          </UiButton>
        </Group>
        <div className="sheetBody single">
          <label className="field wide">
            <span>{t("courses.title")}</span>
            <UiInput
              autoFocus
              required
              value={draft.title}
              onChange={(e) => setDraft({ ...draft, title: e.target.value })}
              placeholder="研究室ゼミ"
            />
          </label>
          <label className="field wide">
            <span>{t("common.room")}</span>
            <UiInput
              value={draft.room}
              onChange={(e) => setDraft({ ...draft, room: e.target.value })}
            />
          </label>
          <div className="field wide">
            <span>{t("courses.everyWeekOn")}</span>
            <DayChips
              value={draft.days}
              onChange={(days) => setDraft({ ...draft, days })}
            />
          </div>
          <div className="field wide">
            <span>{t("courses.color")}</span>
            {/* A new event without a choice gets its colour from Core. */}
            <div className="swatches">
              {palette.map((color) => (
                <UiButton
                  type="button"
                  key={color}
                  className={draft.color === color ? "on" : ""}
                  style={{ background: color }}
                  aria-label={color}
                  aria-pressed={draft.color === color}
                  onClick={() => setDraft({ ...draft, color })}
                />
              ))}
              <UiInput
                type="color"
                value={draft.color ?? palette[0]}
                aria-label={t("courses.customColor")}
                onChange={(e) => setDraft({ ...draft, color: e.target.value })}
              />
            </div>
          </div>
          <label className="field">
            <span>{t("courses.starts")}</span>
            <UiInput
              type="time"
              required
              value={draft.startTime}
              onChange={(e) =>
                setDraft({ ...draft, startTime: e.target.value })
              }
            />
          </label>
          <label className="field">
            <span>{t("courses.ends")}</span>
            <UiInput
              type="time"
              required
              value={draft.endTime}
              onChange={(e) => setDraft({ ...draft, endTime: e.target.value })}
            />
          </label>
          {error && (
            <p className="formError wide" role="alert">
              {error}
            </p>
          )}
          {submission.error && (
            <p className="formError wide" role="alert">
              {submission.error}
            </p>
          )}
        </div>
        {event && onDelete && (
          <footer className="sheetDangerZone">
            <div>
              <b>{t("courses.deleteEvent")}</b>
              <span>{t("courses.removeThisEventFromYour")}</span>
            </div>
            <UiButton
              type="button"
              className="button danger"
              disabled={submission.busy}
              onClick={async () => {
                if (
                  await confirmAction(
                    t("courses.deleteTitle", { title: event.title }),
                    t("common.delete"),
                  )
                )
                  void submission.run(onDelete);
              }}
            >
              {t("common.delete")}
            </UiButton>
          </footer>
        )}
      </form>
    </Drawer>
  );
}

export function TermDialog({
  existing,
  defaultYear,
  onClose,
  onAdd,
}: {
  existing: TermId[];
  defaultYear: number;
  onClose: () => void;
  onAdd: (term: TermId) => void;
}) {
  const [year, setYear] = useState(defaultYear);
  const [quarter, setQuarter] = useState<"1Q" | "2Q" | "3Q" | "4Q">("1Q");
  const term = `${year}-${quarter}` as TermId;
  const taken = existing.includes(term);
  return (
    <Modal
      opened
      onClose={onClose}
      title={t("courses.addQuarter")}
      size="sm"
      centered
      overlayProps={{ backgroundOpacity: 0.28, blur: 1 }}
    >
      <form
        className="termForm"
        onSubmit={(e) => {
          e.preventDefault();
          if (!taken) onAdd(term);
        }}
      >
        <div className="modalFields">
          <UiSelect
            aria-label={t("common.academicYear")}
            value={year}
            onChange={(value) => setYear(Number(value))}
            data={Array.from({ length: 7 }, (_, i) =>
              String(defaultYear - 3 + i),
            )}
          />
          <SegmentedControl
            value={quarter}
            data={["1Q", "2Q", "3Q", "4Q"]}
            onChange={(value) => setQuarter(value as typeof quarter)}
          />
        </div>
        <small>{taken ? t("courses.thisQuarterAlreadyExists") : " "}</small>
        <div className="sheetActions">
          <UiButton type="button" className="button" onClick={onClose}>
            {t("common.cancel")}
          </UiButton>
          <UiButton type="submit" className="button primary" disabled={taken}>
            {t("courses.addTerm", { term: term.replace("-", " ") })}
          </UiButton>
        </div>
      </form>
    </Modal>
  );
}
