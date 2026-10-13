import { COURSE_PALETTE } from "../../core/course-colors";
import { tokyoDate } from "../../core/civil-date";
import { t } from "../../i18n";
import { UiButton, UiInput, UiSelect } from "../../components/mantine";
import { confirmAction } from "../../components/confirm";
import { SyllabusDetails } from "./syllabus-details";
import type { SyllabusCourse } from "../../core/syllabus";
import { useState, type FormEvent } from "react";
import type {
  Course,
  InstructionLanguage,
  Meeting,
  PersonalEvent,
  TermId,
} from "../../core/model";
import { classSlots } from "../../core/timetable-layout";
import { dayNames } from "../../components/timetable/timetable";
import {
  Checkbox,
  Chip,
  Drawer,
  Group,
  Modal,
  SegmentedControl,
  Text,
} from "@mantine/core";

const palette = COURSE_PALETTE;

type MeetingRow = {
  days: number[];
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

const slotFor = (meeting: Meeting) =>
  classSlots.find(
    (slot) => slot.start === meeting.startTime && slot.end === meeting.endTime,
  )?.label ?? "custom";

const rowFromMeeting = (meeting: Meeting): MeetingRow => ({
  days: [...meeting.days],
  slot: slotFor(meeting),
  startTime: meeting.startTime,
  endTime: meeting.endTime,
  room: meeting.room,
});

const blankRow = (): MeetingRow => ({
  days: [],
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
      <Group gap={4} role="group" aria-label="Weekdays">
        {dayNames.map((name, index) => (
          <Chip key={name} value={String(index + 1)} size="xs">
            {name.slice(0, 3)}
          </Chip>
        ))}
      </Group>
    </Chip.Group>
  );
}

export function CourseEditor({
  course,
  fetchSyllabus,
  term,
  onTimetable,
  onClose,
  onSave,
  onDelete,
}: {
  course?: Course;
  fetchSyllabus?: (url: string) => Promise<SyllabusCourse>;
  term: TermId;
  onTimetable: boolean;
  onClose: () => void;
  onSave: (draft: CourseDraft) => void;
  onDelete?: () => void;
}) {
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
      ? course.meetings.map(rowFromMeeting)
      : [blankRow()],
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
        return setError("A meeting must end after it starts.");
      const slot = classSlots.find((s) => s.label === row.slot);
      meetings.push({
        days: row.days,
        periods: slot ? [...slot.periods] : [],
        startTime: row.startTime,
        endTime: row.endTime,
        room: row.room.trim(),
      });
    }
    onSave({ ...values, meetings });
  };
  return (
    <Drawer
      opened
      onClose={onClose}
      position="right"
      size="min(560px, 100%)"
      title={
        <div>
          <Text fw={700}>{course ? "Edit course" : "New course"}</Text>
          <Text c="dimmed" size="xs">
            {term.replace("-", " ")}
          </Text>
        </div>
      }
      overlayProps={{ backgroundOpacity: 0.28, blur: 1 }}
      styles={{ body: { height: "calc(100% - 64px)", padding: 0 } }}
    >
      <form className="sheetInner" onSubmit={submit}>
        <Group className="sheetActions" justify="flex-end" py="sm">
          <UiButton type="button" className="button" onClick={onClose}>
            {t("Cancel")}
          </UiButton>
          <UiButton type="submit" className="button primary">
            {t("Save")}
          </UiButton>
        </Group>
        <div className="sheetBody courseEditBody">
          <section className="sheetCol">
            <label className="field wide">
              <span>{t("Title")}</span>
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
              <span>{t("Code")}</span>
              <UiInput
                required
                value={values.code}
                onChange={(e) => setValues({ ...values, code: e.target.value })}
                placeholder="CSC.T354"
              />
            </label>
            <label className="field">
              <span>{t("Credits")}</span>
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
              <span>{t("Instructor")}</span>
              <UiInput
                value={values.instructor}
                onChange={(e) =>
                  setValues({ ...values, instructor: e.target.value })
                }
              />
            </label>
            <label className="field">
              <span>{t("Language")}</span>
              <UiSelect
                value={values.language}
                onChange={(e) =>
                  setValues({
                    ...values,
                    language: e.target.value as InstructionLanguage,
                  })
                }
              >
                <option value="ja">Japanese</option>
                <option value="en">English</option>
                <option value="ja-en">Japanese / English</option>
                <option value="other">Other</option>
              </UiSelect>
            </label>
            <div className="field">
              <span>{t("Color")}</span>
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
                  aria-label="Custom color"
                  onChange={(e) =>
                    setValues({ ...values, color: e.target.value })
                  }
                />
              </div>
            </div>
            <label className="field wide">
              <span>{t("Syllabus URL")}</span>
              <UiInput
                type="url"
                value={values.url}
                onChange={(e) => setValues({ ...values, url: e.target.value })}
                placeholder="https://syllabus.s.isct.ac.jp/courses/…"
              />
            </label>
            <Checkbox
              className="wide"
              checked={values.onTimetable}
              label={t("Show on the timetable")}
              onChange={(event) =>
                setValues({
                  ...values,
                  onTimetable: event.currentTarget.checked,
                })
              }
            />
          </section>
          <section className="sheetCol">
            <div className="sheetColHead">
              <b>{t("Meetings")}</b>
              <UiButton
                type="button"
                className="button small"
                onClick={() => setRows([...rows, blankRow()])}
              >
                {t("＋ Add")}
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
                    aria-label={t("Period")}
                    value={row.slot}
                    onChange={(e) => {
                      const slot = classSlots.find(
                        (s) => s.label === e.target.value,
                      );
                      setRow(index, {
                        slot: e.target.value,
                        startTime: slot?.start ?? row.startTime,
                        endTime: slot?.end ?? row.endTime,
                      });
                    }}
                  >
                    {classSlots.map((slot) => (
                      <option value={slot.label} key={slot.label}>
                        {slot.label} · {slot.start}–{slot.end}
                      </option>
                    ))}
                    <option value="custom">Custom time</option>
                  </UiSelect>
                  {row.slot === "custom" && (
                    <div className="timeRange">
                      <UiInput
                        type="time"
                        aria-label="Starts"
                        value={row.startTime}
                        onChange={(e) =>
                          setRow(index, { startTime: e.target.value })
                        }
                      />
                      <UiInput
                        type="time"
                        aria-label="Ends"
                        value={row.endTime}
                        onChange={(e) =>
                          setRow(index, { endTime: e.target.value })
                        }
                      />
                    </div>
                  )}
                  <UiInput
                    aria-label={t("Room")}
                    placeholder={t("Room")}
                    value={row.room}
                    onChange={(e) => setRow(index, { room: e.target.value })}
                  />
                  <UiButton
                    type="button"
                    className="iconButton"
                    aria-label="Remove meeting"
                    onClick={() => setRows(rows.filter((_, i) => i !== index))}
                  >
                    ×
                  </UiButton>
                </div>
              ))}
              {!rows.length && (
                <div className="empty small">No meetings. Add one above.</div>
              )}
            </div>
            {error && (
              <p className="formError" role="alert">
                {error}
              </p>
            )}
          </section>
          <SyllabusDetails url={values.url} fetchCourse={fetchSyllabus} />
        </div>
        {course && onDelete && (
          <footer className="sheetDangerZone">
            <div>
              <b>Delete course</b>
              <span>Remove this local course and its timetable entry.</span>
            </div>
            <UiButton
              type="button"
              className="button danger"
              onClick={async () => {
                if (
                  await confirmAction(
                    t("Delete {{title}}? This cannot be undone.", {
                      title: course.title,
                    }),
                    t("Delete"),
                  )
                )
                  onDelete();
              }}
            >
              {t("Delete")}
            </UiButton>
          </footer>
        )}
      </form>
    </Drawer>
  );
}

