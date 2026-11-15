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
  ExamRequest,
  ExamSubject,
  ExamTeacher,
  ExamUploadDraft,
  ExamUploadFile,
  ExamUploadOptions,
  ExamUploadStatus,
} from "../../generated/persistence";

export const examOccasions = [
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

const roles = ["question", "answer", "both", "notes", "other"] as const;
const origins = ["original", "reconstructed"] as const;
const answerSources = ["official", "student", "unknown"] as const;
const numberedOccasions = ["quiz", "check", "report"];
const imageExtension = /\.(?:jpe?g|png|webp)$/i;

type UploadSubject = {
  subject: ExamSubject;
  teachers: ExamTeacher[];
};

type Props = {
  options: ExamUploadOptions;
  subject: ExamSubject;
  request: ExamRequest | null;
  subjectLabel: (subject: ExamSubject) => string;
  running: boolean;
  status: ExamUploadStatus | undefined;
  error: string;
  run: (action: ExamAction) => Promise<ExamAnswer | undefined>;
  submit: (action: ExamAction) => Promise<void>;
  close: () => void;
  cancel: () => void;
};

export function ExamUploadDialog({
  options,
  subject,
  request,
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
  const [draft, setDraft] = useState(options.draft);
  const [subjects, setSubjects] = useState<UploadSubject[]>([
    {
      subject: {
        ...subject,
        code: options.draft.subjectCodes[0] ?? subject.code,
      },
      teachers: options.teachers,
    },
  ]);
  const [query, setQuery] = useState("");
  const [matches, setMatches] = useState<ExamSubject[]>();
  const [choosing, setChoosing] = useState(false);
  const [finding, setFinding] = useState(false);
  const [pickerError, setPickerError] = useState("");
  const teachers = [
    ...new Map(
      subjects.flatMap((entry) =>
        entry.teachers.map((teacher) => [teacher.id, teacher] as const),
      ),
    ).values(),
  ];
  const pending = status?.phase === "pending";
  const waiting = running || choosing || finding;
  const disabled = waiting || pending;
  const mixedFiles =
    files.length > 1 && files.some((file) => !imageExtension.test(file.name));
  const isNonExam = draft.role === "notes" || draft.role === "other";
  const needsSource = draft.role === "answer" || draft.role === "both";

  const update = <K extends keyof ExamUploadDraft>(
    key: K,
    value: ExamUploadDraft[K],
  ) => setDraft((previous) => ({ ...previous, [key]: value }));

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
    if (disabled || !query.trim() || subjects.length === 10) return;
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
    setFinding(true);
    try {
      const answer = await run({
        kind: "uploadOptions",
        subjectCode: match.code,
        request: null,
      });
      if (answer?.kind === "uploadOptions") {
        const code = answer.value.draft.subjectCodes[0] ?? match.code;
        if (draft.subjectCodes.includes(code)) {
          setMatches((previous) =>
            previous?.filter((entry) => entry.code !== match.code),
          );
          return;
        }
        const next = [
          ...subjects,
          { subject: { ...match, code }, teachers: answer.value.teachers },
        ];
        setSubjects(next);
        update(
          "subjectCodes",
          next.map(({ subject }) => subject.code),
        );
        setMatches(undefined);
        setQuery("");
      }
    } finally {
      setFinding(false);
    }
  };

  const removeSubject = (code: string) => {
    const next = subjects.filter((entry) => entry.subject.code !== code);
    const availableTeachers = new Set(
      next.flatMap((entry) => entry.teachers.map(({ id }) => id)),
    );
    setSubjects(next);
    setDraft((previous) => ({
      ...previous,
      subjectCodes: next.map(({ subject }) => subject.code),
      teacherIds: previous.teacherIds.filter((id) => availableTeachers.has(id)),
    }));
  };

  const setRole = (role: string) => {
    setDraft((previous) => {
      const nonExam = role === "notes" || role === "other";
      const occasion = nonExam
        ? "none"
        : previous.occasion === "none"
          ? "unknown"
          : previous.occasion;
      return {
        ...previous,
        role,
        occasion,
        occasionNumber: nonExam ? 0 : previous.occasionNumber,
        answerSource:
          role === "answer" || role === "both"
            ? (previous.answerSource ?? "unknown")
            : null,
      };
    });
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
        request ? "examShare.upload.answerRequest" : "examShare.upload.title",
      )}
      size="lg"
    >
      <form
        className="examUpload"
        onSubmit={(event) => {
          event.preventDefault();
          if (!disabled && files.length > 0 && !mixedFiles) {
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
        {mixedFiles && (
          <p role="alert">{t("examShare.upload.fileSelection")}</p>
        )}
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
            {subjects.map(({ subject: selected }, index) => (
              <li key={selected.code}>
                <span>
                  {subjectLabel(selected)} <small>{selected.code}</small>
                </span>
                {index > 0 && (
                  <UiButton
                    disabled={disabled}
                    onClick={() => removeSubject(selected.code)}
                  >
                    {t("common.remove")}
                  </UiButton>
                )}
              </li>
            ))}
          </ul>
          {subjects.length < 10 && (
            <div className="examShareActions">
              <TextInput
                type="search"
                maxLength={100}
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
          {teachers.length > 0 && (
            <div className="examUploadTeacherList">
              {teachers.map((teacher) => (
                <Checkbox
                  key={teacher.id}
                  label={teacher.name}
                  checked={draft.teacherIds.includes(teacher.id)}
                  disabled={
                    disabled ||
                    (!draft.teacherIds.includes(teacher.id) &&
                      draft.teacherIds.length >= 40)
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
                maxLength={100}
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
                maxLength={100}
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
              disabled={disabled || draft.newTeachers.length >= 40}
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
            onChange={setRole}
            disabled={disabled}
            data={roles.map((value) => ({
              value,
              label: t(`examShare.role.${value}`),
            }))}
          />
          <UiSelect
            label={t("examShare.upload.provenance")}
            value={draft.provenance}
            onChange={(value) => update("provenance", value)}
            disabled={disabled}
            data={origins.map((value) => ({
              value,
              label: t(`examShare.upload.${value}`),
            }))}
          />
          {needsSource && (
            <UiSelect
              label={t("examShare.upload.answerSource")}
              value={draft.answerSource ?? "unknown"}
              onChange={(value) => update("answerSource", value)}
              disabled={disabled}
              data={answerSources.map((value) => ({
                value,
                label: t(`examShare.upload.${value}`),
              }))}
            />
          )}
          <TextInput
            label={t("examShare.year")}
            type="number"
            min={1950}
            max={new Date().getFullYear() + 1}
            value={draft.academicYear ?? ""}
            disabled={disabled}
            onChange={(event) =>
              update("academicYear", Number(event.currentTarget.value) || null)
            }
          />
          {!isNonExam && (
            <UiSelect
              label={t("examShare.occasion")}
              value={draft.occasion}
              onChange={(occasion) =>
                setDraft((previous) => ({
                  ...previous,
                  occasion,
                  occasionNumber: numberedOccasions.includes(occasion)
                    ? previous.occasionNumber
                    : 0,
                }))
              }
              disabled={disabled}
              data={[...examOccasions, "unknown" as const].map((value) => ({
                value,
                label: t(`examShare.occasion.${value}`),
              }))}
            />
          )}
          {numberedOccasions.includes(draft.occasion) && (
            <TextInput
              label={t("examShare.upload.occasionNumber")}
              type="number"
              min={0}
              max={30}
              value={draft.occasionNumber}
              disabled={disabled}
              onChange={(event) =>
                update("occasionNumber", Number(event.currentTarget.value))
              }
            />
          )}
          <TextInput
            label={t("examShare.upload.variant")}
            maxLength={100}
            disabled={disabled}
            value={draft.variant}
            onChange={(event) => update("variant", event.currentTarget.value)}
          />
          <TextInput
            label={t("examShare.upload.note")}
            maxLength={40}
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
            disabled={disabled || !files.length || mixedFiles}
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
  status: ExamUploadStatus | undefined;
  cancel: () => void;
}) {
  const phase = status?.phase ?? "idle";
  const label =
    phase === "idle" || phase === "pending"
      ? t("common.loading")
      : t(`examShare.upload.${phase}`);
  return (
    <div className="examShareActions" role="status">
      <span>{label}</span>
      {status?.phase === "transferring" && status.total > 0 && (
        <progress aria-label={label} max={status.total} value={status.sent} />
      )}
      {(phase === "preparing" || phase === "transferring") && (
        <UiButton onClick={cancel}>{t("common.cancel")}</UiButton>
      )}
    </div>
  );
}
