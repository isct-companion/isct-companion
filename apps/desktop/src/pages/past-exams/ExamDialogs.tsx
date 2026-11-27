import { ChevronLeft, ChevronRight, Plus, X } from "lucide-react";
import { useEffect, useState } from "react";
import { desktopBridge } from "../../platform/native";
import { errorText } from "../../lib/error-text";
import {
  Button,
  Checkbox,
  Chip,
  Field,
  Group,
  Input,
  Modal,
  Progress,
  Radio,
  SegmentedControl,
  Select,
  Stack,
  Text,
  TextInput,
  UiTextarea,
} from "../../components/ui";
import { lookup, t, type MessageKey } from "../../i18n";
import type {
  ExamAction,
  ExamAnswer,
  ExamAnswerSource,
  ExamChoices,
  ExamOccasion,
  ExamProvenance,
  ExamReportReason,
  ExamRequestDraft,
  ExamRole,
  ExamSubject,
  ExamSubjectPage,
  ExamUploadDraft,
  ExamUploadEdit,
  ExamUploadFile,
  ExamUploadForm,
  ExamUploadReadiness,
  ExamUploadStatus,
} from "../../generated/persistence";

/** The academic years the service names, newest first; none when it names
 * no range. */
const years = ({ limits: { earliestYear, latestYear } }: ExamChoices) =>
  earliestYear === null || latestYear === null
    ? []
    : Array.from({ length: latestYear - earliestYear + 1 }, (_, i) =>
        String(latestYear - i),
      );
/** A limit as a field's hint; a limit the service does not send is none. */
const hint = (limit: number | null) => limit ?? undefined;