export type EventDraft = Pick<
  PersonalEvent,
  "title" | "room" | "date" | "days" | "startTime" | "endTime" | "blocking"
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
  onSave: (draft: EventDraft) => void;
  onDelete?: () => void;
}) {
  const [draft, setDraft] = useState<EventDraft>({
    title: event?.title ?? "",
    room: event?.room ?? "",
    date: event?.date,
    days: event?.date ? undefined : (event?.days ?? [3]),
    startTime: event?.startTime ?? "13:30",
    endTime: event?.endTime ?? "15:10",
    blocking: event?.blocking ?? true,
  });
  const [error, setError] = useState("");
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (draft.startTime >= draft.endTime)
      return setError("The event must end after it starts.");
    if (!draft.date && !draft.days?.length)
      return setError("Pick at least one weekday or a date.");
    onSave({ ...draft, title: draft.title.trim(), room: draft.room.trim() });
  };
  return (
    <Drawer
      opened
      onClose={onClose}
      position="right"
      size="min(440px, 100%)"
      title={
        <div>
          <Text fw={700}>{event ? "Edit event" : "New event"}</Text>
          <Text c="dimmed" size="xs">
            {term.replace("-", " ")}
          </Text>
        </div>
      }
      overlayProps={{ backgroundOpacity: 0.28, blur: 1 }}
      styles={{ body: { height: "calc(100% - 64px)", padding: 0 } }}
    >
      <form className="sheetInner narrow" onSubmit={submit}>
        <Group className="sheetActions" justify="flex-end" py="sm">
          <UiButton type="button" className="button" onClick={onClose}>
            {t("Cancel")}
          </UiButton>
          <UiButton type="submit" className="button primary">
            {t("Save")}
          </UiButton>
        </Group>
        <div className="sheetBody single">
          <label className="field wide">
            <span>{t("Title")}</span>
            <UiInput
              autoFocus
              required
              value={draft.title}
              onChange={(e) => setDraft({ ...draft, title: e.target.value })}
              placeholder="研究室ゼミ"
            />
          </label>
          <label className="field wide">
            <span>{t("Room")}</span>
            <UiInput
              value={draft.room}
              onChange={(e) => setDraft({ ...draft, room: e.target.value })}
            />
          </label>
          <div className="field wide">
            <span>Repeats</span>
            <SegmentedControl
              value={draft.date ? "date" : "weekly"}
              data={[
                { value: "weekly", label: "Weekly" },
                { value: "date", label: "One date" },
              ]}
              onChange={(value) => {
                if (value === "weekly")
                  setDraft({
                    ...draft,
                    date: undefined,
                    days: draft.days?.length ? draft.days : [3],
                  });
                else
                  setDraft({
                    ...draft,
                    days: undefined,
                    date:
                      draft.date ??
                      tokyoDate(new Date()),
                  });
              }}
            />
          </div>
          {draft.date ? (
            <label className="field wide">
              <span>Date</span>
              <UiInput
                type="date"
                required
                value={draft.date}
                onChange={(e) => setDraft({ ...draft, date: e.target.value })}
              />
            </label>
          ) : (
            <div className="field wide">
              <span>Weekdays</span>
              <DayChips
                value={draft.days ?? []}
                onChange={(days) => setDraft({ ...draft, days })}
              />
            </div>
          )}
          <label className="field">
            <span>Starts</span>
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
            <span>Ends</span>
            <UiInput
              type="time"
              required
              value={draft.endTime}
              onChange={(e) => setDraft({ ...draft, endTime: e.target.value })}
            />
          </label>
          <Checkbox
            className="wide"
            checked={draft.blocking}
            label="Count as a conflict when it overlaps a class"
            onChange={(event) =>
              setDraft({ ...draft, blocking: event.currentTarget.checked })
            }
          />
          {error && (
            <p className="formError wide" role="alert">
              {error}
            </p>
          )}
        </div>
        {event && onDelete && (
          <footer className="sheetDangerZone">
            <div>
              <b>Delete event</b>
              <span>Remove this event from your timetable.</span>
            </div>
            <UiButton
              type="button"
              className="button danger"
              onClick={async () => {
                if (
                  await confirmAction(
                    t("Delete {{title}}?", { title: event.title }),
                    t("Delete"),
                  )
                )
                  onDelete();
              }}
            >
              {t("Delete")}
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
      title={t("Add quarter")}
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
            aria-label={t("Academic year")}
            value={year}
            onChange={(e) => setYear(Number(e.target.value))}
          >
            {Array.from({ length: 7 }, (_, i) => defaultYear - 3 + i).map(
              (option) => (
                <option key={option} value={option}>
                  {option}
                </option>
              ),
            )}
          </UiSelect>
          <SegmentedControl
            value={quarter}
            data={["1Q", "2Q", "3Q", "4Q"]}
            onChange={(value) => setQuarter(value as typeof quarter)}
          />
        </div>
        <small>{taken ? "This quarter already exists." : " "}</small>
        <div className="sheetActions">
          <UiButton type="button" className="button" onClick={onClose}>
            {t("Cancel")}
          </UiButton>
          <UiButton type="submit" className="button primary" disabled={taken}>
            Add {term.replace("-", " ")}
          </UiButton>
        </div>
      </form>
    </Modal>
  );
}
