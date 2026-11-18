import {
  useCallback,
  useEffect,
  useEffectEvent,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  BookOpen,
  ChevronDown,
  ChevronRight,
  FileText,
  Globe,
  Inbox,
  Library,
  LoaderCircle,
  Search,
  Trash2,
  Upload,
} from "lucide-react";
import { Accordion } from "radix-ui";
import { useDebounceValue } from "usehooks-ts";
import {
  Button,
  EmptyState,
  ErrorBanner,
  Input,
  Menu,
  SegmentedControl,
  Skeleton,
  Tag,
} from "../../components/ui";
import { confirmAction } from "../../components/confirm";
import { ExternalLink } from "../../components/ExternalLink";
import { ResizableWorkspace } from "../../components/ResizableWorkspace";
import { desktopBridge } from "../../app/native";
import { notify } from "../../app/notify";
import { errorCode } from "../../platform/native-error";
import { useScheduleQuery } from "../../platform/core-query";
import { useRecords } from "../../platform/records";
import { lookup, t } from "../../i18n";
import type {
  ExamAccount,
  ExamAction,
  ExamAnswer,
  ExamChoices,
  ExamCourseList,
  ExamDocument,
  ExamLibrary,
  ExamLocalDocument,
  ExamRequest,
  ExamRequestCursor,
  ExamRequestPage,
  ExamSet,
  ExamSubject,
  ExamSubjectPage,
  ExamUploadForm,
  ExamUploadStart,
} from "../../generated/persistence";
import {
  RequestDialog,
  UploadDialog,
  UploadProgress,
  UploadRecovery,
} from "./ExamDialogs";
import { ExamDocumentPanel, openExamFile } from "./ExamDocumentPanel";
import { useExamLibrary } from "./useExamLibrary";
import "./past-exams.css";

const tabs = ["browse", "requests", "library"] as const;
type Tab = (typeof tabs)[number];
type Call = (action: ExamAction) => Promise<ExamAnswer | undefined>;
type OpenUpload = (start: ExamUploadStart) => void;

/** Runs one Core action. Failures go to the toast; a lost session reaches
 * the page as an `exam-account` event. Cancelled sign-ins are silent. */
function useExamShare() {
  /** Every call in flight; a foreground refresh waits for none. */
  const inFlight = useRef(0);
  const call = useCallback<Call>(async (action) => {
    const run = desktopBridge?.examShare;
    inFlight.current += 1;
    try {
      if (!run) throw new Error(t("error.unavailable"));
      return await run(action);
    } catch (e) {
      if (errorCode(e) !== "cancelled") notify.error(e);
      return undefined;
    } finally {
      inFlight.current -= 1;
    }
  }, []);
  return { call, inFlight };
}

/** Core names subjects in the person's language. */
const subjectLabel = (subject: ExamSubject) => subject.name;
/** A sitting's or document's occasion: "Quiz 3 B", else the file name. */
const occasionLabel = (
  item: Pick<ExamSet, "occasion" | "occasionNumber" | "variant">,
  fallback: string,
) =>
  [
    item.occasion && lookup(`examShare.occasion.${item.occasion}`),
    item.occasionNumber > 1 ? item.occasionNumber : null,
    item.variant,
  ]
    .filter(Boolean)
    .join(" ") || fallback;
const documentOccasion = (document: ExamDocument) =>
  occasionLabel(document, document.fileName);
const documentTitle = (document: ExamDocument) =>
  [
    document.subjects.map(subjectLabel).join(" / "),
    document.academicYear,
    documentOccasion(document),
    document.role && lookup(`examShare.role.${document.role}`),
  ]
    .filter(Boolean)
    .join(" · ");
/** A document's role on its pill, with the number Core gives it when its
 * sitting holds several of that role. */
const roleLabel = (document: ExamDocument) =>
  [
    (document.role && lookup(`examShare.role.${document.role}`)) ||
      t("examShare.file"),
    document.number,
  ]
    .filter(Boolean)
    .join(" ");
const teacherNames = (teachers: { name: string }[]) =>
  teachers.map(({ name }) => name).join(" / ") || t("examShare.unknownTeacher");

