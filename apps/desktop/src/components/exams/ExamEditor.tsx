import { Button, Checkbox, Modal, Select, TextInput } from "@mantine/core";
import { useState } from "react";
import type {
  ExamAssociation,
  ExamView,
  ScheduleOverride,
} from "../../generated/persistence";
import type { Course, Workspace } from "../../lib/model";
import { useScheduleQuery } from "../../platform/core-query";
import { workspaceActions } from "../../store/workspace";
import { t } from "../../i18n";
import "./exams.css";

type Draft = Omit<
  ScheduleOverride,
  "id" | "rev" | "updatedAt" | "updatedBy" | "deletedAt"
>;

/** All exam surfaces use the same editor. Core decides whether a new course
 * association needs confirmation and rechecks that decision when saving. */
export function ExamEditor({
  exam,
  data,
  courses,
  onSaved,
}: {
  exam: ExamView;
  data: Workspace;
  courses: Course[];
  onSaved?: () => void;
}) {
  const [courseId, setCourseId] = useState(exam.courseId ?? "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [saved, setSaved] = useState(false);
  const [confirmation, setConfirmation] = useState<{
    draft: Draft;
    exams: ExamView[];
  } | null>(null);
  const existing = data.scheduleOverrides.find(
    (row) => !row.deletedAt && row.targetKey === exam.key,
  );
  const association = useScheduleQuery<ExamAssociation>(
    { kind: "examAssociation", examKey: exam.key, courseId },
    Boolean(courseId),
  );
  const save = async (draft: Draft, confirmedExamKeys?: string[]) => {
    setSaving(true);
    setError("");
    setSaved(false);
    try {
      await workspaceActions.mutate({
        kind: "saveScheduleOverride",
        existingId: existing?.id,
        draft,
        confirmedExamKeys,
      });
      setConfirmation(null);
      setSaved(true);
      onSaved?.();
    } catch (cause) {
      setConfirmation(null);
      setError(String(cause));
      association.retry?.();
    } finally {
      setSaving(false);
    }
  };
  return (
    <>
      <form
        className="examEditor"
        onSubmit={(event) => {
          event.preventDefault();
          if (
            saving ||
            (courseId && (!association.value || association.pending))
          )
            return;
          const values = new FormData(event.currentTarget);
          const draft: Draft = {
            targetKey: exam.key,
            targetKind: "exam",
            courseId:
              courseId ||
              (exam.courseId || existing?.courseId === "" ? "" : undefined),
            date: String(values.get("date")),
            startTime: String(values.get("startTime") || "") || undefined,
            endTime: String(values.get("endTime") || "") || undefined,
            room: String(values.get("room") ?? ""),
            title: existing?.title,
            hidden: values.get("hidden") === "on",
          };
          if (association.value?.requiresConfirmation && courseId)
            setConfirmation({ draft, exams: association.value.conflicts });
          else void save(draft);
        }}
      >
        <p>{t("Personal exam edits are kept when subscriptions refresh.")}</p>
        {exam.notes && <p>{exam.notes}</p>}
        <Select
          name="courseId"
          label={t("Linked course")}
          value={courseId}
          onChange={(value) => {
            setCourseId(value ?? "");
            setSaved(false);
          }}
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
        <TextInput
          name="date"
          type="date"
          label={t("Date")}
          required
          defaultValue={exam.date}
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
        <TextInput name="room" label={t("Room")} defaultValue={exam.room} />
        <Checkbox
          name="hidden"
          label={t("Hide from my schedule")}
          defaultChecked={exam.hidden}
        />
        {(error || (courseId && association.error)) && (
          <p role="alert">{error || association.error}</p>
        )}
        {saved && <p role="status">{t("Exam saved")}</p>}
        <div className="examEditorActions">
          {existing && (
            <Button
              variant="subtle"
              disabled={saving}
              onClick={async () => {
                setSaving(true);
                setError("");
                try {
                  await workspaceActions.mutate({
                    kind: "deleteScheduleOverride",
                    overrideId: existing.id,
                  });
                  setSaved(true);
                  onSaved?.();
                } catch (cause) {
                  setError(String(cause));
                } finally {
                  setSaving(false);
                }
              }}
            >
              {t("Reset to source")}
            </Button>
          )}
          <Button
            type="submit"
            loading={saving}
            disabled={Boolean(
              courseId &&
              (association.pending || !association.value || association.error),
            )}
          >
            {t("Save")}
          </Button>
        </div>
      </form>
      <Modal
        opened={Boolean(confirmation)}
        onClose={() => !saving && setConfirmation(null)}
        title={t("Course already has an exam")}
      >
        <p>{t("Link this exam as well? Existing exam links will be kept.")}</p>
        <ul>
          {confirmation?.exams.map((other) => (
            <li key={other.key}>
              {other.title || other.code} · {other.date}
            </li>
          ))}
        </ul>
        <div className="examEditorActions">
          <Button
            variant="default"
            disabled={saving}
            onClick={() => setConfirmation(null)}
          >
            {t("Cancel")}
          </Button>
          <Button
            loading={saving}
            onClick={() =>
              confirmation &&
              void save(
                confirmation.draft,
                confirmation.exams.map((other) => other.key),
              )
            }
          >
            {t("Link exam")}
          </Button>
        </div>
      </Modal>
    </>
  );
}
