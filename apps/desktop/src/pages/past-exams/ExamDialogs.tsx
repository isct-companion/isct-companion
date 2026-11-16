import { X } from "lucide-react";
import { useState } from "react";
import { desktopBridge } from "../../app/native";
import { notify } from "../../app/notify";
import {
  Button,
  Chip,
  Field,
  Group,
  Modal,
  Radio,
  SegmentedControl,
  Select,
  Stack,
  TextInput,
  UiTextarea,
} from "../../components/ui";
import { t } from "../../i18n";
import type {
  ExamReportReason,
  ExamRequestDraft,
  ExamSubjectPage,
  ExamTeacher,
  ExamUploadDraft,
} from "../../generated/persistence";
import type { ExamPickedFile } from "../../platform/desktop-bridge";

const reasons = [
  "copyright",
  "inappropriate",
  "wrongAnswer",
  "brokenFile",
  "other",
] as const;
export const occasions = [
  "final",
  "midterm",
  "makeup",
  "quiz",
  "check",
  "report",
  "exercise",
  "textbook",
  "mock",
] as const;
/** Requests name a recent year; older ones are rare enough to leave open. */
const YEARS_BACK = 8;

/** Asks for documents of one subject; every field but the subject is optional. */
export function RequestDialog({
  opened,
  subject,
  close,
  submit,
}: {
  opened: boolean;
  subject: ExamSubjectPage;
  close: () => void;
  submit: (draft: ExamRequestDraft) => Promise<void>;
}) {
  const [classId, setClassId] = useState("");
  const [year, setYear] = useState("");
  const [occasion, setOccasion] = useState("");
  const [role, setRole] = useState("");
  const [description, setDescription] = useState("");
  const [sending, setSending] = useState(false);
  const thisYear = new Date().getFullYear();
  const classes = subject.rows.flatMap((row) =>
    row.classIds.slice(0, 1).map((id) => ({
      value: String(id),
      label:
        [...row.labels, ...row.teachers.map(({ name }) => name), row.taughtIn]
          .filter(Boolean)
          .join(" · ") || String(id),
    })),
  );
  return (
    <Modal opened={opened} onClose={close} title={t("examShare.newRequest")}>
      <form
        onSubmit={async (event) => {
          event.preventDefault();
          setSending(true);
          await submit({
            subjectCode: subject.subject.code,
            classId: Number(classId) || null,
            academicYear: Number(year) || null,
            occasion: occasion || null,
            wantedRole: role || null,
            description: description.trim(),
          });
          setSending(false);
        }}
      >
        <Stack>
          {classes.length > 1 && (
            <Select
              label={t("examShare.class")}
              value={classId}
              onChange={setClassId}
              data={[
                { value: "", label: t("examShare.allClasses") },
                ...classes,
              ]}
            />
          )}
          <Group grow>
            <Select
              label={t("examShare.year")}
              value={year}
              onChange={setYear}
              data={[
                { value: "", label: t("examShare.anyYear") },
                ...Array.from({ length: YEARS_BACK }, (_, i) =>
                  String(thisYear - i),
                ),
              ]}
            />
            <Select
              label={t("examShare.occasion")}
              value={occasion}
              onChange={setOccasion}
              data={[
                { value: "", label: t("examShare.allOccasions") },
                ...occasions.map((value) => ({
                  value,
                  label: t(`examShare.occasion.${value}`),
                })),
              ]}
            />
          </Group>
          <Field label={t("examShare.fileType")}>
            <SegmentedControl
              value={role || "any"}
              onChange={(value) => setRole(value === "any" ? "" : value)}
              data={[
                { value: "any", label: t("examShare.anyRole") },
                { value: "question", label: t("examShare.role.question") },
                { value: "answer", label: t("examShare.role.answer") },
              ]}
            />
          </Field>
          <Field label={t("examShare.description")}>
            <UiTextarea
              rows={3}
              maxLength={1000}
              value={description}
              onChange={(e) => setDescription(e.currentTarget.value)}
            />
          </Field>
          <Group justify="flex-end">
            <Button variant="default" onClick={close}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" loading={sending}>
              {t("examShare.sendRequest")}
            </Button>
          </Group>
        </Stack>
      </form>
    </Modal>
  );
}