export function PastExams({
  active,
  courseId,
}: {
  active: boolean;
  /** A course to open its subject for, once, when the page is shown. */
  courseId?: string;
}) {
  const { call, inFlight } = useExamShare();
  const [account, setAccount] = useState<ExamAccount>();
  const [choices, setChoices] = useState<ExamChoices>();
  const [tab, setTab] = useState<Tab>("browse");
  const [pending, setPending] = useState(false);
  const [reader, setReader] = useState<ExamLocalDocument>();
  /** The document being opened; only its button spins. */
  const [opening, setOpening] = useState<string>();
  /** Bumped when every view should read Core again: back in the
   * foreground, or after an upload changed what the site lists. */
  const [generation, setGeneration] = useState(0);
  /** Core's address for the view on screen, once it has loaded. */
  const [viewUrl, setViewUrl] = useState<string>();
  const [upload, setUpload] = useState<ExamUploadForm>();
  const [uploadRunning, setUploadRunning] = useState(false);
  const connected = account?.status === "connected";
  const {
    library,
    failed: libraryFailed,
    refresh: refreshLibrary,
    replace: replaceLibrary,
  } = useExamLibrary(call, active && connected && tab === "library");
  const courseOpened = useRef(false);
  // The last document clicked wins: an earlier one still loading is
  // dropped when it arrives, so a slow file cannot replace the new one.
  const openSeq = useRef(0);

  const closeUpload = useCallback(() => {
    setUpload(undefined);
    void desktopBridge?.examUploadClearFiles?.().catch(() => undefined);
  }, []);
  const applyAccount = useCallback(
    (next: ExamAccount) => {
      setAccount(next);
      if (next.status === "connected") return;
      courseOpened.current = false;
      openSeq.current += 1;
      setOpening(undefined);
      setReader(undefined);
      replaceLibrary(undefined);
      closeUpload();
    },
    [closeUpload, replaceLibrary],
  );
  const accountAction = useCallback(
    async (kind: "account" | "connect" | "disconnect" | "acceptTerms") => {
      setPending(kind !== "account");
      const answer = await call({ kind });
      setPending(false);
      if (answer?.kind === "account") applyAccount(answer.value);
      return answer?.kind === "account" ? answer.value : undefined;
    },
    [applyAccount, call],
  );
  // Back in the foreground: read the account, then the views again.
  const returnToForeground = useEffectEvent(async (foreground: boolean) => {
    if (inFlight.current > 0 || document.visibilityState === "hidden") return;
    const was = account?.status;
    if ((await accountAction("account"))?.status === "connected" && was) {
      setGeneration((value) => value + 1);
      if (foreground) refreshLibrary();
    }
  });
  useEffect(() => {
    if (!active) return;
    const stop = desktopBridge?.onExamAccount?.(applyAccount);
    const refresh = () => void returnToForeground(true);
    void returnToForeground(false);
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

  const open = useCallback(
    async (id: string) => {
      const seq = ++openSeq.current;
      setOpening(id);
      const answer = await call({ kind: "open", id, download: false });
      if (seq !== openSeq.current) return;
      setOpening(undefined);
      if (answer?.kind === "document") {
        setReader(answer.value);
        await openExamFile(answer.value).catch(notify.error);
      }
    },
    [call],
  );
  // Core names subjects in the content language, which these two settings
  // decide (`approach/content-language-in-core`). When either changes, every
  // browse/request view and the open document are read again.
  const settings = useRecords([])?.settings;
  const language =
    settings && `${settings.contentLanguage ?? "auto"}:${settings.locale}`;
  const shownLanguage = useRef(language);
  const rereadDocument = useEffectEvent(async () => {
    if (!reader) return;
    const seq = ++openSeq.current;
    const answer = await call({
      kind: "open",
      id: reader.document.id,
      download: false,
    });
    if (seq === openSeq.current && answer?.kind === "document")
      setReader(answer.value);
  });
  useEffect(() => {
    if (!language || shownLanguage.current === language) return;
    const first = shownLanguage.current === undefined;
    shownLanguage.current = language;
    if (first || !connected) return;
    setGeneration((value) => value + 1);
    void rereadDocument();
  }, [language, connected]);
  const closeReader = useCallback(() => {
    openSeq.current += 1;
    setOpening(undefined);
    setReader(undefined);
  }, []);
  const switchTab = (next: string) => {
    closeReader();
    setViewUrl(undefined);
    setTab(next as Tab);
  };

  const openUpload = useCallback(
    async (start: ExamUploadStart) => {
      const answer = await call({ kind: "uploadForm", start });
      if (answer?.kind === "uploadForm") setUpload(answer.value);
    },
    [call],
  );
  const cancelUpload = () =>
    void desktopBridge
      ?.examShare?.({ kind: "cancelUpload" })
      .catch((failure) => notify.error(failure));
  const submitUpload = async (action: ExamAction) => {
    setUploadRunning(true);
    try {
      const answer = await call(action);
      if (answer?.kind !== "uploaded" && answer?.kind !== "done") return;
      if (answer.kind === "uploaded") {
        closeUpload();
        refreshLibrary();
      }
      // Refresh the browse and request views after upload recovery or publication.
      setGeneration((value) => value + 1);
      notify.info(
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

  // Core loads the document actions; the system application shows its contents.
  const readerPane =
    opening && opening !== reader?.document.id ? (
      <div className="examOpening" role="status">
        <LoaderCircle
          size={20}
          strokeWidth={1.75}
          className="examSpin"
          aria-hidden
        />
        {t("examShare.opening")}
      </div>
    ) : (
      reader && (
        <ExamDocumentPanel
          key={reader.document.id}
          file={reader}
          title={documentTitle(reader.document)}
          choices={choices}
          call={call}
          setFile={(update) =>
            setReader((current) =>
              current?.document.id === reader.document.id
                ? update(current)
                : current,
            )
          }
          onLibraryChanged={refreshLibrary}
          close={closeReader}
        />
      )
    );
  const webUrl = connected ? (reader?.webUrl ?? viewUrl) : undefined;
  const takeCourse = useCallback(() => {
    if (!courseId || courseOpened.current) return undefined;
    courseOpened.current = true;
    return courseId;
  }, [courseId]);
  const shared = {
    call,
    open: (id: string) => void open(id),
    opening,
    reading: reader?.document.id,
    pane: readerPane,
    generation,
    setViewUrl,
  };

  return (
    <section className="page pastExams" data-active={active || undefined}>
      <header className="pageBar">
        <h1>{t("examShare.title")}</h1>
        {connected && (
          <SegmentedControl
            value={tab}
            onChange={switchTab}
            data={tabs.map((value) => ({
              value,
              label: t(`examShare.${value}`),
            }))}
          />
        )}
        <div className="pageBarRight">
          <span className="examPartner">{t("examShare.partner")}</span>
          {webUrl && (
            <ExternalLink className="examSiteLink" href={webUrl}>
              <Globe size={14} strokeWidth={1.75} aria-hidden />
              {t("examShare.openOnWeb")}
            </ExternalLink>
          )}
          {connected && (
            <Menu position="bottom-end">
              <Menu.Target>
                <Button variant="subtle" color="gray" className="examAccount">
                  {account.email ?? t("examShare.account")}
                </Button>
              </Menu.Target>
              <Menu.Dropdown>
                <Menu.Item
                  className="examDanger"
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
                </Menu.Item>
              </Menu.Dropdown>
            </Menu>
          )}
        </div>
      </header>
      {connected &&
        !upload &&
        (uploadRunning ? (
          <UploadProgress status={account.upload} cancel={cancelUpload} />
        ) : (
          account.upload.phase === "pending" && (
            <UploadRecovery
              recover={() => void submitUpload({ kind: "recoverUpload" })}
            />
          )
        ))}
      {!connected ? (
        <AccountGate
          account={account}
          pending={pending}
          connect={() => void accountAction("connect")}
          accept={() => void accountAction("acceptTerms")}
        />
      ) : tab === "browse" ? (
        <BrowseList
          {...shared}
          active={active}
          choices={choices}
          takeCourse={takeCourse}
          openUpload={openUpload}
        />
      ) : tab === "requests" ? (
        <RequestList
          {...shared}
          closeReader={closeReader}
          openUpload={openUpload}
        />
      ) : (
        <LibraryList
          {...shared}
          library={library}
          failed={libraryFailed}
          refresh={refreshLibrary}
          replace={replaceLibrary}
          closeReader={(id) => {
            if (reader?.document.id === id) closeReader();
          }}
        />
      )}
      {upload && choices && connected && (
        <UploadDialog
          form={upload}
          choices={choices}
          running={uploadRunning}
          status={account.upload}
          run={call}
          submit={submitUpload}
          cancel={cancelUpload}
          close={closeUpload}
        />
      )}
    </section>
  );
}

/** What every tab's view takes from the page. */
type ViewProps = {
  call: Call;
  open: (id: string) => void;
  opening?: string;
  reading?: string;
  pane: ReactNode;
  generation: number;
  setViewUrl: (url: string | undefined) => void;
};

/** Runs `reload` each time the page asks every view to read Core again,
 * but not for the generation the view mounted at. */
function useReload(generation: number, reload: () => void) {
  const seen = useRef(generation);
  const run = useEffectEvent(reload);
  useEffect(() => {
    if (seen.current === generation) return;
    seen.current = generation;
    run();
  }, [generation]);
}

/** The list on the left and what it opens on the right; one layout for all three tabs. */
function Workspace({ children }: { children: [ReactNode, ReactNode] }) {
  return (
    <ResizableWorkspace
      id="past-exams"
      enabled
      firstSize={320}
      firstMin={260}
      firstMax="45%"
      secondMin="40%"
    >
      {children}
    </ResizableWorkspace>
  );
}

/** Everything before a usable session: sign in, the terms, or why not. */
function AccountGate({
  account,
  pending,
  connect,
  accept,
}: {
  account?: ExamAccount;
  pending: boolean;
  connect: () => void;
  accept: () => void;
}) {
  if (!account) return <Skeleton count={4} />;
  if (account.status === "unsupported")
    return (
      <EmptyState role="alert">{t("error.exam-share-unsupported")}</EmptyState>
    );
  if (account.status === "needsConsent")
    return (
      <EmptyState
        icon={FileText}
        action={
          <div className="examGateActions">
            <span>
              <ExternalLink href="https://sciencetokyoexam.com/terms">
                {t("examShare.terms")}
              </ExternalLink>
              {" · "}
              <ExternalLink href="https://sciencetokyoexam.com/privacy">
                {t("examShare.privacy")}
              </ExternalLink>
            </span>
            <Button loading={pending} onClick={accept}>
              {t("examShare.accept")}
            </Button>
          </div>
        }
      >
        {t("examShare.consentHint")}
      </EmptyState>
    );
  return (
    <EmptyState
      icon={BookOpen}
      action={
        <div className="examGateActions">
          <Button loading={pending} onClick={connect}>
            {t("examShare.connect")}
          </Button>
          {pending && (
            <Button
              variant="subtle"
              onClick={() =>
                void desktopBridge
                  ?.examShare?.({ kind: "cancelConnect" })
                  .catch((failure) => notify.error(failure))
              }
            >
              {t("common.cancel")}
            </Button>
          )}
        </div>
      }
    >
      {t(pending ? "examShare.connectingHint" : "examShare.connectHint")}
    </EmptyState>
  );
}

/** One list row: a title, a muted line under it, and optional trailing UI. */
function Row({
  title,
  meta,
  current,
  pending,
  disabled,
  onClick,
  trailing,
}: {
  title: ReactNode;
  meta?: ReactNode;
  current?: boolean;
  pending?: boolean;
  disabled?: boolean;
  onClick?: () => void;
  trailing?: ReactNode;
}) {
  return (
    <li className="examRow" aria-current={current || undefined}>
      <button
        type="button"
        className="examRowMain"
        disabled={disabled}
        aria-busy={pending || undefined}
        onClick={onClick}
      >
        <strong>{title}</strong>
        {meta && <small>{meta}</small>}
      </button>
      {trailing}
    </li>
  );
}

/** One course or search hit on the left, and how Core opens its subject:
 * a course by its id, so Core can put its own class's teachers first. */
type Entry = {
  key: string;
  title: string;
  meta: string;
  action?: Extract<ExamAction, { kind: "subject" | "courseSubject" }>;
};
/** A panel's content: the subject, the subjects a course maps to, or a failure. */
type PanelState = {
  page?: ExamSubjectPage;
  subjects?: ExamSubject[];
  failed?: boolean;
};

/** Left: this quarter's courses or a search, each subject an expansion
 * panel of its sittings. Right: always the reader. */
function BrowseList({
  active,
  choices,
  takeCourse,
  openUpload,
  call,
  open,
  opening,
  reading,
  pane,
  generation,
  setViewUrl,
}: ViewProps & {
  active: boolean;
  choices?: ExamChoices;
  takeCourse: () => string | undefined;
  openUpload: OpenUpload;
}) {
  const [query, setQuery] = useState("");
  const [debounced] = useDebounceValue(query.trim(), 300);
  const [hits, setHits] = useState<{ query: string; items: ExamSubject[] }>();
  const [expanded, setExpanded] = useState("");
  const [pages, setPages] = useState<Record<string, PanelState>>({});
  /** A course opened from elsewhere that this quarter's list lacks. */
  const [extra, setExtra] = useState<string>();
  const searchSeq = useRef(0);

  useEffect(() => {
    if (!debounced) return setHits(undefined);
    const seq = ++searchSeq.current;
    void call({ kind: "search", query: debounced }).then((answer) => {
      if (seq !== searchSeq.current) return;
      setHits({
        query: debounced,
        items: answer?.kind === "subjects" ? answer.value : [],
      });
    });
  }, [debounced, call]);

  const searching = !!query.trim();
  const timetable = useScheduleQuery<ExamCourseList>(
    { kind: "examCourses" },
    active && !searching,
  );

  const load = useCallback(
    async (key: string, action: Entry["action"]) => {
      if (!action) return;
      // A refresh keeps what is shown until the new answer arrives.
      setPages((all) => ({ ...all, [key]: all[key]?.page ? all[key] : {} }));
      const answer = await call(action);
      setPages((all) => ({
        ...all,
        [key]:
          answer?.kind === "subject"
            ? { page: answer.value }
            : answer?.kind === "subjects"
              ? { subjects: answer.value }
              : all[key]?.page
                ? all[key]
                : { failed: true },
      }));
    },
    [call],
  );
  /** Reads an open subject again in the order it was opened with. */
  const reloadAction = (key: string, entry?: Entry): Entry["action"] => {
    const page = pages[key]?.page;
    return page
      ? { kind: "subject", code: page.subject.code, classId: page.classId }
      : entry?.action;
  };

  // A course opened from its course page expands at once.
  useEffect(() => {
    const id = takeCourse();
    if (!id) return;
    const key = `course:${id}`;
    setQuery("");
    setExtra(id);
    setExpanded(key);
    void load(key, { kind: "courseSubject", courseId: id });
  }, [takeCourse, load]);

  const courses = timetable.value?.courses;
  const courseEntries: Entry[] | undefined = courses?.map((course) => ({
    key: `course:${course.courseId}`,
    title: course.title,
    meta:
      [course.subjectCodes.join(" / "), course.instructor]
        .filter(Boolean)
        .join(" · ") || t("examShare.noSubjectCode"),
    action: course.subjectCodes.length
      ? { kind: "courseSubject", courseId: course.courseId }
      : undefined,
  }));
  const extraPage = extra ? pages[`course:${extra}`]?.page : undefined;
  if (
    courseEntries &&
    extra &&
    !courseEntries.some((entry) => entry.key === `course:${extra}`)
  )
    courseEntries.unshift({
      key: `course:${extra}`,
      title: extraPage ? subjectLabel(extraPage.subject) : t("common.loading"),
      meta: extraPage?.subject.code ?? "",
      action: { kind: "courseSubject", courseId: extra },
    });
  const entries: Entry[] | undefined = searching
    ? hits?.query === debounced && debounced === query.trim()
      ? hits.items.map((subject) => ({
          key: `search:${subject.code}`,
          title: subjectLabel(subject),
          meta: subject.code,
          action: { kind: "subject", code: subject.code, classId: null },
        }))
      : undefined
    : courseEntries;

  const shownUrl = pages[expanded]?.page?.webUrl;
  useEffect(() => setViewUrl(shownUrl), [shownUrl, setViewUrl]);
  // Only the open panel reads again; the others load when next opened.
  useReload(generation, () => {
    const entry = entries?.find((e) => e.key === expanded);
    setPages((all) => (all[expanded] ? { [expanded]: all[expanded] } : {}));
    if (expanded) void load(expanded, reloadAction(expanded, entry));
  });

  return (
    <Workspace>
      <div className="examPane">
        <Input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.currentTarget.value)}
          maxLength={choices?.limits.query ?? 100}
          placeholder={t("examShare.search")}
          aria-label={t("examShare.search")}
          rightSection={<Search size={14} strokeWidth={1.75} aria-hidden />}
        />
        <div className="scroll">
          {!searching && timetable.error ? (
            <ErrorBanner onRetry={timetable.retry}>
              {timetable.error}
            </ErrorBanner>
          ) : !entries ? (
            <Skeleton count={5} />
          ) : entries.length ? (
            <>
              {!searching && (
                <h2 className="examListHeading">
                  {t("examShare.currentCourses")}
                </h2>
              )}
              <Accordion.Root
                type="single"
                collapsible
                value={expanded}
                onValueChange={(key) => {
                  setExpanded(key);
                  const entry = entries.find((e) => e.key === key);
                  if (entry && !pages[key]?.page) void load(key, entry.action);
                }}
                className="examPanels"
              >
                {entries.map((entry) => (
                  <Accordion.Item
                    key={entry.key}
                    value={entry.key}
                    disabled={!entry.action}
                    className="examPanel"
                  >
                    <Accordion.Header asChild>
                      <h3>
                        <Accordion.Trigger className="examPanelTrigger">
                          <span>
                            <strong>{entry.title}</strong>
                            <small>{entry.meta}</small>
                          </span>
                          <ChevronDown
                            size={16}
                            strokeWidth={1.75}
                            aria-hidden
                          />
                        </Accordion.Trigger>
                      </h3>
                    </Accordion.Header>
                    <Accordion.Content className="examPanelContent">
                      <SubjectSets
                        state={pages[entry.key]}
                        choices={choices}
                        call={call}
                        open={open}
                        opening={opening}
                        reading={reading}
                        retry={() =>
                          void load(entry.key, reloadAction(entry.key, entry))
                        }
                        choose={(code) =>
                          void load(entry.key, {
                            kind: "subject",
                            code,
                            classId: null,
                          })
                        }
                        openUpload={openUpload}
                      />
                    </Accordion.Content>
                  </Accordion.Item>
                ))}
              </Accordion.Root>
            </>
          ) : (
            <EmptyState compact icon={Search}>
              {searching
                ? t("examShare.noSubjects")
                : t("examShare.noCurrentCourses")}
            </EmptyState>
          )}
        </div>
      </div>
      <div className="examPane examDetail">
        {pane || (
          <EmptyState icon={expanded ? FileText : BookOpen}>
            {t(expanded ? "examShare.pickDocument" : "examShare.pickSubject")}
          </EmptyState>
        )}
      </div>
    </Workspace>
  );
}

/** A subject's sittings inside its panel: the person's own class's
 * teachers first, the rest behind one toggle, then a way to share or ask for
 * what is missing. */
function SubjectSets({
  state,
  choices,
  call,
  open,
  opening,
  reading,
  retry,
  choose,
  openUpload,
}: {
  state?: PanelState;
  choices?: ExamChoices;
  call: Call;
  open: (id: string) => void;
  opening?: string;
  reading?: string;
  retry: () => void;
  choose: (code: string) => void;
  openUpload: OpenUpload;
}) {
  const [requesting, setRequesting] = useState(false);
  const [othersOpen, setOthersOpen] = useState(false);
  if (state?.failed)
    return (
      <ErrorBanner onRetry={retry}>{t("examShare.loadFailed")}</ErrorBanner>
    );
  if (state?.subjects)
    return state.subjects.length ? (
      <ul className="examList">
        {state.subjects.map((subject) => (
          <Row
            key={subject.code}
            title={subjectLabel(subject)}
            meta={subject.code}
            onClick={() => choose(subject.code)}
          />
        ))}
      </ul>
    ) : (
      <p className="examQuiet">{t("examShare.noSubjects")}</p>
    );
  const page = state?.page;
  if (!page) return <Skeleton count={3} />;
  const sets = page.sets;
  // Core knows the person's class only when the subject came from a course.
  const grouped = page.classId !== null;
  const mine = sets.filter((set) => !grouped || set.mine);
  const others = grouped ? sets.filter((set) => !set.mine) : [];
  const rows = (list: ExamSet[]) => (
    <ul className="examSets">
      {list.map((set) => (
        <SetRow
          key={set.documents[0]?.id}
          set={set}
          open={open}
          opening={opening}
          reading={reading}
          withTeachers={!set.mine}
        />
      ))}
    </ul>
  );
  return (
    <>
      {!sets.length ? (
        <p className="examQuiet">{t("examShare.noDocuments")}</p>
      ) : (
        <>
          {mine.length > 0
            ? rows(mine)
            : grouped && (
                <p className="examQuiet">{t("examShare.noOwnTeacher")}</p>
              )}
          {others.length > 0 && (
            <>
              <button
                type="button"
                className="examOthers"
                aria-expanded={othersOpen}
                onClick={() => setOthersOpen(!othersOpen)}
              >
                <ChevronRight size={14} strokeWidth={1.75} aria-hidden />
                {t("examShare.otherTeachers", { count: others.length })}
              </button>
              {othersOpen && rows(others)}
            </>
          )}
        </>
      )}
      <div className="examPanelActions">
        <Button
          variant="subtle"
          size="compact-sm"
          disabled={!choices}
          onClick={() =>
            openUpload({ kind: "subject", code: page.subject.code })
          }
        >
          {t("examShare.uploadHere")}
        </Button>
        <Button
          variant="subtle"
          size="compact-sm"
          disabled={!choices}
          onClick={() => setRequesting(true)}
        >
          {t("examShare.newRequest")}
        </Button>
      </div>
      {choices && (
        <RequestDialog
          opened={requesting}
          subject={page}
          choices={choices}
          close={() => setRequesting(false)}
          submit={async (draft) => {
            if (await call({ kind: "createRequest", draft })) {
              setRequesting(false);
              notify.info(t("examShare.requestCreated"));
            }
          }}
        />
      )}
    </>
  );
}

/** One sitting. The sitting is a label; each of its documents (question,
 * answer, …) is its own pill button, and only the one being opened spins. */
function SetRow({
  set,
  open,
  opening,
  reading,
  withTeachers,
}: {
  set: ExamSet;
  open: (id: string) => void;
  opening?: string;
  reading?: string;
  withTeachers: boolean;
}) {
  const current = set.documents.some((d) => d.id === reading);
  return (
    <li className="examSet" aria-current={current || undefined}>
      <div className="examSetMain">
        <FileText size={15} strokeWidth={1.75} aria-hidden />
        <span className="examSetYear">{set.academicYear ?? "—"}</span>
        <span className="examSetText">
          <strong>
            {occasionLabel(set, set.documents[0]?.fileName ?? "")}
          </strong>
          {withTeachers && <small>{teacherNames(set.teachers)}</small>}
        </span>
      </div>
      <span className="examSetRoles">
        {set.documents.map((document) => (
          <button
            key={document.id}
            type="button"
            className="examRole"
            data-role={document.role ?? undefined}
            aria-pressed={document.id === reading}
            aria-busy={document.id === opening || undefined}
            title={document.note ?? document.fileName}
            onClick={() => open(document.id)}
          >
            {document.id === opening && (
              <LoaderCircle
                size={12}
                strokeWidth={2}
                className="examSpin"
                aria-hidden
              />
            )}
            {roleLabel(document)}
          </button>
        ))}
      </span>
    </li>
  );
}

function DocumentRow({
  document,
  open,
  opening,
  current,
  trailing,
}: {
  document: ExamDocument;
  open: (id: string) => void;
  opening?: string;
  current?: boolean;
  trailing?: ReactNode;
}) {
  return (
    <Row
      title={
        <>
          {document.subjects.map(subjectLabel).join(" / ")}
          {document.role && (
            <Tag tone={document.role === "answer" ? "ok" : "neutral"}>
              {lookup(`examShare.role.${document.role}`)}
            </Tag>
          )}
        </>
      }
      meta={[
        [document.academicYear, documentOccasion(document)]
          .filter(Boolean)
          .join(" "),
        teacherNames(document.teachers),
        document.note,
      ]
        .filter(Boolean)
        .join(" · ")}
      current={current}
      pending={opening === document.id}
      onClick={() => open(document.id)}
      trailing={trailing}
    />
  );
}

const requestFacts = (item: ExamRequest) =>
  [
    item.academicYear,
    item.occasion && lookup(`examShare.occasion.${item.occasion}`),
    item.wantedRole && lookup(`examShare.role.${item.wantedRole}`),
  ]
    .filter(Boolean)
    .join(" · ");

function RequestList({
  call,
  open,
  opening,
  reading,
  pane,
  generation,
  setViewUrl,
  closeReader,
  openUpload,
}: ViewProps & { closeReader: () => void; openUpload: OpenUpload }) {
  const [mine, setMine] = useState(false);
  const [selected, setSelected] = useState<string>();
  const [page, setPage] = useState<ExamRequestPage & { mine: boolean }>();
  const [failed, setFailed] = useState(false);
  const [more, setMore] = useState(false);
  const show = useCallback(
    (personal: boolean, value: ExamRequestPage) =>
      setPage({ ...value, mine: personal }),
    [],
  );
  const load = useCallback(
    async (personal: boolean, cursor: ExamRequestCursor | null = null) => {
      setFailed(false);
      const answer = await call({ kind: "requests", mine: personal, cursor });
      if (answer?.kind !== "requests") return setFailed(!cursor);
      setPage((previous) => ({
        ...answer.value,
        mine: personal,
        items:
          cursor && previous?.mine === personal
            ? [...previous.items, ...answer.value.items]
            : answer.value.items,
      }));
    },
    [call],
  );
  useEffect(() => {
    void load(mine);
  }, [mine, load]);
  useReload(generation, () => void load(mine));
  const items = page?.mine === mine ? page.items : undefined;
  const shownUrl = page?.webUrl;
  useEffect(() => setViewUrl(shownUrl), [shownUrl, setViewUrl]);
  const chosen = items?.find((item) => item.id === selected);

  return (
    <Workspace>
      <div className="examPane">
        <SegmentedControl
          value={mine ? "mine" : "open"}
          onChange={(value) => setMine(value === "mine")}
          data={[
            { value: "open", label: t("examShare.openRequests") },
            { value: "mine", label: t("examShare.myRequests") },
          ]}
        />
        <div className="scroll">
          {failed ? (
            <ErrorBanner onRetry={() => void load(mine)}>
              {t("examShare.loadFailed")}
            </ErrorBanner>
          ) : !items ? (
            <Skeleton count={5} />
          ) : items.length ? (
            <>
              <ul className="examList">
                {items.map((item) => (
                  <Row
                    key={item.id}
                    title={
                      <>
                        {subjectLabel(item.subject)}
                        {item.status !== "open" && (
                          <Tag
                            tone={
                              item.status === "fulfilled" ? "ok" : "neutral"
                            }
                          >
                            {lookup(`examShare.requestStatus.${item.status}`) ??
                              item.status}
                          </Tag>
                        )}
                      </>
                    }
                    meta={
                      [requestFacts(item), item.description]
                        .filter(Boolean)
                        .join(" · ") || item.subject.code
                    }
                    current={selected === item.id}
                    onClick={() => {
                      setSelected(item.id);
                      closeReader();
                    }}
                  />
                ))}
              </ul>
              {page?.cursor && (
                <Button
                  variant="subtle"
                  className="examMore"
                  loading={more}
                  onClick={async () => {
                    setMore(true);
                    await load(mine, page.cursor);
                    setMore(false);
                  }}
                >
                  {t("examShare.more")}
                </Button>
              )}
            </>
          ) : (
            <EmptyState compact>
              {t(mine ? "examShare.noMyRequests" : "examShare.noOpenRequests")}
            </EmptyState>
          )}
        </div>
      </div>
      <div className="examPane examDetail">
        {pane ||
          (chosen ? (
            <RequestDetail
              item={chosen}
              opening={opening === chosen.answerExamId}
              reading={!!chosen.answerExamId && reading === chosen.answerExamId}
              view={() => chosen.answerExamId && open(chosen.answerExamId)}
              answer={() => openUpload({ kind: "request", request: chosen })}
              setOpen={async (next) => {
                const answer = await call({
                  kind: "setRequestOpen",
                  id: chosen.id,
                  open: next,
                  mine,
                });
                if (answer?.kind === "requests") show(mine, answer.value);
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
                  id: chosen.id,
                  mine,
                });
                if (answer?.kind === "requests") {
                  setSelected(undefined);
                  show(mine, answer.value);
                }
              }}
            />
          ) : (
            <EmptyState icon={Inbox}>{t("examShare.pickRequest")}</EmptyState>
          ))}
      </div>
    </Workspace>
  );
}

/** One request in full: what is wanted and from whom, what it is for, and
 * what to do next: share a file, see the shared one, or manage one's own. */
function RequestDetail({
  item,
  opening,
  reading,
  view,
  answer,
  setOpen,
  remove,
}: {
  item: ExamRequest;
  opening: boolean;
  reading: boolean;
  view: () => void;
  answer: () => void;
  setOpen: (open: boolean) => Promise<void>;
  remove: () => Promise<void>;
}) {
  const facts = requestFacts(item);
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
      <div className="examRequestActions">
        {item.answerExamId && (
          <Button onClick={view} loading={opening} disabled={reading}>
            {t("examShare.viewAnswer")}
          </Button>
        )}
        {!item.mine && item.status === "open" && (
          <Button onClick={answer}>
            <Upload size={15} strokeWidth={1.75} aria-hidden />
            {t("examShare.upload.answerRequest")}
          </Button>
        )}
        {item.mine && (
          <>
            <Button
              variant="default"
              onClick={() => void setOpen(item.status !== "open")}
            >
              {t(
                item.status === "open"
                  ? "examShare.closeRequest"
                  : "examShare.reopenRequest",
              )}
            </Button>
            <Button variant="subtle" color="red" onClick={() => void remove()}>
              {t("common.delete")}
            </Button>
          </>
        )}
      </div>
    </article>
  );
}

