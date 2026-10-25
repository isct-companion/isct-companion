import { Button, Select, TextInput } from "../ui";
import { useState } from "react";
import type { ExamAssociation, ExamView } from "../../generated/persistence";
import type { Course } from "../../lib/model";
import { useScheduleQuery } from "../../platform/core-query";
import { workspaceActions } from "../../store/workspace";
import { notify } from "../../app/notify";
import { t } from "../../i18n";
import "./exams.css";

const timeRange = (start: string | null, end: string | null) =>
  start && end ? `${start}–${end}` : t("Time not announced");

/** All exam surfaces use the same editor. The course link applies at once;
 * the schedule form saves only what differs from the published exam, so later
 * source corrections still reach every field the user left alone. */
export function ExamEditor({
  exam,
  courses,
  onSaved,
  onRemoved,
}: {
  exam: ExamView;
  courses: Course[];
  onSaved?: () => void;
  onRemoved?: () => void;
}) {
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [saved, setSaved] = useState(false);
  // A session marked on the calendar is its own course's; it has no link.
  const published = exam.origin === "published";
  const association = useScheduleQuery<ExamAssociation>(
    {
      kind: "examAssociation",
      examKey: exam.key,
      courseId: exam.courseId ?? "",
    },
    published && Boolean(exam.courseId),
  );
  const run = async (
    intent: Parameters<typeof workspaceActions.mutate>[0],
    done?: () => void,
  ) => {
    setSaving(true);
    setError("");
    setSaved(false);
    try {
      await workspaceActions.mutate(intent);
      setSaved(true);
      done?.();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setSaving(false);
    }
  };
  const edit = (values?: FormData) => {
    const field = (name: string) =>
      String(values?.get(name) ?? "").trim() || undefined;
    void run(
      {
        kind: "editExam",
        examKey: exam.key,
        date: field("date"),
        startTime: field("startTime"),
        endTime: field("endTime"),
        room: field("room"),
      },
      onSaved,
    );
  };
  /** Saves these values; Core records today's published facts beside them. */
  const settle = (values: {
    date: string;
    startTime: string | null;
    endTime: string | null;
    room: string;
  }) =>
    void run({
      kind: "editExam",
      examKey: exam.key,
      date: values.date,
      startTime: values.startTime ?? undefined,
      endTime: values.endTime ?? undefined,
      room: values.room || undefined,
    });
  /** Takes the exam off the schedule. Undo puts back what Core changed: the
   * session's mark, or the published exam's course link. */
  const remove = () => {
    const restore: Parameters<typeof workspaceActions.mutate>[0] = published
      ? {
          kind: "linkExam",
          examKey: exam.key,
          courseId: exam.courseId ?? undefined,
        }
      : { kind: "markSession", key: exam.key, mark: exam.kind };
    void run({ kind: "removeExam", examKey: exam.key }, () => {
      onRemoved?.();
      notify.undoable(
        "Exam deleted",
        () => void workspaceActions.mutate(restore).catch(notify.error),
      );
    });
  };
  const source = exam.source;
  const sourceHint = (changed: boolean, value: string) =>
    changed ? t("Source: {{value}}", { value: value || "—" }) : undefined;
  const conflicts =
    published && exam.courseId ? (association.value?.conflicts ?? []) : [];
  const facts = (value: {
    date: string;
    startTime: string | null;
    endTime: string | null;
    room: string;
  }) =>
    [value.date, timeRange(value.startTime, value.endTime), value.room]
      .filter(Boolean)
      .join(" · ");
  return (
    <div className="examEditor">
      {exam.conflict && (
        <section className="examConflict" role="status">
          <b>
            {t(
              exam.conflict.removed
                ? "No longer in the published timetable."
                : "The published timetable changed after your edit.",
            )}
          </b>
          <p>{t("Your edit: {{value}}", { value: facts(exam) })}</p>
          {!exam.conflict.removed && (
            <>
              <p>{t("Published now: {{value}}", { value: facts(source) })}</p>
              <p>
                {t("When you edited: {{value}}", {
                  value: facts(exam.conflict.base),
                })}
              </p>
              <div className="examEditorActions">
                <Button disabled={saving} onClick={() => settle(source)}>
                  {t("Use published")}
                </Button>
                <Button
                  variant="subtle"
                  disabled={saving}
                  onClick={() => settle(exam)}
                >
                  {t("Keep mine")}
                </Button>
              </div>
            </>
          )}
        </section>
      )}
      {!published && <p>{t("Marked on the calendar")}</p>}
      {published && (
        <Select
          label={t("Linked course")}
          value={exam.courseId ?? ""}
          onChange={(value) =>
            void run({
              kind: "linkExam",
              examKey: exam.key,
              courseId: value || undefined,
            })
          }
          description={
            exam.autoLinked ? t("Matched by course code") : undefined
          }
          data={[
            { value: "", label: t("Not linked") },
            ...courses.map((course) => ({
              value: course.id,
              label: `${course.title} · ${course.code}`,
            })),
          ]}
          searchable
          allowDeselect={false}
          disabled={saving}
        />
      )}
      {conflicts.length > 0 && (
        <p className="examEditorNote">
          {t("This course also has: {{exams}}", {
            exams: conflicts
              .map((other) => `${other.title || other.code} · ${other.date}`)
              .join(", "),
          })}
        </p>
      )}
      <form
        key={[
          exam.key,
          exam.date,
          exam.startTime,
          exam.endTime,
          exam.room,
        ].join("|")}
        onSubmit={(event) => {
          event.preventDefault();
          if (!saving) edit(new FormData(event.currentTarget));
        }}
      >
        <p>
          {t(
            "Fields you change stay yours; the rest follows the published timetable.",
          )}
        </p>
        {exam.notes && <p>{exam.notes}</p>}
        <TextInput
          name="date"
          type="date"
          label={t("Date")}
          required
          defaultValue={exam.date}
          description={sourceHint(exam.date !== source.date, source.date)}
        />
        <div className="examEditorTimes">
          <TextInput
            name="startTime"
            type="time"
            label={t("Start time")}
            defaultValue={exam.startTime ?? ""}
          />
          <TextInput
            name="endTime"
            type="time"
            label={t("End time")}
            defaultValue={exam.endTime ?? ""}
          />
        </div>
        {(exam.startTime !== source.startTime ||
          exam.endTime !== source.endTime) && (
          <p className="examEditorNote">
            {sourceHint(true, timeRange(source.startTime, source.endTime))}
          </p>
        )}
        <TextInput
          name="room"
          label={t("Room")}
          defaultValue={exam.room}
          description={sourceHint(exam.room !== source.room, source.room)}
        />
        {(error || association.error) && (
          <p role="alert">{error || association.error}</p>
        )}
        {saved && <p role="status">{t("Exam saved")}</p>}
        <div className="examEditorActions">
          {(!published || exam.courseId) && (
            <Button
              variant="subtle"
              color="red"
              disabled={saving}
              onClick={remove}
            >
              {t("Delete")}
            </Button>
          )}
          {exam.edited && (
            <Button variant="subtle" disabled={saving} onClick={() => edit()}>
              {t("Reset to source")}
            </Button>
          )}
          <Button type="submit" loading={saving}>
            {t("Save")}
          </Button>
        </div>
      </form>
    </div>
  );
}