/** Asks for documents of one subject; every field but the subject is optional. */
export function RequestDialog({
  subject,
  choices,
  close,
  submit,
}: {
  subject: ExamSubjectPage;
  choices: ExamChoices;
  close: () => void;
  submit: (draft: ExamRequestDraft) => Promise<void>;
}) {
  const [classId, setClassId] = useState("");
  const [year, setYear] = useState("");
  const [occasion, setOccasion] = useState("");
  const [role, setRole] = useState("");
  const [description, setDescription] = useState("");
  const [sending, setSending] = useState(false);
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
    <Modal opened onClose={close} title={t("examShare.newRequest")}>
      <form
        onSubmit={async (event) => {
          event.preventDefault();
          setSending(true);
          await submit({
            subjectCode: subject.subject.code,
            classId: Number(classId) || null,
            academicYear: Number(year) || null,
            occasion: (occasion || null) as ExamOccasion | null,
            wantedRole: (role || null) as ExamRole | null,
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
                { value: "", label: t("examShare.allYears") },
                ...years(choices),
              ]}
            />
            <Select
              label={t("examShare.occasion")}
              value={occasion}
              onChange={setOccasion}
              data={[
                { value: "", label: t("examShare.allOccasions") },
                ...choices.requestOccasions.map((value) => ({
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
                ...choices.requestRoles.map((value) => ({
                  value,
                  label: t(`examShare.role.${value}`),
                })),
              ]}
            />
          </Field>
          <Field label={t("examShare.description")}>
            <UiTextarea
              rows={3}
              maxLength={hint(choices.limits.requestDescription)}
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
  choices,
  close,
  submit,
}: {
  choices: ExamChoices;
  close: () => void;
  submit: (reason: ExamReportReason, detail: string) => Promise<void>;
}) {
  const [reason, setReason] = useState(choices.reportReasons.at(0));
  const [detail, setDetail] = useState("");
  const [sending, setSending] = useState(false);
  return (
    <Modal opened onClose={close} title={t("examShare.report")}>
      <form
        onSubmit={async (event) => {
          event.preventDefault();
          if (!reason) return;
          setSending(true);
          await submit(reason, detail.trim());
          setSending(false);
        }}
      >
        <Stack>
          <Radio.Group
            label={t("examShare.reportReason")}
            value={reason ?? ""}
            onChange={(value) => setReason(value as ExamReportReason)}
          >
            {choices.reportReasons.map((value) => (
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
              maxLength={hint(choices.limits.reportDetail)}
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

/** Shares a past exam. Core owns the form: it starts it from a subject or a
 * request, applies the edits that follow a rule (subjects, role), checks the
 * files and merges photos into one PDF; this dialog keeps the choices. */
export function UploadDialog({
  form: initial,
  choices,
  running,
  status,
  run,
  submit,
  close,
  cancel,
}: {
  form: ExamUploadForm;
  choices: ExamChoices;
  running: boolean;
  status: ExamUploadStatus;
  run: (action: ExamAction) => Promise<ExamAnswer | undefined>;
  submit: (action: ExamAction) => Promise<void>;
  close: () => void;
  cancel: () => void;
}) {
  const [files, setFiles] = useState<ExamUploadFile[]>([]);
  const [form, setForm] = useState(initial);
  const [query, setQuery] = useState("");
  const [matches, setMatches] = useState<ExamSubject[]>();
  const [choosing, setChoosing] = useState(false);
  const [finding, setFinding] = useState(false);
  const [pickerError, setPickerError] = useState("");
  const { draft } = form;
  const { limits } = choices;
  const role = choices.roles.find((choice) => choice.role === draft.role);
  const occasions = role?.occasions ?? [];
  const answerSources = role?.answerSources ?? [];
  const pending = status.phase === "pending";
  const waiting = running || choosing || finding;
  const disabled = waiting || pending;
  const teachersDisabled = disabled || draft.allTeachers;
  const numbered =
    draft.occasion !== null && choices.numbered.includes(draft.occasion);

  // Core checks the file selection; the service checks the rest at send.
  const [checked, setChecked] = useState<{
    files: ExamUploadFile[];
    readiness: ExamUploadReadiness;
  }>();
  const readiness = checked?.files === files ? checked.readiness : undefined;
  const canSubmit = !disabled && readiness?.reasonKey === null;
  useEffect(() => {
    if (choosing) return;
    let current = true;
    void run({ kind: "uploadReadiness", files }).then((answer) => {
      if (current && answer?.kind === "uploadReadiness")
        setChecked({ files, readiness: answer.value });
    });
    return () => {
      current = false;
    };
  }, [choosing, files, run]);

  const update = (change: Partial<ExamUploadDraft>) =>
    setForm((previous) => ({
      ...previous,
      draft: { ...previous.draft, ...change },
    }));

  /** Core applies the edits that follow a rule and answers the whole form. */
  const edit = async (change: ExamUploadEdit) => {
    setFinding(true);
    try {
      const answer = await run({ kind: "editUpload", form, edit: change });
      if (answer?.kind === "uploadForm") setForm(answer.value);
      return answer?.kind === "uploadForm";
    } finally {
      setFinding(false);
    }
  };

  const chooseFiles = async () => {
    setChoosing(true);
    setPickerError("");
    try {
      const pick = desktopBridge?.examUploadPickFiles;
      if (!pick) throw new Error(t("examShare.upload.failedFiles"));
      const picked = await pick(t("examShare.upload.chooseFiles"));
      // Each pick adds pages after the ones already chosen; a file picked
      // again keeps its place.
      if (picked)
        setFiles((previous) => [
          ...previous,
          ...picked.filter(
            (file) => !previous.some(({ path }) => path === file.path),
          ),
        ]);
    } catch (failure) {
      setPickerError(errorText(failure) ?? t("examShare.upload.failedFiles"));
    } finally {
      setChoosing(false);
    }
  };

  /** Pages go into the PDF in this order; a chip moves one place. */
  const move = (from: number, to: number) =>
    setFiles((previous) => {
      const next = previous.slice();
      [next[from], next[to]] = [next[to], next[from]];
      return next;
    });

  const searchSubjects = async () => {
    if (disabled || !query.trim()) return;
    setFinding(true);
    try {
      const answer = await run({ kind: "search", query });
      if (answer?.kind === "subjects") {
        const selected = new Set(draft.subjectCodes);
        setMatches(answer.value.filter((match) => !selected.has(match.code)));
      }
    } finally {
      setFinding(false);
    }
  };

  const setName = (
    index: number,
    key: "familyName" | "givenName",
    value: string,
  ) =>
    update({
      newTeachers: draft.newTeachers.map((entry, position) =>
        position === index ? { ...entry, [key]: value } : entry,
      ),
    });

  return (
    <Modal
      opened
      onClose={() => {
        if (!waiting) close();
      }}
      closeOnClickOutside={false}
      closeOnEscape={!waiting}
      withCloseButton={!waiting}
      size="lg"
      title={
        form.request
          ? t("examShare.upload.answerRequest")
          : t("examShare.uploadTitle", {
              subject: form.subjects[0]?.subject.name ?? "",
            })
      }
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (canSubmit) void submit({ kind: "upload", files, draft });
        }}
      >
        <Stack>
          <Field
            label={t("examShare.uploadFiles")}
            description={t("examShare.upload.fileSelection")}
            error={pickerError || undefined}
          >
            <ol
              className="examUploadFiles"
              aria-label={t("examShare.upload.fileOrder")}
            >
              {files.map((file, index) => (
                <li key={file.path} className="examUploadFile">
                  {files.length > 1 && <small>{index + 1}</small>}
                  <span title={file.name}>{file.name}</span>
                  {files.length > 1 && (
                    <>
                      <button
                        type="button"
                        disabled={disabled || index === 0}
                        aria-label={`${t("examShare.upload.moveUp")} ${file.name}`}
                        title={t("examShare.upload.moveUp")}
                        onClick={() => move(index, index - 1)}
                      >
                        <ChevronLeft size={12} aria-hidden />
                      </button>
                      <button
                        type="button"
                        disabled={disabled || index + 1 === files.length}
                        aria-label={`${t("examShare.upload.moveDown")} ${file.name}`}
                        title={t("examShare.upload.moveDown")}
                        onClick={() => move(index, index + 1)}
                      >
                        <ChevronRight size={12} aria-hidden />
                      </button>
                    </>
                  )}
                  <button
                    type="button"
                    disabled={disabled}
                    aria-label={`${t("common.remove")} ${file.name}`}
                    title={t("common.remove")}
                    onClick={() =>
                      setFiles(files.filter(({ path }) => path !== file.path))
                    }
                  >
                    <X size={12} aria-hidden />
                  </button>
                </li>
              ))}
              <li>
                <Button
                  variant="default"
                  size="compact-sm"
                  loading={choosing}
                  disabled={disabled}
                  onClick={() => void chooseFiles()}
                >
                  {t("examShare.upload.chooseFiles")}
                </Button>
              </li>
            </ol>
          </Field>
          <Field label={t("examShare.upload.subjects")}>
            <div className="examUploadFiles">
              {form.subjects.map(({ subject, removable }) => (
                <span key={subject.code} className="examUploadFile">
                  <span title={subject.code}>{subject.name}</span>
                  <small>{subject.code}</small>
                  {removable && (
                    <button
                      type="button"
                      disabled={disabled}
                      aria-label={`${t("common.remove")} ${subject.name}`}
                      title={t("common.remove")}
                      onClick={() =>
                        void edit({ kind: "removeSubject", code: subject.code })
                      }
                    >
                      <X size={12} aria-hidden />
                    </button>
                  )}
                </span>
              ))}
            </div>
            {(limits.subjects === null ||
              form.subjects.length < limits.subjects) && (
              <Input
                type="search"
                className="examUploadSearch"
                maxLength={hint(limits.searchQuery)}
                value={query}
                disabled={disabled}
                aria-label={t("examShare.upload.addSubject")}
                placeholder={t("examShare.upload.addSubject")}
                onChange={(event) => {
                  setQuery(event.currentTarget.value);
                  setMatches(undefined);
                }}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    void searchSubjects();
                  }
                }}
                rightSection={
                  <button
                    type="button"
                    className="examUploadSearchGo"
                    disabled={disabled || !query.trim()}
                    aria-label={t("examShare.upload.addSubject")}
                    onClick={() => void searchSubjects()}
                  >
                    <Plus size={14} strokeWidth={1.75} aria-hidden />
                  </button>
                }
              />
            )}
            {matches &&
              (matches.length ? (
                <ul className="examList">
                  {matches.map((match) => (
                    <li key={match.code} className="examRow">
                      <button
                        type="button"
                        className="examRowMain"
                        disabled={disabled}
                        onClick={async () => {
                          if (
                            await edit({ kind: "addSubject", code: match.code })
                          ) {
                            setMatches(undefined);
                            setQuery("");
                          }
                        }}
                      >
                        <strong>{match.name}</strong>
                        <small>{match.code}</small>
                      </button>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="examQuiet">{t("examShare.noSubjects")}</p>
              ))}
          </Field>
          <Select
            label={t("examShare.upload.role")}
            value={draft.role}
            disabled={disabled}
            onChange={(value) =>
              void edit({ kind: "role", role: value as ExamRole })
            }
            data={choices.roles.map(({ role: value }) => ({
              value,
              label: t(`examShare.role.${value}`),
            }))}
          />
          {draft.role === "other" && (
            <TextInput
              label={t("examShare.upload.roleText")}
              maxLength={hint(limits.metadataText)}
              disabled={disabled}
              value={draft.roleText}
              onChange={(e) => update({ roleText: e.currentTarget.value })}
            />
          )}
          <Group grow>
            <Select
              label={t("examShare.year")}
              value={draft.academicYear ? String(draft.academicYear) : ""}
              disabled={disabled}
              onChange={(value) =>
                update({ academicYear: Number(value) || null })
              }
              data={[
                ...years(choices),
                { value: "", label: t("examShare.unknownYear") },
              ]}
            />
            {/* A role with one occasion has it set by Core. The picker
                starts again when Core clears the occasion, since a select
                with no value keeps its last pick. */}
            {occasions.length > 1 && (
              <Select
                key={draft.occasion === null ? "unpicked" : "picked"}
                label={t("examShare.occasion")}
                value={draft.occasion}
                placeholder={t("examShare.upload.pickOccasion")}
                disabled={disabled}
                onChange={(value) =>
                  update({ occasion: value as ExamOccasion })
                }
                data={occasions.map((value) => ({
                  value,
                  label: t(`examShare.occasion.${value}`),
                }))}
              />
            )}
            {numbered && limits.occasionNumberMax !== null && (
              <Select
                label={t("examShare.upload.occasionNumber")}
                value={String(draft.occasionNumber)}
                disabled={disabled}
                onChange={(value) => update({ occasionNumber: Number(value) })}
                data={[
                  { value: "0", label: t("examShare.upload.noNumber") },
                  ...Array.from({ length: limits.occasionNumberMax }, (_, i) =>
                    String(i + 1),
                  ),
                ]}
              />
            )}
          </Group>
          {draft.occasion === "other" && (
            <TextInput
              label={t("examShare.upload.occasionText")}
              maxLength={hint(limits.metadataText)}
              disabled={disabled}
              value={draft.occasionText}
              onChange={(e) => update({ occasionText: e.currentTarget.value })}
            />
          )}
          <Field label={t("examShare.teacher")}>
            <Checkbox
              label={t("examShare.allTeachers")}
              checked={draft.allTeachers}
              disabled={disabled}
              onCheckedChange={(checked) =>
                update(
                  checked === true
                    ? { allTeachers: true, teacherIds: [], newTeachers: [] }
                    : { allTeachers: false },
                )
              }
            />
            {form.teachers.length > 0 && (
              <Chip.Group
                multiple
                value={draft.teacherIds}
                disabled={teachersDisabled}
                onChange={(ids) => {
                  if (limits.teachers === null || ids.length <= limits.teachers)
                    update({ teacherIds: ids });
                }}
              >
                {form.teachers.map((teacher) => (
                  <Chip key={teacher.id} value={teacher.id}>
                    {teacher.name}
                  </Chip>
                ))}
              </Chip.Group>
            )}
            {draft.newTeachers.map((teacher, index) => (
              <Group key={index} className="examUploadTeacher">
                <TextInput
                  aria-label={t("examShare.upload.familyName")}
                  placeholder={t("examShare.upload.familyName")}
                  maxLength={hint(limits.metadataText)}
                  value={teacher.familyName}
                  disabled={teachersDisabled}
                  onChange={(e) =>
                    setName(index, "familyName", e.currentTarget.value)
                  }
                />
                <TextInput
                  aria-label={t("examShare.upload.givenName")}
                  placeholder={t("examShare.upload.givenName")}
                  maxLength={hint(limits.metadataText)}
                  value={teacher.givenName}
                  disabled={teachersDisabled}
                  onChange={(e) =>
                    setName(index, "givenName", e.currentTarget.value)
                  }
                />
                <Button
                  variant="subtle"
                  color="gray"
                  size="compact-sm"
                  disabled={teachersDisabled}
                  aria-label={t("common.remove")}
                  title={t("common.remove")}
                  onClick={() =>
                    update({
                      newTeachers: draft.newTeachers.filter(
                        (_, position) => position !== index,
                      ),
                    })
                  }
                >
                  <X size={14} aria-hidden />
                </Button>
              </Group>
            ))}
            <Button
              variant="subtle"
              size="compact-sm"
              className="examUploadAdd"
              disabled={
                teachersDisabled ||
                (limits.newTeachers !== null &&
                  draft.newTeachers.length >= limits.newTeachers)
              }
              onClick={() =>
                update({
                  newTeachers: [
                    ...draft.newTeachers,
                    { familyName: "", givenName: "" },
                  ],
                })
              }
            >
              {t("examShare.upload.newTeacher")}
            </Button>
          </Field>
          <Field label={t("examShare.upload.provenance")}>
            <SegmentedControl
              value={draft.provenance}
              disabled={disabled}
              onChange={(value) =>
                update({ provenance: value as ExamProvenance })
              }
              data={choices.provenances.map((value) => ({
                value,
                label: t(`examShare.upload.${value}` as MessageKey),
              }))}
            />
          </Field>
          {answerSources.length > 0 && (
            <Field label={t("examShare.upload.answerSource")}>
              <SegmentedControl
                value={draft.answerSource ?? ""}
                disabled={disabled}
                onChange={(value) =>
                  update({ answerSource: value as ExamAnswerSource })
                }
                data={answerSources.map((value) => ({
                  value,
                  label: t(`examShare.upload.${value}` as MessageKey),
                }))}
              />
            </Field>
          )}
          <Group grow>
            <TextInput
              label={t("examShare.upload.variant")}
              maxLength={hint(limits.metadataText)}
              disabled={disabled}
              value={draft.variant}
              onChange={(e) => update({ variant: e.currentTarget.value })}
            />
            <TextInput
              label={t("examShare.upload.note")}
              maxLength={hint(limits.note)}
              disabled={disabled}
              value={draft.note}
              onChange={(e) => update({ note: e.currentTarget.value })}
            />
          </Group>
          <p className="examQuiet">{t("examShare.uploadPublic")}</p>
          {/* An empty selection is where the dialog starts, not a mistake;
              the disabled send button says enough. */}
          {readiness?.reasonKey &&
            !(
              files.length === 0 &&
              readiness.reasonKey === "error.exam-share-upload-no-files"
            ) &&
            !pending &&
            !running && (
              <Text size="sm" c="red" role="status">
                {lookup(readiness.reasonKey)}
              </Text>
            )}
          {running && <UploadProgress status={status} cancel={cancel} />}
          {pending && !running && (
            <UploadRecovery
              recover={() => void submit({ kind: "recoverUpload" })}
            />
          )}
          <Group justify="flex-end">
            <Button variant="default" disabled={waiting} onClick={close}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" loading={running} disabled={!canSubmit}>
              {t("examShare.upload.title")}
            </Button>
          </Group>
        </Stack>
      </form>
    </Modal>
  );
}

/** Where an upload is: its phase, how much is sent, and a way to stop it
 * while Core can. */
export function UploadProgress({
  status,
  cancel,
}: {
  status: ExamUploadStatus;
  cancel: () => void;
}) {
  const { phase } = status;
  const label =
    phase === "idle" || phase === "pending"
      ? t("common.loading")
      : t(`examShare.upload.${phase}`);
  return (
    <div className="examUploadStatus" role="status">
      <span>{label}</span>
      {phase === "transferring" && status.total > 0 && (
        <Progress
          aria-label={label}
          className="examUploadBar"
          value={(status.sent / status.total) * 100}
        />
      )}
      {status.cancellable && (
        <Button variant="subtle" size="compact-sm" onClick={cancel}>
          {t("common.cancel")}
        </Button>
      )}
    </div>
  );
}

/** An upload Core could not finish; recovering registers or clears it. */
export function UploadRecovery({ recover }: { recover: () => void }) {
  return (
    <div className="examUploadStatus" role="status">
      <span>{t("examShare.upload.pending")}</span>
      <Button variant="subtle" size="compact-sm" onClick={recover}>
        {t("examShare.upload.recover")}
      </Button>
    </div>
  );
}
