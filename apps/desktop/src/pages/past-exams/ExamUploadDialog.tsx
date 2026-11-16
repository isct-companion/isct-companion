import { useState } from "react";
import { desktopBridge } from "../../app/native";
import { errorText } from "../../app/shared";
import {
  Checkbox,
  Modal,
  TextInput,
  UiButton,
  UiSelect,
} from "../../components/ui";
import { t } from "../../i18n";
import type {
  ExamAction,
  ExamAnswer,
  ExamAnswerSource,
  ExamChoices,
  ExamOccasion,
  ExamProvenance,
  ExamRole,
  ExamSubject,
  ExamUploadDraft,
  ExamUploadEdit,
  ExamUploadFile,
  ExamUploadForm,
  ExamUploadStatus,
} from "../../generated/persistence";

type Props = {
  form: ExamUploadForm;
  choices: ExamChoices;
  subjectLabel: (subject: ExamSubject) => string;
  running: boolean;
  status: ExamUploadStatus;
  error: string;
  run: (action: ExamAction) => Promise<ExamAnswer | undefined>;
  submit: (action: ExamAction) => Promise<void>;
  close: () => void;
  cancel: () => void;
};

export function ExamUploadDialog({
  form: initial,
  choices,
  subjectLabel,
  running,
  status,
  error,
  run,
  submit,
  close,
  cancel,
}: Props) {
  const [files, setFiles] = useState<ExamUploadFile[]>([]);
  const [form, setForm] = useState(initial);
  const [query, setQuery] = useState("");
  const [matches, setMatches] = useState<ExamSubject[]>();
  const [choosing, setChoosing] = useState(false);
  const [finding, setFinding] = useState(false);
  const [pickerError, setPickerError] = useState("");
  const { draft } = form;
  const { limits } = choices;
  // Core's choices list every role; the draft's role is always among them.
  const role = choices.roles.find((choice) => choice.role === draft.role)!;
  const pending = status.phase === "pending";
  const waiting = running || choosing || finding;
  const disabled = waiting || pending;

  const update = <K extends keyof ExamUploadDraft>(
    key: K,
    value: ExamUploadDraft[K],
  ) =>
    setForm((previous) => ({
      ...previous,
      draft: { ...previous.draft, [key]: value },
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
      if (picked) setFiles(picked);
    } catch (failure) {
      setPickerError(errorText(failure) ?? t("examShare.upload.failedFiles"));
    } finally {
      setChoosing(false);
    }
  };

  const moveFile = (from: number, to: number) => {
    setFiles((previous) => {
      const next = previous.slice();
      [next[from], next[to]] = [next[to], next[from]];
      return next;
    });
  };

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

  const addSubject = async (match: ExamSubject) => {
    if (await edit({ kind: "addSubject", code: match.code })) {
      setMatches(undefined);
      setQuery("");
    }
  };

  return (
    <Modal
      opened
      onClose={() => {
        if (!waiting) close();
      }}
      closeOnClickOutside={false}
      closeOnEscape={!waiting}
      withCloseButton={!waiting}
      title={t(
        form.request
          ? "examShare.upload.answerRequest"
          : "examShare.upload.title",
      )}
      size="lg"
    >
      <form
        className="examUpload"
        onSubmit={(event) => {
          event.preventDefault();
          if (!disabled && files.length > 0) {
            void submit({ kind: "upload", files, draft });
          }
        }}
      >
        <div className="examShareActions">
          <UiButton disabled={disabled} onClick={() => void chooseFiles()}>
            {t("examShare.upload.chooseFiles")}
          </UiButton>
        </div>
        {pickerError && <p role="alert">{pickerError}</p>}
        {files.length > 0 && (
          <ol
            className="examImportList examUploadList"
            aria-label={t("examShare.upload.fileOrder")}
          >
            {files.map((file, index) => (
              <li key={file.path}>
                <span>{file.name}</span>
                <span className="examShareActions">
                  {files.length > 1 && (
                    <>
                      <UiButton
                        disabled={disabled || index === 0}
                        onClick={() => moveFile(index, index - 1)}
                      >
                        {t("examShare.upload.moveUp")}
                      </UiButton>
                      <UiButton
                        disabled={disabled || index + 1 === files.length}
                        onClick={() => moveFile(index, index + 1)}
                      >
                        {t("examShare.upload.moveDown")}
                      </UiButton>
                    </>
                  )}
                  <UiButton
                    disabled={disabled}
                    onClick={() =>
                      setFiles(files.filter(({ path }) => path !== file.path))
                    }
                  >
                    {t("common.remove")}
                  </UiButton>
                </span>
              </li>
            ))}
          </ol>
        )}
        <fieldset className="examUploadFields" disabled={disabled}>
          <legend>{t("examShare.upload.subjects")}</legend>
          <ul className="examImportList examUploadList">
            {form.subjects.map(({ subject: selected, removable }) => (
              <li key={selected.code}>
                <span>
                  {subjectLabel(selected)} <small>{selected.code}</small>
                </span>
                {removable && (
                  <UiButton
                    disabled={disabled}
                    onClick={() =>
                      void edit({ kind: "removeSubject", code: selected.code })
                    }
                  >
                    {t("common.remove")}
                  </UiButton>
                )}
              </li>
            ))}
          </ul>
          {form.subjects.length < limits.subjects && (
            <div className="examShareActions">
              <TextInput
                type="search"
                maxLength={limits.query}
                value={query}
                aria-label={t("examShare.search")}
                placeholder={t("examShare.search")}
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
              />
              <UiButton
                disabled={disabled || !query.trim()}
                onClick={() => void searchSubjects()}
              >
                {t("examShare.upload.addSubject")}
              </UiButton>
            </div>
          )}
          {matches &&
            (matches.length ? (
              <ul className="examImportList">
                {matches.map((match) => (
                  <li key={match.code}>
                    <UiButton
                      className="textButton"
                      type="button"
                      disabled={disabled}
                      onClick={() => void addSubject(match)}
                    >
                      {subjectLabel(match)} <small>{match.code}</small>
                    </UiButton>
                  </li>
                ))}
              </ul>
            ) : (
              <p>{t("examShare.noItems")}</p>
            ))}
        </fieldset>
        <fieldset className="examUploadFields" disabled={disabled}>
          <legend>{t("examShare.teacher")}</legend>
          {form.teachers.length > 0 && (
            <div className="examUploadTeacherList">
              {form.teachers.map((teacher) => (
                <Checkbox
                  key={teacher.id}
                  label={teacher.name}
                  checked={draft.teacherIds.includes(teacher.id)}
                  disabled={
                    disabled ||
                    (!draft.teacherIds.includes(teacher.id) &&
                      draft.teacherIds.length >= limits.teachers)
                  }
                  onCheckedChange={(checked) =>
                    update(
                      "teacherIds",
                      checked
                        ? [...draft.teacherIds, teacher.id]
                        : draft.teacherIds.filter((id) => id !== teacher.id),
                    )
                  }
                />
              ))}
            </div>
          )}
          {draft.newTeachers.map((teacher, index) => (
            <div className="examShareActions" key={index}>
              <TextInput
                label={t("examShare.upload.familyName")}
                required
                maxLength={limits.teacherName}
                value={teacher.familyName}
                onChange={(event) =>
                  update(
                    "newTeachers",
                    draft.newTeachers.map((entry, position) =>
                      position === index
                        ? { ...entry, familyName: event.currentTarget.value }
                        : entry,
                    ),
                  )
                }
              />
              <TextInput
                label={t("examShare.upload.givenName")}
                maxLength={limits.teacherName}
                value={teacher.givenName}
                onChange={(event) =>
                  update(
                    "newTeachers",
                    draft.newTeachers.map((entry, position) =>
                      position === index
                        ? { ...entry, givenName: event.currentTarget.value }
                        : entry,
                    ),
                  )
                }
              />
              <UiButton
                disabled={disabled}
                onClick={() =>
                  update(
                    "newTeachers",
                    draft.newTeachers.filter(
                      (_, position) => position !== index,
                    ),
                  )
                }
              >
                {t("common.remove")}
              </UiButton>
            </div>
          ))}
          <div>
            <UiButton
              disabled={disabled || draft.newTeachers.length >= limits.teachers}
              onClick={() =>
                update("newTeachers", [
                  ...draft.newTeachers,
                  { familyName: "", givenName: "" },
                ])
              }
            >
              {t("examShare.upload.newTeacher")}
            </UiButton>
          </div>
        </fieldset>
        <div className="examUploadMetadata">
          <UiSelect
            label={t("examShare.upload.role")}
            value={draft.role}
            onChange={(value) =>
              void edit({ kind: "role", role: value as ExamRole })
            }
            disabled={disabled}
            data={choices.roles.map(({ role: value }) => ({
              value,
              label: t(`examShare.role.${value}`),
            }))}
          />
          <UiSelect
            label={t("examShare.upload.provenance")}
            value={draft.provenance}
            onChange={(value) => update("provenance", value as ExamProvenance)}
            disabled={disabled}
            data={choices.provenances.map((value) => ({
              value,
              label: t(`examShare.upload.${value}`),
            }))}
          />
          {role.answerSources.length > 0 && (
            <UiSelect
              label={t("examShare.upload.answerSource")}
              value={draft.answerSource ?? undefined}
              onChange={(value) =>
                update("answerSource", value as ExamAnswerSource)
              }
              disabled={disabled}
              data={role.answerSources.map((value) => ({
                value,
                label: t(`examShare.upload.${value}`),
              }))}
            />
          )}
          <TextInput
            label={t("examShare.year")}
            type="number"
            min={limits.earliestYear}
            max={limits.latestYear}
            value={draft.academicYear ?? ""}
            disabled={disabled}
            onChange={(event) =>
              update("academicYear", Number(event.currentTarget.value) || null)
            }
          />
          {role.occasions.length !== 1 && (
            <UiSelect
              label={t("examShare.occasion")}
              value={draft.occasion}
              onChange={(value) => update("occasion", value as ExamOccasion)}
              disabled={disabled}
              data={role.occasions.map((value) => ({
                value,
                label: t(`examShare.occasion.${value}`),
              }))}
            />
          )}
          {choices.numbered.includes(draft.occasion) && (
            <TextInput
              label={t("examShare.upload.occasionNumber")}
              type="number"
              min={0}
              max={limits.occasionNumber}
              value={draft.occasionNumber}
              disabled={disabled}
              onChange={(event) =>
                update("occasionNumber", Number(event.currentTarget.value))
              }
            />
          )}
          <TextInput
            label={t("examShare.upload.variant")}
            maxLength={limits.variant}
            disabled={disabled}
            value={draft.variant}
            onChange={(event) => update("variant", event.currentTarget.value)}
          />
          <TextInput
            label={t("examShare.upload.note")}
            maxLength={limits.note}
            disabled={disabled}
            value={draft.note}
            onChange={(event) => update("note", event.currentTarget.value)}
          />
        </div>
        {error && <p role="alert">{error}</p>}
        {running && <ExamUploadProgress status={status} cancel={cancel} />}
        {pending && !running && (
          <div className="examShareActions">
            <p role="status">{t("examShare.upload.pending")}</p>
            <UiButton onClick={() => void submit({ kind: "recoverUpload" })}>
              {t("examShare.upload.recover")}
            </UiButton>
          </div>
        )}
        <footer className="examUploadFooter">
          <UiButton onClick={close} disabled={waiting}>
            {t("common.close")}
          </UiButton>
          <UiButton
            type="submit"
            className="button primary"
            disabled={disabled || !files.length}
          >
            {t("examShare.upload.title")}
          </UiButton>
        </footer>
      </form>
    </Modal>
  );
}

export function ExamUploadProgress({
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
    <div className="examShareActions" role="status">
      <span>{label}</span>
      {phase === "transferring" && status.total > 0 && (
        <progress aria-label={label} max={status.total} value={status.sent} />
      )}
      {status.cancellable && (
        <UiButton onClick={cancel}>{t("common.cancel")}</UiButton>
      )}
    </div>
  );
}
