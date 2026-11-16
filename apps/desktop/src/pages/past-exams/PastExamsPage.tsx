import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useEffectEvent,
  useRef,
  useState,
  type FormEvent,
} from "react";
import { Globe, LoaderCircle } from "lucide-react";
import {
  EmptyState,
  Tag,
  UiButton,
  UiInput,
  UiSelect,
} from "../../components/ui";
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
  ExamChoices,
  ExamDocument,
  ExamLibrary,
  ExamLocalDocument,
  ExamOccasion,
  ExamRequest,
  ExamRequestCursor,
  ExamRequestPage,
  ExamReportReason,
  ExamRole,
  ExamSet,
  ExamSubject,
  ExamSubjectPage,
  ExamUploadForm,
  ExamUploadStart,
} from "../../generated/persistence";
import { ExamUploadDialog, ExamUploadProgress } from "./ExamUploadDialog";
import { CurrentCourses } from "./CurrentCourses";
import "./past-exams.css";

const Pdf = lazy(() =>
  import("../../features/lms/PdfDocument").then((m) => ({
    default: m.PdfDocument,
  })),
);
const tabs = ["browse", "requests", "library"] as const;
type Tab = (typeof tabs)[number];
export function PastExams({
  active,
  courseId,
}: {
  active: boolean;
  courseId?: string;
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
  const [tab, setTab] = useState<Tab>("browse");
  const [account, setAccount] = useState<ExamAccount>();
  const [choices, setChoices] = useState<ExamChoices>();
  const [query, setQuery] = useState("");
  const [subjects, setSubjects] = useState<ExamSubject[]>([]);
  const [searched, setSearched] = useState(false);
  const [subject, setSubject] = useState<ExamSubjectPage>();
  const [file, setFile] = useState<ExamLocalDocument>();
  const [library, setLibrary] = useState<ExamLibrary>();
  const [requests, setRequests] = useState<ExamRequestPage>();
  const [mine, setMine] = useState(false);
  /** Views an upload changed while hidden; each reloads when next shown. */
  const [stale, setStale] = useState<ReadonlySet<Tab>>(new Set());
  const [requestDraft, setRequestDraft] = useState(false);
  const [report, setReport] = useState(false);
  const [upload, setUpload] = useState<ExamUploadForm>();
  const [uploadRunning, setUploadRunning] = useState(false);
  const closeUpload = useCallback(() => {
    setUpload(undefined);
    void desktopBridge?.examUploadClearFiles?.().catch(() => undefined);
  }, []);
  const [year, setYear] = useState("");
  const [teacher, setTeacher] = useState("");
  const [occasion, setOccasion] = useState("");
  const [busy, setBusy] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  /** The document being opened; only its button spins. */
  const [opening, setOpening] = useState<string>();
  /** The request shown beside the requests list. */
  const [selectedRequest, setSelectedRequest] = useState<string>();
  const courseOpened = useRef(false);
  /** Every call in flight; a foreground refresh waits for none. */
  const pendingCalls = useRef(0);
  /** Calls that lock the page with `busy`; opening a document does not. */
  const lockingCalls = useRef(0);
  // The last document clicked wins: an earlier one still loading is dropped
  // when it arrives, so a slow file cannot replace the newer one.
  const openSeq = useRef(0);
  const applyAccount = useCallback(
    (next: ExamAccount) => {
      setAccount(next);
      if (next.status === "connected") return;
      courseOpened.current = false;
      openSeq.current += 1;
      setOpening(undefined);
      setSelectedRequest(undefined);
      closeUpload();
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
    async (
      action: ExamAction,
      lock = true,
    ): Promise<ExamAnswer | undefined> => {
      pendingCalls.current += 1;
      if (lock) {
        lockingCalls.current += 1;
        setBusy(true);
        setConnecting(action.kind === "connect");
      }
      setError("");
      // An open keeps the notice of the upload that led to it.
      if (lock) setNotice("");
      try {
        const run = desktopBridge?.examShare;
        if (!run) throw new Error(t("error.unavailable"));
        return await run(action);
      } catch (e) {
        if (errorCode(e) !== "cancelled")
          setError(errorText(e) ?? t("error.unavailable"));
        return undefined;
      } finally {
        pendingCalls.current -= 1;
        if (lock) lockingCalls.current -= 1;
        if (lock && lockingCalls.current === 0) {
          setConnecting(false);
          setBusy(false);
        }
      }
    },
    [],
  );
  const accountAction = useCallback(
    async (kind: "account" | "connect" | "disconnect" | "acceptTerms") => {
      const answer = await call({ kind });
      if (answer?.kind === "account") applyAccount(answer.value);
      return answer?.kind === "account" ? answer.value : undefined;
    },
    [applyAccount, call],
  );
  const connected = account?.status === "connected";

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
  /** Reads again the view on screen: the open subject, the request board or
   * the library. One that was never loaded is left to load when shown. */
  const reloadVisible = async () => {
    if (tab === "browse" && subject) {
      const answer = await call({
        kind: "subject",
        code: subject.subject.code,
        classId: subject.classId,
      });
      if (answer?.kind === "subject") setSubject(answer.value);
    } else if (tab === "requests" && requests) {
      await loadRequests(mine);
    } else if (tab === "library" && library) {
      await loadLibrary();
    }
  };
  // Back in the foreground: read the account, then the view on screen.
  const returnToForeground = useEffectEvent(async () => {
    if (pendingCalls.current > 0 || document.visibilityState === "hidden")
      return;
    if ((await accountAction("account"))?.status === "connected")
      await reloadVisible();
  });
  useEffect(() => {
    if (!active) return;
    const stop = desktopBridge?.onExamAccount?.(applyAccount);
    const refresh = () => void returnToForeground();
    refresh();
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      stop?.();
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [active, applyAccount]);
  useEffect(() => {
    if (!active || !connected || choices) return;
    void call({ kind: "choices" }).then((answer) => {
      if (answer?.kind === "choices") setChoices(answer.value);
    });
  }, [active, connected, choices, call]);
  // A tab's view loads the first time it is shown and is kept afterwards.
  useEffect(() => {
    if (!active || !connected) return;
    if (tab === "requests" && !requests) void loadRequests(mine);
    if (tab === "library" && !library) void loadLibrary();
  }, [
    active,
    connected,
    tab,
    mine,
    requests,
    library,
    loadLibrary,
    loadRequests,
  ]);
  const reloadShown = useEffectEvent(() => reloadVisible());
  useEffect(() => {
    if (!active || !connected || !stale.has(tab)) return;
    setStale((previous) => new Set([...previous].filter((key) => key !== tab)));
    void reloadShown();
  }, [active, connected, tab, stale]);
  const switchTab = (next: Tab) => {
    // A document still opening belonged to the view being left.
    openSeq.current += 1;
    setOpening(undefined);
    setTab(next);
    setFile(undefined);
    setReport(false);
    setRequestDraft(false);
    setError("");
  };
  const open = async (id: string, download = false) => {
    const seq = ++openSeq.current;
    setOpening(id);
    const answer = await call({ kind: "open", id, download }, false);
    if (seq !== openSeq.current) return;
    setOpening(undefined);
    if (answer?.kind === "document") {
      setFile(answer.value);
      // A download joins the library, which answers no view of its own.
      if (download) setLibrary(undefined);
    }
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
    if (!active || !courseId || !connected || courseOpened.current) return;
    courseOpened.current = true;
    void loadSubject({ kind: "courseSubject", courseId });
  }, [active, connected, courseId, loadSubject]);
  const openUpload = async (start: ExamUploadStart) => {
    const answer = await call({ kind: "uploadForm", start });
    if (answer?.kind === "uploadForm") setUpload(answer.value);
  };
  const cancelUpload = () => {
    void desktopBridge
      ?.examShare?.({ kind: "cancelUpload" })
      .catch((failure) =>
        setError(errorText(failure) ?? t("error.unavailable")),
      );
  };
  const submitUpload = async (action: ExamAction) => {
    setUploadRunning(true);
    try {
      const answer = await call(action);
      if (answer?.kind !== "uploaded" && answer?.kind !== "done") return;
      if (answer.kind === "uploaded") closeUpload();
      // The upload changed every view: read the one on screen again; the
      // others reload when next shown.
      setStale(new Set(tabs.filter((key) => key !== tab)));
      await reloadVisible();
      setNotice(
        t(
          answer.kind === "uploaded"
            ? "examShare.upload.uploaded"
            : "examShare.upload.cleaned",
        ),
      );
      // The new document, when Core names it, opens straight away.
      if (answer.kind === "uploaded" && answer.value) void open(answer.value);
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
              aria-busy={opening === entry.id || undefined}
              onClick={() => void open(entry.id)}
            >
              {opening === entry.id && <Spinner />}
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
                  const answer = await call({
                    kind:
                      kind === "submitted"
                        ? "deleteSubmission"
                        : "removeDownload",
                    id: entry.id,
                  });
                  if (answer?.kind === "library") setLibrary(answer.value);
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

  const chosenRequest = requests?.items.find(
    (item) => item.id === selectedRequest,
  );
  const requestFacts = (item: ExamRequest) =>
    [
      item.academicYear,
      item.occasion && lookup(`examShare.occasion.${item.occasion}`),
      item.wantedRole && lookup(`examShare.role.${item.wantedRole}`),
    ]
      .filter(Boolean)
      .join(" · ");
  // Core's address for what is on screen: the open document, else the tab's
  // view once it has loaded.
  const webUrl = !connected
    ? undefined
    : file
      ? file.webUrl
      : tab === "browse"
        ? subject?.webUrl
        : tab === "requests"
          ? requests?.webUrl
          : library?.webUrl;

  return (
    <section className="page" aria-busy={busy}>
      <header className="pageBar">
        <h1>{t("examShare.title")}</h1>
        <div className="pageBarRight">
          {webUrl && (
            <ExternalLink className="examSiteLink" href={webUrl}>
              <Globe size={14} strokeWidth={1.75} aria-hidden />
              {t("examShare.openOnWeb")}
            </ExternalLink>
          )}
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
      {error && !upload && account?.status !== "unsupported" && (
        <p role="alert">{error}</p>
      )}
      {notice && <p role="status">{notice}</p>}
      {busy && !uploadRunning && <p role="status">{t("common.loading")}</p>}
      {!upload && uploadRunning && connected && (
        <ExamUploadProgress status={account.upload} cancel={cancelUpload} />
      )}
      {!upload &&
        !uploadRunning &&
        connected &&
        account.upload.phase === "pending" && (
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
      {upload && choices && connected && (
        <ExamUploadDialog
          form={upload}
          choices={choices}
          subjectLabel={subjectLabel}
          running={uploadRunning}
          status={account.upload}
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
                setError(errorText(failure) ?? t("error.unavailable")),
              );
          }}
        >
          {t("common.cancel")}
        </UiButton>
      )}
      {account?.status === "unsupported" && (
        <p role="alert">{t("error.exam-share-unsupported")}</p>
      )}
      {connected &&
        (file ? (
          <>
            <div className="examShareActions">
              <UiButton
                disabled={busy}
                onClick={() => {
                  setReport(false);
                  setFile(undefined);
                }}
              >
                {t("common.back")}
              </UiButton>
              <UiButton
                disabled={
                  busy ||
                  file.downloaded ||
                  file.offline ||
                  opening === file.document.id
                }
                aria-busy={opening === file.document.id || undefined}
                onClick={() => void open(file.document.id, true)}
              >
                {opening === file.document.id && <Spinner />}
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
                    if (answer?.kind === "saved") {
                      setFile({ ...file, saved: answer.value });
                      // The library lists saved materials; it reads them again.
                      setLibrary(undefined);
                    }
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
            {report && choices && (
              <ReportForm
                choices={choices}
                disabled={busy}
                submit={async (reason, detail) => {
                  if (
                    await call({
                      kind: "report",
                      id: file.document.id,
                      reason,
                      detail,
                    })
                  ) {
                    setReport(false);
                    setNotice(t("examShare.reported"));
                  }
                }}
              />
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
                maxLength={choices?.limits.query}
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
                      onClick={() =>
                        void openUpload({
                          kind: "subject",
                          code: subject.subject.code,
                        })
                      }
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
                {requestDraft && choices && (
                  <RequestForm
                    subject={subject}
                    choices={choices}
                    disabled={busy}
                    submit={async (action) => {
                      if (await call(action)) {
                        setRequestDraft(false);
                        // Both request boards list it; they load fresh when shown.
                        setRequests(undefined);
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
                      ...subject.years.map(String),
                    ]}
                  />
                  <UiSelect
                    aria-label={t("examShare.teacher")}
                    value={teacher}
                    onChange={setTeacher}
                    data={[
                      { value: "", label: t("examShare.allTeachers") },
                      ...subject.teachers.map(({ id, name }) => ({
                        value: id,
                        label: name,
                      })),
                    ]}
                  />
                  <UiSelect
                    aria-label={t("examShare.occasion")}
                    value={occasion}
                    onChange={setOccasion}
                    data={[
                      { value: "", label: t("examShare.allOccasions") },
                      ...subject.occasions.map((value) => ({
                        value,
                        label: lookup(`examShare.occasion.${value}`) ?? value,
                      })),
                    ]}
                  />
                </div>
                <SetList
                  sets={subject.sets.filter(
                    (set) =>
                      (!year || String(set.academicYear) === year) &&
                      (!teacher ||
                        set.teachers.some((x) => x.id === teacher)) &&
                      (!occasion || set.occasion === occasion),
                  )}
                  opening={opening}
                  open={(id) => void open(id)}
                />
              </>
            ) : (
              <>
                <ul className="examImportList">
                  {subjects.map((entry) => (
                    <li key={entry.code}>
                      <UiButton
                        className="textButton"
                        disabled={busy}
                        onClick={() =>
                          void loadSubject({
                            kind: "subject",
                            code: entry.code,
                            classId: null,
                          })
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
                <CurrentCourses
                  active={active}
                  disabled={busy}
                  choose={(id) =>
                    void loadSubject({ kind: "courseSubject", courseId: id })
                  }
                />
              </>
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
            {requests && !requests.items.length ? (
              <EmptyState compact>{t("examShare.noItems")}</EmptyState>
            ) : (
              requests && (
                <div className="examRequests">
                  <div>
                    <ul className="examList">
                      {requests.items.map((item) => (
                        <li
                          key={item.id}
                          className="examRow"
                          aria-current={
                            item.id === selectedRequest || undefined
                          }
                        >
                          <button
                            type="button"
                            className="examRowMain"
                            onClick={() => setSelectedRequest(item.id)}
                          >
                            <strong>
                              {subjectLabel(item.subject)}
                              {item.status !== "open" && (
                                <Tag
                                  tone={
                                    item.status === "fulfilled"
                                      ? "ok"
                                      : "neutral"
                                  }
                                >
                                  {lookup(
                                    `examShare.requestStatus.${item.status}`,
                                  )}
                                </Tag>
                              )}
                            </strong>
                            <small>
                              {[requestFacts(item), item.description]
                                .filter(Boolean)
                                .join(" · ") || item.subject.code}
                            </small>
                          </button>
                        </li>
                      ))}
                    </ul>
                    {requests.cursor && (
                      <UiButton
                        className="examMore"
                        disabled={busy}
                        onClick={() => void loadRequests(mine, requests.cursor)}
                      >
                        {t("examShare.more")}
                      </UiButton>
                    )}
                  </div>
                  {chosenRequest ? (
                    <RequestDetail
                      item={chosenRequest}
                      subjectLabel={subjectLabel}
                      facts={requestFacts(chosenRequest)}
                      disabled={busy}
                      opening={
                        !!chosenRequest.answerExamId &&
                        opening === chosenRequest.answerExamId
                      }
                      view={() =>
                        chosenRequest.answerExamId &&
                        void open(chosenRequest.answerExamId)
                      }
                      answer={() =>
                        void openUpload({
                          kind: "request",
                          request: chosenRequest,
                        })
                      }
                      setOpen={async (next) => {
                        const answer = await call({
                          kind: "setRequestOpen",
                          id: chosenRequest.id,
                          open: next,
                          mine,
                        });
                        if (answer?.kind === "requests")
                          setRequests(answer.value);
                      }}
                      remove={async () => {
                        if (
                          !(await confirmAction(
                            t("examShare.deleteRequest"),
                            t("common.delete"),
                          ))
                        )
                          return;
                        const answer = await call({
                          kind: "deleteRequest",
                          id: chosenRequest.id,
                          mine,
                        });
                        if (answer?.kind === "requests") {
                          setSelectedRequest(undefined);
                          setRequests(answer.value);
                        }
                      }}
                    />
                  ) : (
                    <EmptyState>{t("examShare.pickRequest")}</EmptyState>
                  )}
                </div>
              )
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
      {/* English only by design: it names the service and its operator. */}
      <div
        className="examShareCredit"
        role="img"
        aria-label="Supported by ScienceTokyoExam · teduryu"
      />
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
  if (failed) return <p role="alert">{t("error.storage")}</p>;
  return text === undefined ? (
    <p>{t("common.loading")}</p>
  ) : (
    <pre className="examDocumentText">{text}</pre>
  );
}

function ReportForm({
  choices,
  disabled,
  submit,
}: {
  choices: ExamChoices;
  disabled: boolean;
  submit: (reason: ExamReportReason, detail: string) => Promise<void>;
}) {
  const [reason, setReason] = useState(choices.reportReasons[0]);
  return (
    <form
      className="examShareActions"
      onSubmit={(event) => {
        event.preventDefault();
        const data = new FormData(event.currentTarget);
        void submit(reason, String(data.get("detail") ?? ""));
      }}
    >
      <UiSelect
        aria-label={t("examShare.report")}
        value={reason}
        onChange={(value) => setReason(value as ExamReportReason)}
        data={choices.reportReasons.map((value) => ({
          value,
          label: t(`examShare.reason.${value}`),
        }))}
      />
      <UiInput
        name="detail"
        maxLength={choices.limits.reportDetail}
        placeholder={t("examShare.description")}
      />
      <UiButton type="submit" disabled={disabled}>
        {t("examShare.report")}
      </UiButton>
    </form>
  );
}

function RequestForm({
  subject,
  choices,
  disabled,
  submit,
}: {
  subject: ExamSubjectPage;
  choices: ExamChoices;
  disabled: boolean;
  submit: (action: ExamAction) => Promise<void>;
}) {
  const [classId, setClassId] = useState("");
  const [occasion, setOccasion] = useState<ExamOccasion | "">("");
  const [role, setRole] = useState<ExamRole | "">("");
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
        min={choices.limits.earliestYear}
        max={choices.limits.latestYear}
        aria-label={t("examShare.year")}
        placeholder={t("examShare.year")}
      />
      <UiSelect
        aria-label={t("examShare.occasion")}
        value={occasion}
        onChange={(value) => setOccasion(value as ExamOccasion | "")}
        disabled={disabled}
        data={[
          { value: "", label: t("examShare.allOccasions") },
          ...choices.requestOccasions.map((value) => ({
            value,
            label: t(`examShare.occasion.${value}`),
          })),
        ]}
      />
      <UiSelect
        aria-label={t("examShare.wantedRole")}
        value={role}
        onChange={(value) => setRole(value as ExamRole | "")}
        disabled={disabled}
        data={[
          { value: "", label: t("examShare.wantedRole") },
          ...choices.requestRoles.map((value) => ({
            value,
            label: t(`examShare.role.${value}`),
          })),
        ]}
      />
      <UiInput
        name="description"
        maxLength={choices.limits.requestDescription}
        placeholder={t("examShare.description")}
      />
      <UiButton type="submit" disabled={disabled}>
        {t("examShare.newRequest")}
      </UiButton>
    </form>
  );
}

/** The spinner a document button shows while its file loads. */
function Spinner() {
  return (
    <LoaderCircle size={12} strokeWidth={2} className="examSpin" aria-hidden />
  );
}

/** One row per sitting. The row is a label; each of its documents (question,
 * answer, …) is its own pill button, and only the one being opened spins. */
function SetList({
  sets,
  opening,
  open,
}: {
  sets: ExamSet[];
  opening?: string;
  open: (id: string) => void;
}) {
  if (!sets.length) return <p>{t("examShare.noItems")}</p>;
  return (
    <ul className="examSets">
      {sets.map((set) => (
        <li key={set.documents[0]?.id} className="examSet">
          <span className="examSetYear">{set.academicYear}</span>
          <span className="examSetText">
            <strong>
              {[
                set.occasion && lookup(`examShare.occasion.${set.occasion}`),
                set.occasionNumber > 1 ? set.occasionNumber : null,
                set.variant,
              ]
                .filter(Boolean)
                .join(" ")}
            </strong>
            {set.teachers.length > 0 && (
              <small>{set.teachers.map(({ name }) => name).join(" / ")}</small>
            )}
          </span>
          <span className="examSetRoles">
            {set.documents.map((document) => (
              <button
                key={document.id}
                type="button"
                className="examRole"
                data-role={document.role ?? undefined}
                aria-busy={document.id === opening || undefined}
                title={document.note ?? document.fileName}
                onClick={() => open(document.id)}
              >
                {document.id === opening && <Spinner />}
                {document.role
                  ? (lookup(`examShare.role.${document.role}`) ??
                    document.fileName)
                  : t("examShare.role.other")}
              </button>
            ))}
          </span>
        </li>
      ))}
    </ul>
  );
}

/** One request in full: what is wanted and from whom, what it is for, and
 * what to do next: share a file, see the shared one, or manage one's own. */
function RequestDetail({
  item,
  subjectLabel,
  facts,
  disabled,
  opening,
  view,
  answer,
  setOpen,
  remove,
}: {
  item: ExamRequest;
  subjectLabel: (subject: ExamSubject) => string;
  facts: string;
  disabled: boolean;
  opening: boolean;
  view: () => void;
  answer: () => void;
  setOpen: (open: boolean) => Promise<void>;
  remove: () => Promise<void>;
}) {
  return (
    <article className="examRequest">
      <header>
        <small>{item.subject.code}</small>
        <h2>{subjectLabel(item.subject)}</h2>
        <Tag
          tone={
            item.status === "fulfilled"
              ? "ok"
              : item.status === "open"
                ? "accent"
                : "neutral"
          }
        >
          {lookup(`examShare.requestStatus.${item.status}`) ?? item.status}
        </Tag>
      </header>
      {facts && <p className="examRequestFacts">{facts}</p>}
      {item.teachers.length > 0 && (
        <p className="examRequestFacts">
          {item.teachers.map(({ name }) => name).join(" / ")}
        </p>
      )}
      {item.description && (
        <p className="examRequestText">{item.description}</p>
      )}
      <div className="examShareActions">
        {item.answerExamId && (
          <UiButton
            aria-busy={opening || undefined}
            disabled={opening}
            onClick={view}
          >
            {opening && <Spinner />}
            {t("examShare.viewAnswer")}
          </UiButton>
        )}
        {item.status === "open" && (
          <UiButton disabled={disabled} onClick={answer}>
            {t("examShare.upload.answerRequest")}
          </UiButton>
        )}
        {item.mine && (
          <>
            <UiButton
              disabled={disabled}
              onClick={() => void setOpen(item.status !== "open")}
            >
              {t(
                item.status === "open"
                  ? "examShare.closeRequest"
                  : "examShare.reopenRequest",
              )}
            </UiButton>
            <UiButton disabled={disabled} onClick={() => void remove()}>
              {t("common.delete")}
            </UiButton>
          </>
        )}
      </div>
    </article>
  );
}
