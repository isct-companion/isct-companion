import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useRef,
  useState,
  type FormEvent,
} from "react";
import { UiButton, UiInput, UiSelect } from "../../components/ui";
import { confirmAction } from "../../components/confirm";
import { ExternalLink } from "../../components/ExternalLink";
import { desktopBridge } from "../../app/native";
import { errorText } from "../../app/shared";
import { errorCode } from "../../platform/native-error";
import { localFileUrl } from "../../features/lms/files";
import { lookup, t } from "../../i18n";
import type {
  ExamAccount,
  ExamAction,
  ExamAnswer,
  ExamDocument,
  ExamLibrary,
  ExamLocalDocument,
  ExamRequest,
  ExamRequestCursor,
  ExamRequestPage,
  ExamReportReason,
  ExamSubject,
  ExamSubjectPage,
  ExamUploadOptions,
  ExamUploadStatus,
} from "../../generated/persistence";
import {
  ExamUploadDialog,
  ExamUploadProgress,
  examOccasions as occasions,
} from "./ExamUploadDialog";
import { TimetableImportPreview } from "./TimetableImportPreview";
import "./past-exams.css";

const Pdf = lazy(() =>
  import("../../features/lms/PdfDocument").then((m) => ({
    default: m.PdfDocument,
  })),
);
const tabs = ["browse", "requests", "library"] as const;
const reasons = [
  "copyright",
  "inappropriate",
  "wrongAnswer",
  "brokenFile",
  "other",
] as const;
export function PastExams({
  active,
  courseId,
  currentTerm,
  terms,
}: {
  active: boolean;
  courseId?: string;
  currentTerm: string;
  terms: string[];
}) {
  const subjectLabel = (subject: ExamSubject) => subject.name;
  const examLabel = (document: ExamDocument) =>
    [
      document.subjects.map(subjectLabel).join(" / "),
      document.academicYear,
      [
        lookup(`examShare.occasion.${document.occasion}`),
        document.occasionNumber > 1 ? document.occasionNumber : null,
      ]
        .filter(Boolean)
        .join(" "),
      document.variant,
      lookup(`examShare.role.${document.role}`),
      document.teachers.map(({ name }) => name).join(" / "),
      document.note,
    ]
      .filter(Boolean)
      .join(" · ") || document.fileName;
  const [tab, setTab] = useState<(typeof tabs)[number]>("browse");
  const [account, setAccount] = useState<ExamAccount>();
  const [query, setQuery] = useState("");
  const [subjects, setSubjects] = useState<ExamSubject[]>([]);
  const [searched, setSearched] = useState(false);
  const [subject, setSubject] = useState<ExamSubjectPage>();
  const [file, setFile] = useState<ExamLocalDocument>();
  const [library, setLibrary] = useState<ExamLibrary>();
  const [requests, setRequests] = useState<ExamRequestPage>();
  const [mine, setMine] = useState(false);
  const [preview, setPreview] = useState(false);
  const [requestDraft, setRequestDraft] = useState(false);
  const [report, setReport] = useState(false);
  const [upload, setUpload] = useState<{
    options: ExamUploadOptions;
    subject: ExamSubject;
    request: ExamRequest | null;
  }>();
  const [uploadStatus, setUploadStatus] = useState<ExamUploadStatus>();
  const [uploadRunning, setUploadRunning] = useState(false);
  const closeUpload = useCallback(() => {
    setUpload(undefined);
    void desktopBridge?.examUploadClearFiles?.().catch(() => undefined);
  }, []);
  const [reason, setReason] = useState<ExamReportReason>(reasons[0]);
  const [year, setYear] = useState("");
  const [teacher, setTeacher] = useState("");
  const [occasion, setOccasion] = useState("");
  const [busy, setBusy] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const courseOpened = useRef(false);
  const pendingCalls = useRef(0);
  const applyAccount = useCallback(
    (next: ExamAccount) => {
      setAccount(next);
      if (next.status === "connected") return;
      courseOpened.current = false;
      closeUpload();
      setUploadStatus(undefined);
      setSubjects([]);
      setSubject(undefined);
      setFile(undefined);
      setLibrary(undefined);
      setRequests(undefined);
      setSearched(false);
      setReport(false);
      setRequestDraft(false);
    },
    [closeUpload],
  );
  const call = useCallback(
    async (action: ExamAction): Promise<ExamAnswer | undefined> => {
      pendingCalls.current += 1;
      setBusy(true);
      setConnecting(action.kind === "connect");
      setError("");
      setNotice("");
      try {
        const run = desktopBridge?.examShare;
        if (!run) throw new Error(t("error.exam-share-unavailable"));
        return await run(action);
      } catch (e) {
        if (errorCode(e) === "exam-share-cancelled") return undefined;
        setError(errorText(e) ?? t("error.exam-share-unavailable"));
        const snapshot = await desktopBridge
          ?.examShare?.({ kind: "accountSnapshot" })
          .catch(() => undefined);
        if (snapshot?.kind === "account") applyAccount(snapshot.value);
        return undefined;
      } finally {
        pendingCalls.current -= 1;
        if (pendingCalls.current === 0) {
          setConnecting(false);
          setBusy(false);
        }
      }
    },
    [applyAccount],
  );
  const accountAction = useCallback(
    async (kind: "account" | "connect" | "disconnect" | "acceptTerms") => {
      const answer = await call({ kind });
      if (answer?.kind === "account") applyAccount(answer.value);
    },
    [applyAccount, call],
  );
  useEffect(() => {
    if (!active) return;
    const refreshAccount = () => {
      if (pendingCalls.current === 0 && document.visibilityState !== "hidden")
        void accountAction("account");
    };
    refreshAccount();
    window.addEventListener("focus", refreshAccount);
    document.addEventListener("visibilitychange", refreshAccount);
    return () => {
      window.removeEventListener("focus", refreshAccount);
      document.removeEventListener("visibilitychange", refreshAccount);
    };
  }, [active, accountAction]);

  const loadLibrary = useCallback(async () => {
    const answer = await call({ kind: "library" });
    if (answer?.kind === "library") setLibrary(answer.value);
  }, [call]);
  const loadRequests = useCallback(
    async (personal: boolean, cursor: ExamRequestCursor | null = null) => {
      const answer = await call({ kind: "requests", mine: personal, cursor });
      if (answer?.kind === "requests") {
        setRequests((previous) => ({
          ...answer.value,
          items: cursor
            ? [...(previous?.items ?? []), ...answer.value.items]
            : answer.value.items,
        }));
      }
    },
    [call],
  );
  useEffect(() => {
    if (!active || account?.status !== "connected") return;
    if (tab === "requests") void loadRequests(mine);
    if (tab === "library") void loadLibrary();
  }, [active, account?.status, tab, mine, loadLibrary, loadRequests]);
  const switchTab = (next: (typeof tabs)[number]) => {
    setTab(next);
    setFile(undefined);
    setReport(false);
    setRequestDraft(false);
    setError("");
  };
  const open = async (id: string, download = false) => {
    const answer = await call({ kind: "open", id, download });
    if (answer?.kind === "document") setFile(answer.value);
  };
  const loadSubject = useCallback(
    async (
      action: Extract<ExamAction, { kind: "subject" | "courseSubject" }>,
    ) => {
      const answer = await call(action);
      if (answer?.kind === "subject") {
        setSubject(answer.value);
        setYear("");
        setTeacher("");
        setOccasion("");
      } else if (answer?.kind === "subjects") {
        setSubjects(answer.value);
        setSearched(true);
        setSubject(undefined);
      }
    },
    [call],
  );
  useEffect(() => {
    if (
      !active ||
      !courseId ||
      account?.status !== "connected" ||
      courseOpened.current
    )
      return;
    courseOpened.current = true;
    void loadSubject({ kind: "courseSubject", courseId });
  }, [active, account?.status, courseId, loadSubject]);
  useEffect(() => {
    if (!active || account?.status !== "connected") return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const read = async () => {
      const answer = await desktopBridge
        ?.examShare?.({ kind: "uploadStatus" })
        .catch(() => undefined);
      if (stopped) return;
      if (answer?.kind === "uploadStatus") setUploadStatus(answer.value);
      if (uploadRunning) timer = setTimeout(() => void read(), 300);
    };
    void read();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [active, account?.status, uploadRunning]);
  const openUpload = async (
    subject: ExamSubject,
    request: ExamRequest | null = null,
  ) => {
    const answer = await call({
      kind: "uploadOptions",
      subjectCode: subject.code,
      request,
    });
    if (answer?.kind === "uploadOptions") {
      setUpload({ options: answer.value, subject, request });
    }
  };
  const cancelUpload = () => {
    void desktopBridge
      ?.examShare?.({ kind: "cancelUpload" })
      .catch((failure) =>
        setError(errorText(failure) ?? t("error.exam-share-unavailable")),
      );
  };
  const submitUpload = async (action: ExamAction) => {
    setUploadRunning(true);
    try {
      const answer = await call(action);
      if (answer?.kind === "uploaded") {
        closeUpload();
        setLibrary(undefined);
        if (tab === "library") {
          await loadLibrary();
        } else if (tab === "requests") {
          await loadRequests(mine);
        } else if (subject) {
          const updated = await call({
            kind: "subject",
            code: subject.subject.code,
          });
          if (updated?.kind === "subject") setSubject(updated.value);
        }
        setNotice(t("examShare.upload.uploaded"));
      } else if (action.kind === "recoverUpload" && answer?.kind === "done") {
        setNotice(t("examShare.upload.cleaned"));
      }
    } finally {
      setUploadRunning(false);
    }
  };
  const submitSearch = async (event: FormEvent) => {
    event.preventDefault();
    const answer = await call({ kind: "search", query });
    if (answer?.kind === "subjects") {
      setSubjects(answer.value);
      setSearched(true);
      setSubject(undefined);
    }
  };
  const documents = (
    items: ExamDocument[],
    kind?: "downloaded" | "submitted",
  ) =>
    items.length ? (
      <ul className="examImportList">
        {items.map((entry) => (
          <li key={entry.id}>
            <UiButton
              disabled={busy}
              className="textButton"
              onClick={() => void open(entry.id)}
            >
              {examLabel(entry)}
            </UiButton>
            {kind && (
              <UiButton
                disabled={busy}
                onClick={async () => {
                  if (
                    !(await confirmAction(
                      t(
                        kind === "submitted"
                          ? "examShare.deleteSubmission"
                          : "examShare.removeDownload",
                      ),
                      t("common.delete"),
                    ))
                  )
                    return;
                  if (
                    await call({
                      kind:
                        kind === "submitted"
                          ? "deleteSubmission"
                          : "removeDownload",
                      id: entry.id,
                    })
                  )
                    await loadLibrary();
                }}
              >
                {t(
                  kind === "submitted"
                    ? "examShare.deleteSubmission"
                    : "examShare.removeDownload",
                )}
              </UiButton>
            )}
          </li>
        ))}
      </ul>
    ) : (
      <p>{t("examShare.noItems")}</p>
    );

  return (
    <section className="page" aria-busy={busy}>
      <header className="pageBar">
        <h1>{t("examShare.title")}</h1>
        <div className="pageBarRight">
          {account?.email && <span>{account.email}</span>}
          {account?.status === "connected" && (
            <UiButton
              disabled={busy}
              onClick={async () => {
                if (
                  await confirmAction(
                    t("examShare.disconnectHint"),
                    t("examShare.disconnect"),
                  )
                )
                  await accountAction("disconnect");
              }}
            >
              {t("examShare.disconnect")}
            </UiButton>
          )}
        </div>
      </header>
      {error && !upload && account?.status !== "unavailable" && (
        <p role="alert">{error}</p>
      )}
      {notice && <p role="status">{notice}</p>}
      {busy && !uploadRunning && <p role="status">{t("common.loading")}</p>}
      {!upload && uploadRunning && account?.status === "connected" && (
        <ExamUploadProgress status={uploadStatus} cancel={cancelUpload} />
      )}
      {!upload &&
        !uploadRunning &&
        account?.status === "connected" &&
        uploadStatus?.phase === "pending" && (
          <div className="examShareActions">
            <p role="status">{t("examShare.upload.pending")}</p>
            <UiButton
              disabled={busy}
              onClick={() => void submitUpload({ kind: "recoverUpload" })}
            >
              {t("examShare.upload.recover")}
            </UiButton>
          </div>
        )}
      {upload && account?.status === "connected" && (
        <ExamUploadDialog
          options={upload.options}
          subject={upload.subject}
          request={upload.request}
          subjectLabel={subjectLabel}
          running={uploadRunning}
          status={uploadStatus}
          error={error}
          run={call}
          submit={submitUpload}
          cancel={cancelUpload}
          close={closeUpload}
        />
      )}
      <nav aria-label={t("examShare.title")} className="examShareActions">
        {tabs.map((key) => (
          <UiButton
            key={key}
            disabled={busy}
            aria-pressed={key === tab}
            onClick={() => switchTab(key)}
          >
            {t(`examShare.${key}`)}
          </UiButton>
        ))}
      </nav>
      {account?.status === "needsConsent" && (
        <div className="examShareActions">
          <ExternalLink href="https://sciencetokyoexam.com/terms">
            {t("examShare.terms")}
          </ExternalLink>
          <ExternalLink href="https://sciencetokyoexam.com/privacy">
            {t("examShare.privacy")}
          </ExternalLink>
          <UiButton
            disabled={busy}
            onClick={() => void accountAction("acceptTerms")}
          >
            {t("examShare.accept")}
          </UiButton>
        </div>
      )}
      {(!account || account.status === "disconnected") && (
        <UiButton disabled={busy} onClick={() => void accountAction("connect")}>
          {t("examShare.connect")}
        </UiButton>
      )}
      {connecting && (
        <UiButton
          onClick={() => {
            void desktopBridge
              ?.examShare?.({ kind: "cancelConnect" })
              .catch((failure) =>
                setError(
                  errorText(failure) ?? t("error.exam-share-unavailable"),
                ),
              );
          }}
        >
          {t("common.cancel")}
        </UiButton>
      )}
      {account?.status === "unavailable" && (
        <p role="alert">{t("error.exam-share-unsupported")}</p>
      )}
      {tab === "browse" && !file && (
        <div>
          <UiButton
            disabled={busy}
            aria-expanded={preview}
            onClick={() => setPreview(!preview)}
          >
            {t("examShare.importPreview")}
          </UiButton>
          {preview && (
            <TimetableImportPreview
              terms={terms}
              currentTerm={currentTerm}
              active={active}
            />
          )}
        </div>
      )}
      {account?.status === "connected" &&
        (file ? (
          <>
            <div className="examShareActions">
              <UiButton
                disabled={busy}
                onClick={() => {
                  setReport(false);
                  setFile(undefined);
                  if (tab === "library") void loadLibrary();
                }}
              >
                {t("common.back")}
              </UiButton>
              <UiButton
                disabled={busy || file.downloaded || file.offline}
                onClick={() => void open(file.document.id, true)}
              >
                {t(
                  file.downloaded
                    ? "examShare.downloaded"
                    : "examShare.download",
                )}
              </UiButton>
              {file.saved !== null && (
                <UiButton
                  disabled={busy || file.offline}
                  onClick={async () => {
                    const answer = await call({
                      kind: "save",
                      saved: !file.saved,
                      id: file.document.id,
                    });
                    if (answer?.kind === "saved")
                      setFile({ ...file, saved: answer.value });
                  }}
                >
                  {t(file.saved ? "examShare.unsave" : "examShare.save")}
                </UiButton>
              )}
              <UiButton
                disabled={busy || file.offline}
                onClick={() => setReport(!report)}
              >
                {t("examShare.report")}
              </UiButton>
            </div>
            {file.offline && <p>{t("examShare.offline")}</p>}
            {report && (
              <form
                className="examShareActions"
                onSubmit={async (event) => {
                  event.preventDefault();
                  const data = new FormData(event.currentTarget);
                  if (
                    await call({
                      kind: "report",
                      id: file.document.id,
                      reason,
                      detail: String(data.get("detail") ?? ""),
                    })
                  ) {
                    setReport(false);
                    setNotice(t("examShare.reported"));
                  }
                }}
              >
                <UiSelect
                  aria-label={t("examShare.report")}
                  value={reason}
                  onChange={(value) => setReason(value as ExamReportReason)}
                  data={reasons.map((value) => ({
                    value,
                    label: t(`examShare.reason.${value}`),
                  }))}
                />
                <UiInput
                  name="detail"
                  maxLength={900}
                  placeholder={t("examShare.description")}
                />
                <UiButton type="submit" disabled={busy}>
                  {t("examShare.report")}
                </UiButton>
              </form>
            )}
            {file.document.fileType === "application/pdf" ? (
              <div className="examDocument">
                <Suspense fallback={t("common.loading")}>
                  <Pdf
                    src={localFileUrl(file.path)}
                    title={examLabel(file.document)}
                  />
                </Suspense>
              </div>
            ) : file.document.fileType === "text/plain" ? (
              <PlainTextPreview key={file.path} path={file.path} />
            ) : (
              <img
                className="examDocumentImage"
                src={localFileUrl(file.path)}
                alt={examLabel(file.document)}
              />
            )}
          </>
        ) : tab === "browse" ? (
          <>
            <form className="examShareActions" onSubmit={submitSearch}>
              <UiInput
                type="search"
                value={query}
                onChange={(e) => setQuery(e.currentTarget.value)}
                maxLength={100}
                placeholder={t("examShare.search")}
              />
              <UiButton type="submit" disabled={busy || !query.trim()}>
                {t("examShare.search")}
              </UiButton>
            </form>
            {subject ? (
              <>
                <header className="pageBar">
                  <h2>{subjectLabel(subject.subject)}</h2>
                  <div className="pageBarRight">
                    <UiButton
                      disabled={busy}
                      onClick={() => setSubject(undefined)}
                    >
                      {t("common.back")}
                    </UiButton>
                    <UiButton
                      disabled={busy}
                      onClick={() => void openUpload(subject.subject)}
                    >
                      {t("examShare.upload.title")}
                    </UiButton>
                    <UiButton
                      disabled={busy}
                      onClick={() => setRequestDraft(!requestDraft)}
                    >
                      {t("examShare.newRequest")}
                    </UiButton>
                  </div>
                </header>
                {requestDraft && (
                  <RequestForm
                    subject={subject}
                    disabled={busy}
                    submit={async (action) => {
                      if (await call(action)) {
                        setRequestDraft(false);
                        setNotice(t("examShare.requestCreated"));
                      }
                    }}
                  />
                )}
                <div className="examShareActions">
                  <UiSelect
                    aria-label={t("examShare.year")}
                    value={year}
                    onChange={setYear}
                    data={[
                      { value: "", label: t("examShare.allYears") },
                      ...[
                        ...new Set(
                          subject.exams.flatMap((exam) =>
                            exam.academicYear === null
                              ? []
                              : [exam.academicYear],
                          ),
                        ),
                      ]
                        .sort((a, b) => b - a)
                        .map(String),
                    ]}
                  />
                  <UiSelect
                    aria-label={t("examShare.teacher")}
                    value={teacher}
                    onChange={setTeacher}
                    data={[
                      { value: "", label: t("examShare.allTeachers") },
                      ...new Map(
                        subject.exams.flatMap((exam) =>
                          exam.teachers.map(
                            ({ id, name }) =>
                              [id, { value: id, label: name }] as const,
                          ),
                        ),
                      ).values(),
                    ]}
                  />
                  <UiSelect
                    aria-label={t("examShare.occasion")}
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
                </div>
                {documents(
                  subject.exams.filter(
                    (d) =>
                      (!year || String(d.academicYear) === year) &&
                      (!teacher || d.teachers.some((x) => x.id === teacher)) &&
                      (!occasion || d.occasion === occasion),
                  ),
                )}
              </>
            ) : (
              <ul className="examImportList">
                {subjects.map((entry) => (
                  <li key={entry.code}>
                    <UiButton
                      className="textButton"
                      disabled={busy}
                      onClick={() =>
                        void loadSubject({ kind: "subject", code: entry.code })
                      }
                    >
                      {subjectLabel(entry)}
                    </UiButton>
                    <small>{entry.code}</small>
                  </li>
                ))}
                {searched && !subjects.length && (
                  <li>{t("examShare.noItems")}</li>
                )}
              </ul>
            )}
          </>
        ) : tab === "requests" ? (
          <>
            <div className="examShareActions">
              <UiButton
                disabled={busy}
                aria-pressed={!mine}
                onClick={() => {
                  setMine(false);
                  if (mine) setRequests(undefined);
                }}
              >
                {t("examShare.openRequests")}
              </UiButton>
              <UiButton
                disabled={busy}
                aria-pressed={mine}
                onClick={() => {
                  setMine(true);
                  if (!mine) setRequests(undefined);
                }}
              >
                {t("examShare.myRequests")}
              </UiButton>
              <UiButton disabled={busy} onClick={() => switchTab("browse")}>
                {t("examShare.newRequest")}
              </UiButton>
            </div>
            {requests && !requests.items.length && (
              <p>{t("examShare.noItems")}</p>
            )}
            <ul className="examImportList">
              {requests?.items.map((item) => (
                <li key={item.id}>
                  <div>
                    <strong>{subjectLabel(item.subject)}</strong>
                    <small>
                      {[
                        item.academicYear,
                        lookup(`examShare.requestStatus.${item.status}`),
                        item.description,
                      ]
                        .filter(Boolean)
                        .join(" · ")}
                    </small>
                  </div>
                  <span className="examShareActions">
                    {item.status === "open" && (
                      <UiButton
                        disabled={busy}
                        onClick={() => void openUpload(item.subject, item)}
                      >
                        {t("examShare.upload.answerRequest")}
                      </UiButton>
                    )}
                    {item.answerExamId && (
                      <UiButton
                        disabled={busy}
                        onClick={() => void open(item.answerExamId!)}
                      >
                        {t("examShare.viewAnswer")}
                      </UiButton>
                    )}
                    {item.mine && (
                      <>
                        <UiButton
                          disabled={busy}
                          onClick={async () => {
                            if (
                              await call({
                                kind: "setRequestOpen",
                                id: item.id,
                                open: item.status !== "open",
                              })
                            )
                              await loadRequests(mine);
                          }}
                        >
                          {t(
                            item.status === "open"
                              ? "examShare.closeRequest"
                              : "examShare.reopenRequest",
                          )}
                        </UiButton>
                        <UiButton
                          disabled={busy}
                          onClick={async () => {
                            if (
                              (await confirmAction(
                                t("examShare.deleteRequest"),
                                t("common.delete"),
                              )) &&
                              (await call({
                                kind: "deleteRequest",
                                id: item.id,
                              }))
                            )
                              await loadRequests(mine);
                          }}
                        >
                          {t("common.delete")}
                        </UiButton>
                      </>
                    )}
                  </span>
                </li>
              ))}
            </ul>
            {requests?.cursor && (
              <UiButton
                disabled={busy}
                onClick={() => void loadRequests(mine, requests.cursor)}
              >
                {t("examShare.more")}
              </UiButton>
            )}
          </>
        ) : (
          library && (
            <>
              {library.offline ? (
                <p>{t("examShare.libraryUnavailable")}</p>
              ) : (
                <>
                  <h2>{t("examShare.saved")}</h2>
                  {documents(library.saved)}
                </>
              )}
              <h2>{t("examShare.downloaded")}</h2>
              {documents(library.downloaded, "downloaded")}
              {!library.offline && (
                <>
                  <h2>{t("examShare.submitted")}</h2>
                  {documents(library.submitted, "submitted")}
                </>
              )}
            </>
          )
        ))}
    </section>
  );
}

function PlainTextPreview({ path }: { path: string }) {
  const [text, setText] = useState<string>();
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    const controller = new AbortController();
    void fetch(localFileUrl(path), { signal: controller.signal })
      .then((response) => {
        if (!response.ok) throw new Error();
        return response.text();
      })
      .then(setText, () => {
        if (!controller.signal.aborted) setFailed(true);
      });
    return () => controller.abort();
  }, [path]);
  if (failed) return <p role="alert">{t("error.exam-share-response")}</p>;
  return text === undefined ? (
    <p>{t("common.loading")}</p>
  ) : (
    <pre className="examDocumentText">{text}</pre>
  );
}