export function ReportDialog({
  opened,
  close,
  submit,
}: {
  opened: boolean;
  close: () => void;
  submit: (reason: ExamReportReason, detail: string) => Promise<void>;
}) {
  const [reason, setReason] = useState<ExamReportReason>(reasons[0]);
  const [detail, setDetail] = useState("");
  const [sending, setSending] = useState(false);
  return (
    <Modal opened={opened} onClose={close} title={t("examShare.report")}>
      <form
        onSubmit={async (event) => {
          event.preventDefault();
          setSending(true);
          await submit(reason, detail.trim());
          setSending(false);
        }}
      >
        <Stack>
          <Radio.Group
            label={t("examShare.reportReason")}
            value={reason}
            onChange={(value) => setReason(value as ExamReportReason)}
          >
            {reasons.map((value) => (
              <Radio
                key={value}
                value={value}
                label={t(`examShare.reason.${value}`)}
              />
            ))}
          </Radio.Group>
          <Field label={t("examShare.description")}>
            <UiTextarea
              rows={3}
              maxLength={900}
              value={detail}
              onChange={(e) => setDetail(e.currentTarget.value)}
            />
          </Field>
          <Group justify="flex-end">
            <Button variant="default" onClick={close}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" color="red" loading={sending}>
              {t("examShare.sendReport")}
            </Button>
          </Group>
        </Stack>
      </form>
    </Modal>
  );
}

/** What an upload belongs to: a subject opened from the list, or a request
 * it answers. The teachers to pick from, and the ones ticked at first. */
export type UploadTarget = {
  subjectCode: string;
  subjectName: string;
  teachers: ExamTeacher[];
  mine: string[];
  academicYear?: number | null;
  occasion?: string | null;
  role?: string | null;
  requestId?: string | null;
};

const roles = ["question", "answer", "both", "notes", "other"] as const;
/** Held several times a term, so the number matters (Core's NUMBERED). */
const numbered = ["quiz", "check", "report"];
const photo = /\.(jpe?g|png|webp)$/i;
const MAX_BYTES = 25 * 1024 * 1024;

const size = (bytes: number) =>
  bytes < 1024 * 1024
    ? `${Math.max(1, Math.round(bytes / 1024))} KB`
    : `${(bytes / 1024 / 1024).toFixed(1)} MB`;

/** Shares a past exam. Core checks the same rules again and merges several
 * photos into one PDF; this form only keeps the choices the site accepts. */