function LibraryList({
  call,
  open,
  opening,
  reading,
  pane,
  setViewUrl,
  library,
  failed,
  refresh,
  replace,
  closeReader,
}: Omit<ViewProps, "generation"> & {
  library?: ExamLibrary;
  failed: boolean;
  refresh: () => void;
  replace: (value: ExamLibrary) => void;
  closeReader: (id: string) => void;
}) {
  const shownUrl = library?.webUrl;
  useEffect(() => setViewUrl(shownUrl), [shownUrl, setViewUrl]);

  const remove = (kind: "removeDownload" | "deleteSubmission", id: string) => {
    const label = t(
      kind === "deleteSubmission"
        ? "examShare.deleteSubmission"
        : "examShare.removeDownload",
    );
    return (
      <Button
        variant="subtle"
        color="gray"
        size="compact-sm"
        className="examRowTool"
        aria-label={label}
        title={label}
        onClick={async () => {
          if (!(await confirmAction(label, t("common.delete")))) return;
          const answer = await call({ kind, id });
          if (answer?.kind === "library") {
            closeReader(id);
            replace(answer.value);
          }
        }}
      >
        <Trash2 size={15} strokeWidth={1.75} aria-hidden />
      </Button>
    );
  };
  const section = (
    key: "saved" | "downloaded" | "submitted",
    items: ExamDocument[],
    removal?: "removeDownload" | "deleteSubmission",
  ) =>
    items.length > 0 && (
      <section key={key}>
        <h2 className="examListHeading">{t(`examShare.${key}`)}</h2>
        <ul className="examList">
          {items.map((document) => (
            <DocumentRow
              key={document.id}
              document={document}
              open={open}
              opening={opening}
              current={reading === document.id}
              trailing={removal && remove(removal, document.id)}
            />
          ))}
        </ul>
      </section>
    );

  const empty =
    library &&
    !library.saved.length &&
    !library.downloaded.length &&
    !library.submitted.length;
  return (
    <Workspace>
      <div className="examPane">
        <div className="scroll">
          {failed && !library ? (
            <ErrorBanner onRetry={refresh}>
              {t("examShare.loadFailed")}
            </ErrorBanner>
          ) : !library ? (
            <Skeleton count={6} />
          ) : (
            <>
              {library.offline && (
                <ErrorBanner onRetry={refresh}>
                  {t("examShare.libraryUnavailable")}
                </ErrorBanner>
              )}
              {empty ? (
                <EmptyState compact icon={Library}>
                  {t("examShare.emptyLibrary")}
                </EmptyState>
              ) : (
                <>
                  {section("saved", library.saved)}
                  {section("downloaded", library.downloaded, "removeDownload")}
                  {section("submitted", library.submitted, "deleteSubmission")}
                </>
              )}
            </>
          )}
        </div>
      </div>
      <div className="examPane examDetail">
        {pane || (
          <EmptyState icon={Library}>{t("examShare.pickDocument")}</EmptyState>
        )}
      </div>
    </Workspace>
  );
}