function RequestForm({
  subject,
  disabled,
  submit,
}: {
  subject: ExamSubjectPage;
  disabled: boolean;
  submit: (action: ExamAction) => Promise<void>;
}) {
  const [classId, setClassId] = useState("");
  const [occasion, setOccasion] = useState("");
  const [role, setRole] = useState("");
  return (
    <form
      className="examShareActions"
      onSubmit={(event) => {
        event.preventDefault();
        const data = new FormData(event.currentTarget);
        void submit({
          kind: "createRequest",
          draft: {
            subjectCode: subject.subject.code,
            classId: Number(classId) || null,
            academicYear: Number(data.get("year")) || null,
            occasion: occasion || null,
            wantedRole: role || null,
            description: String(data.get("description") || ""),
          },
        });
      }}
    >
      <UiSelect
        aria-label={t("examShare.allClasses")}
        value={classId}
        onChange={setClassId}
        disabled={disabled}
        data={[
          { value: "", label: t("examShare.allClasses") },
          ...subject.rows.flatMap((row) =>
            row.classIds.slice(0, 1).map((id) => ({
              value: String(id),
              label:
                [
                  ...row.labels,
                  ...row.teachers.map(({ name }) => name),
                  row.taughtIn,
                ]
                  .filter(Boolean)
                  .join(" · ") || String(id),
            })),
          ),
        ]}
      />
      <UiInput
        name="year"
        type="number"
        min={1950}
        max={3000}
        aria-label={t("examShare.year")}
        placeholder={t("examShare.year")}
      />
      <UiSelect
        aria-label={t("examShare.occasion")}
        value={occasion}
        onChange={setOccasion}
        disabled={disabled}
        data={[
          { value: "", label: t("examShare.allOccasions") },
          ...occasions.map((value) => ({
            value,
            label: t(`examShare.occasion.${value}`),
          })),
        ]}
      />
      <UiSelect
        aria-label={t("examShare.wantedRole")}
        value={role}
        onChange={setRole}
        disabled={disabled}
        data={[
          { value: "", label: t("examShare.wantedRole") },
          { value: "question", label: t("examShare.role.question") },
          { value: "answer", label: t("examShare.role.answer") },
        ]}
      />
      <UiInput
        name="description"
        maxLength={1000}
        placeholder={t("examShare.description")}
      />
      <UiButton type="submit" disabled={disabled}>
        {t("examShare.newRequest")}
      </UiButton>
    </form>
  );
}
