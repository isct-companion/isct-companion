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
  start && end ? `${start}–${end}` : t("exam.timeNotAnnounced");

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
        t("exam.examDeleted"),
        () => void workspaceActions.mutate(restore).catch(notify.error),
      );
    });
  };
  const source = exam.source;
  const sourceHint = (changed: boolean, value: string) =>
    changed ? t("exam.sourceValue", { value: value || "—" }) : undefined;
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
                ? "exam.noLongerInThePublished"
                : "exam.thePublishedTimetableChangedAfter",
            )}
          </b>
          <p>{t("exam.yourEditValue", { value: facts(exam) })}</p>
          {!exam.conflict.removed && (
            <>
              <p>{t("exam.publishedNowValue", { value: facts(source) })}</p>
              <p>
                {t("exam.whenYouEditedValue", {
                  value: facts(exam.conflict.base),
                })}
              </p>
              <div className="examEditorActions">
                <Button
                  variant="light"
                  disabled={saving}
                  onClick={() => settle(source)}
                >
                  {t("exam.usePublished")}
                </Button>
                <Button
                  variant="subtle"
                  disabled={saving}
                  onClick={() => settle(exam)}
                >
                  {t("exam.keepMine")}
                </Button>
              </div>
            </>
          )}
        </section>
      )}
      {!published && <p>{t("exam.markedOnTheCalendar")}</p>}
      {published && (
        <Select
          label={t("exam.linkedCourse")}
          value={exam.courseId ?? ""}
          onChange={(value) =>
            void run({
              kind: "linkExam",
              examKey: exam.key,
              courseId: value || undefined,
            })
          }
          description={
            exam.autoLinked ? t("exam.matchedByCourseCode") : undefined
          }
          data={[
            { value: "", label: t("common.notLinked") },
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
          {t("exam.thisCourseAlsoHasExams", {
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
        <p>{t("exam.fieldsYouChangeStayYours")}</p>
        {exam.notes && <p>{exam.notes}</p>}
        <TextInput
          name="date"
          type="date"
          label={t("common.date")}
          required
          defaultValue={exam.date}
          description={sourceHint(exam.date !== source.date, source.date)}
        />
        <div className="examEditorTimes">
          <TextInput
            name="startTime"
            type="time"
            label={t("common.startTime")}
            defaultValue={exam.startTime ?? ""}
          />
          <TextInput
            name="endTime"
            type="time"
            label={t("common.endTime")}
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
          label={t("common.room")}
          defaultValue={exam.room}
          description={sourceHint(exam.room !== source.room, source.room)}
        />
        {(error || association.error) && (
          <p role="alert">{error || association.error}</p>
        )}
        {saved && <p role="status">{t("exam.examSaved")}</p>}
        <div className="examEditorActions">
          {(!published || exam.courseId) && (
            <Button
              variant="subtle"
              color="red"
              disabled={saving}
              onClick={remove}
            >
              {t("common.delete")}
            </Button>
          )}
          {exam.edited && (
            <Button variant="subtle" disabled={saving} onClick={() => edit()}>
              {t("exam.resetToSource")}
            </Button>
          )}
          <Button type="submit" loading={saving}>
            {t("common.save")}
          </Button>
        </div>
      </form>
    </div>
  );
}