export function UploadDialog({
  target,
  close,
  submit,
}: {
  target: UploadTarget;
  close: () => void;
  submit: (draft: ExamUploadDraft) => Promise<boolean>;
}) {
  const thisYear = new Date().getFullYear();
  const [files, setFiles] = useState<ExamPickedFile[]>([]);
  const [role, setRole] = useState<string>(target.role ?? "question");
  const [year, setYear] = useState(String(target.academicYear ?? thisYear));
  const [occasion, setOccasion] = useState(target.occasion ?? "");
  const [number, setNumber] = useState("1");
  const [teachers, setTeachers] = useState(target.mine);
  const [provenance, setProvenance] = useState("original");
  const [answerSource, setAnswerSource] = useState("unknown");
  const [note, setNote] = useState("");
  const [sending, setSending] = useState(false);

  const sitting = role !== "notes" && role !== "other";
  const question = role === "question" || role === "both";
  const answer = role === "answer" || role === "both";
  const merging = files.length > 1;
  const problem =
    merging && !files.every((file) => photo.test(file.name))
      ? t("examShare.onlyPhotosMerge")
      : !merging &&
          files[0] &&
          !photo.test(files[0].name) &&
          files[0].size > MAX_BYTES
        ? t("error.exam-share-too-large")
        : undefined;
  const ready = files.length > 0 && !problem && (!sitting || !!occasion);

  const pick = async () => {
    let picked: ExamPickedFile[];
    try {
      picked = (await desktopBridge?.examPickUpload?.()) ?? [];
    } catch (e) {
      notify.error(e);
      return;
    }
    setFiles((current) => [
      ...current,
      ...picked.filter((file) => !current.some((c) => c.path === file.path)),
    ]);
  };

  return (
    <Modal
      opened
      onClose={close}
      title={t("examShare.uploadTitle", { subject: target.subjectName })}
    >
      <form
        onSubmit={async (event) => {
          event.preventDefault();
          if (!ready) return;
          setSending(true);
          const done = await submit({
            files: files.map((file) => file.path),
            subjectCodes: [target.subjectCode],
            teacherIds: teachers,
            academicYear: sitting && year ? Number(year) : null,
            occasion: sitting ? occasion : null,
            occasionNumber: numbered.includes(occasion) ? Number(number) : 0,
            variant: "",
            role,
            provenance: question ? provenance : null,
            answerSource: answer ? answerSource : null,
            note: note.trim(),
            requestId: target.requestId ?? null,
          });
          setSending(false);
          if (done) close();
        }}
      >
        <Stack>
          <Field
            label={t("examShare.uploadFiles")}
            description={t("examShare.uploadFilesHint")}
            error={problem}
          >
            <div className="examUploadFiles">
              {files.map((file) => (
                <span key={file.path} className="examUploadFile">
                  <span title={file.name}>{file.name}</span>
                  <small>{size(file.size)}</small>
                  <button
                    type="button"
                    aria-label={`${t("examShare.removeFile")} ${file.name}`}
                    title={t("examShare.removeFile")}
                    onClick={() =>
                      setFiles(files.filter((f) => f.path !== file.path))
                    }
                  >
                    <X size={12} aria-hidden />
                  </button>
                </span>
              ))}
              <Button variant="default" size="compact-sm" onClick={pick}>
                {t("examShare.pickFiles")}
              </Button>
            </div>
          </Field>
          <Select
            label={t("examShare.kind")}
            value={role}
            onChange={setRole}
            data={roles.map((value) => ({
              value,
              label: t(`examShare.role.${value}`),
            }))}
          />
          {sitting && (
            <Group grow>
              <Select
                label={t("examShare.year")}
                value={year}
                onChange={setYear}
                data={[
                  ...Array.from({ length: YEARS_BACK }, (_, i) =>
                    String(thisYear - i),
                  ),
                  { value: "", label: t("examShare.unknownYear") },
                ]}
              />
              <Select
                label={t("examShare.occasion")}
                value={occasion}
                onChange={setOccasion}
                placeholder={t("examShare.pickOccasion")}
                data={[...occasions, "unknown" as const].map((value) => ({
                  value,
                  label: t(`examShare.occasion.${value}`),
                }))}
              />
              {numbered.includes(occasion) && (
                <Select
                  label={t("examShare.occasionNumber")}
                  value={number}
                  onChange={setNumber}
                  data={Array.from({ length: 15 }, (_, i) => String(i + 1))}
                />
              )}
            </Group>
          )}
          {target.teachers.length > 0 && (
            <Field label={t("examShare.teachers")}>
              <Chip.Group multiple value={teachers} onChange={setTeachers}>
                {target.teachers.map((teacher) => (
                  <Chip key={teacher.id} value={teacher.id}>
                    {teacher.name}
                  </Chip>
                ))}
              </Chip.Group>
            </Field>
          )}
          {question && (
            <Field label={t("examShare.provenance")}>
              <SegmentedControl
                value={provenance}
                onChange={setProvenance}
                data={(["original", "reconstructed"] as const).map((value) => ({
                  value,
                  label: t(`examShare.provenance.${value}`),
                }))}
              />
            </Field>
          )}
          {answer && (
            <Field label={t("examShare.answerSource")}>
              <SegmentedControl
                value={answerSource}
                onChange={setAnswerSource}
                data={(["official", "student", "unknown"] as const).map(
                  (value) => ({
                    value,
                    label: t(`examShare.answerSource.${value}`),
                  }),
                )}
              />
            </Field>
          )}
          <TextInput
            label={t("examShare.note")}
            maxLength={40}
            value={note}
            onChange={(e) => setNote(e.currentTarget.value)}
          />
          <p className="examQuiet">{t("examShare.uploadPublic")}</p>
          <Group justify="flex-end">
            <Button variant="default" onClick={close}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" loading={sending} disabled={!ready}>
              {t("examShare.upload")}
            </Button>
          </Group>
        </Stack>
      </form>
    </Modal>
  );
}
