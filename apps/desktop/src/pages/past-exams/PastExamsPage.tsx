import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Bookmark,
  BookmarkCheck,
  BookOpen,
  ChevronDown,
  ChevronRight,
  Download,
  FileText,
  Flag,
  Globe,
  Inbox,
  Library,
  Loader2,
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
import { localFileUrl } from "../../features/lms/files";
import { ReaderPane, ReaderTools } from "../../features/lms/ReaderPane";
import { lookup, t } from "../../i18n";
import { pickLocalized, type Lang } from "../../lib/model";
import type {
  ExamAccount,
  ExamAction,
  ExamAnswer,
  ExamCourseImportPreview,
  ExamDocument,
  ExamLibrary,
  ExamLocalDocument,
  ExamRequest,
  ExamRequestCursor,
  ExamRequestPage,
  ExamSet,
  ExamSubject,
  ExamSubjectPage,
  ExamTeacher,
  ExamUploadDraft,
} from "../../generated/persistence";
import { ImageView } from "../../features/lms/ImageView";
import {
  ReportDialog,
  RequestDialog,
  UploadDialog,
  type UploadTarget,
} from "./ExamDialogs";
import "./past-exams.css";

const Pdf = lazy(() =>
  import("../../features/lms/PdfDocument").then((m) => ({
    default: m.PdfDocument,
  })),
);
const tabs = ["browse", "requests", "library"] as const;
const SITE = "https://sciencetokyoexam.com";
type Tab = (typeof tabs)[number];

/** Runs one Core action. Failures go to the toast; a lost session resets
 * the page through the account snapshot. Cancelled sign-ins are silent. */
function useExamShare(onAccount: (account: ExamAccount) => void) {
  return useCallback(
    async (action: ExamAction): Promise<ExamAnswer | undefined> => {
      const run = desktopBridge?.examShare;
      try {
        if (!run) throw new Error(t("error.exam-share-unavailable"));
        return await run(action);
      } catch (e) {
        if (errorCode(e) === "exam-share-cancelled") return undefined;
        notify.error(e);
        const snapshot = await run?.({ kind: "accountSnapshot" }).catch(
          () => undefined,
        );
        if (snapshot?.kind === "account") onAccount(snapshot.value);
        return undefined;
      }
    },
    [onAccount],
  );
}
type Call = ReturnType<typeof useExamShare>;

/** Shares a past exam; true once it is on the site. */
async function uploadExam(call: Call, draft: ExamUploadDraft) {
  const answer = await call({ kind: "upload", draft });
  if (answer?.kind !== "uploaded") return false;
  notify.info(t("examShare.uploaded"));
  return true;
}

/** Each teacher once, in the order the subject lists them. */
const uniqueTeachers = (teachers: ExamTeacher[]) =>
  teachers.filter(
    (teacher, index) =>
      teachers.findIndex(({ id }) => id === teacher.id) === index,
  );

/** Labels shared by every list and the reader title. */
function useLabels(language: Lang) {
  return useMemo(() => {
    const subject = (value: ExamSubject) =>
      pickLocalized(
        { ja: value.name, en: value.nameEn ?? "" },
        language,
        value.code,
      );
    const occasion = (document: ExamDocument) =>
      [
        document.occasion && lookup(`examShare.occasion.${document.occasion}`),
        document.occasionNumber > 1 ? document.occasionNumber : null,
        document.variant,
      ]
        .filter(Boolean)
        .join(" ") || document.fileName;
    const title = (document: ExamDocument) =>
      [
        document.subjects.map(subject).join(" / "),
        document.academicYear,
        occasion(document),
        document.role && lookup(`examShare.role.${document.role}`),
      ]
        .filter(Boolean)
        .join(" · ");
    return { subject, occasion, title };
  }, [language]);
}
type Labels = ReturnType<typeof useLabels>;

export function PastExams({
  active,
  currentTerm,
  language,
}: {
  active: boolean;
  currentTerm: string;
  terms: string[];
  language: Lang;
}) {
  const [account, setAccount] = useState<ExamAccount>();
  const call = useExamShare(setAccount);
  const labels = useLabels(language);
  const [tab, setTab] = useState<Tab>("browse");
  const [pending, setPending] = useState(false);
  const [reader, setReader] = useState<ExamLocalDocument>();
  const [opening, setOpening] = useState<string>();

  const accountAction = useCallback(
    async (kind: "account" | "connect" | "disconnect" | "acceptTerms") => {
      setPending(kind !== "account");
      const answer = await call({ kind });
      setPending(false);
      if (answer?.kind === "account") setAccount(answer.value);
    },
    [call],
  );
  useEffect(() => {
    if (active) void accountAction("account");
  }, [active, accountAction]);
  const connected = account?.status === "connected";
  useEffect(() => {
    if (!connected) setReader(undefined);
  }, [connected]);

  // The last document clicked wins: an earlier one still loading is
  // dropped when it arrives, so a slow file cannot replace the new one.
  const openSeq = useRef(0);
  const open = useCallback(
    async (id: string, download = false) => {
      const seq = ++openSeq.current;
      setOpening(id);
      const answer = await call({ kind: "open", id, download });
      if (seq !== openSeq.current) return;
      setOpening(undefined);
      if (answer?.kind === "document") {
        setReader(answer.value);
        if (download) notify.info(t("examShare.downloadedNotice"));
      }
    },
    [call],
  );
  const switchTab = (next: string) => {
    setTab(next as Tab);
    setReader(undefined);
  };

  // Opening another document shows that it is coming; a download of the
  // one on screen keeps it there.
  const readerPane =
    opening && opening !== reader?.document.id ? (
      <div className="examOpening" role="status">
        <Loader2
          size={20}
          strokeWidth={1.75}
          className="examSpin"
          aria-hidden
        />
        {t("examShare.opening")}
      </div>
    ) : (
      reader && (
        <ExamReader
          key={reader.document.id}
          file={reader}
          title={labels.title(reader.document)}
          call={call}
          download={() => void open(reader.document.id, true)}
          downloading={opening === reader.document.id}
          setFile={setReader}
          close={() => setReader(undefined)}
        />
      )
    );

  // The site's page for what is on screen: the open document, else the tab.
  const siteUrl = `${SITE}${
    reader
      ? `/view/${encodeURIComponent(reader.document.id)}`
      : tab === "requests"
        ? "/request"
        : tab === "library"
          ? "/library"
          : "/search"
  }`;

  return (
    <section className="page pastExams">
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
          <ExternalLink className="examSiteLink" href={siteUrl}>
            <Globe size={14} strokeWidth={1.75} aria-hidden />
            {t("examShare.openInBrowser")}
          </ExternalLink>
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
      {!connected ? (
        <AccountGate
          account={account}
          pending={pending}
          connect={() => void accountAction("connect")}
          accept={() => void accountAction("acceptTerms")}
        />
      ) : (
        <>
          {tab === "browse" ? (
            <BrowseList
              active={active}
              currentTerm={currentTerm}
              call={call}
              labels={labels}
              open={(id) => void open(id)}
              opening={opening}
              reading={reader?.document.id}
              pane={readerPane}
            />
          ) : tab === "requests" ? (
            <RequestList
              call={call}
              labels={labels}
              open={(id) => void open(id)}
              opening={opening}
              reading={reader?.document.id}
              closeReader={() => setReader(undefined)}
              pane={readerPane}
            />
          ) : (
            <LibraryList
              call={call}
              labels={labels}
              open={(id) => void open(id)}
              opening={opening}
              reading={reader?.document.id}
              refreshKey={reader}
              closeReader={(id) => {
                if (reader?.document.id === id) setReader(undefined);
              }}
              pane={readerPane}
            />
          )}
        </>
      )}
    </section>
  );
}

/** The list on the left and what it opens on the right; one layout for all three tabs. */
function Workspace({
  children,
}: {
  children: [React.ReactNode, React.ReactNode];
}) {
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
  if (account.status === "unavailable")
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
  title: React.ReactNode;
  meta?: React.ReactNode;
  current?: boolean;
  pending?: boolean;
  disabled?: boolean;
  onClick?: () => void;
  trailing?: React.ReactNode;
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

/** One course or search hit on the left; the instructor is set only for
 * timetable courses, so Core can tell the person's own teachers. */
type Entry = {
  key: string;
  code: string | undefined;
  title: string;
  meta: string;
  instructor: string | null;
};

/** Left: this quarter's timetable or a search, each subject an expansion
 * panel of its sittings. Right: always the reader. */
function BrowseList({
  active,
  currentTerm,
  call,
  labels,
  open,
  opening,
  reading,
  pane,
}: {
  active: boolean;
  currentTerm: string;
  call: Call;
  labels: Labels;
  open: (id: string) => void;
  opening?: string;
  reading?: string;
  pane: React.ReactNode;
}) {
  const [query, setQuery] = useState("");
  const [debounced] = useDebounceValue(query.trim(), 300);
  const [hits, setHits] = useState<{ query: string; items: ExamSubject[] }>();
  const [expanded, setExpanded] = useState("");
  const [pages, setPages] = useState<
    Record<string, { page?: ExamSubjectPage; failed?: boolean }>
  >({});
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

  const timetable = useScheduleQuery<ExamCourseImportPreview>(
    { kind: "examCourseImport", term: null },
    active && !query.trim(),
  );
  const courses =
    timetable.value?.term === currentTerm ? timetable.value.courses : undefined;

  const load = async (entry: Entry) => {
    if (!entry.code) return;
    const code = entry.code;
    setPages((all) => ({ ...all, [entry.key]: {} }));
    const answer = await call({
      kind: "subject",
      code,
      instructor: entry.instructor,
    });
    setPages((all) => ({
      ...all,
      [entry.key]:
        answer?.kind === "subject" ? { page: answer.value } : { failed: true },
    }));
  };

  const searching = !!query.trim();
  const entries: Entry[] | undefined = searching
    ? hits?.query === debounced && debounced === query.trim()
      ? hits.items.map((entry) => ({
          key: `search:${entry.code}`,
          code: entry.code,
          title: labels.subject(entry),
          meta: [
            entry.code,
            entry.examCount
              ? t("examShare.documentCount", { count: entry.examCount })
              : null,
          ]
            .filter(Boolean)
            .join(" · "),
          instructor: null,
        }))
      : undefined
    : courses?.map((course) => ({
        key: `course:${course.courseId}`,
        code: course.subjectCodes[0],
        title: course.title,
        meta:
          [course.subjectCodes.join(" / "), course.instructor]
            .filter(Boolean)
            .join(" · ") || t("examShare.noSubjectCode"),
        instructor: course.instructor || null,
      }));

  return (
    <Workspace>
      <div className="examPane">
        <Input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.currentTarget.value)}
          maxLength={100}
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
                <h2 className="examListHeading">{t("examShare.myCourses")}</h2>
              )}
              <Accordion.Root
                type="single"
                collapsible
                value={expanded}
                onValueChange={(key) => {
                  setExpanded(key);
                  const entry = entries.find((e) => e.key === key);
                  if (entry && !pages[key]?.page) void load(entry);
                }}
                className="examPanels"
              >
                {entries.map((entry) => (
                  <Accordion.Item
                    key={entry.key}
                    value={entry.key}
                    disabled={!entry.code}
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
                        name={entry.title}
                        state={pages[entry.key]}
                        call={call}
                        open={open}
                        opening={opening}
                        reading={reading}
                        retry={() => void load(entry)}
                      />
                    </Accordion.Content>
                  </Accordion.Item>
                ))}
              </Accordion.Root>
            </>
          ) : (
            <EmptyState compact icon={Search}>
              {t(searching ? "examShare.noSubjects" : "examShare.searchHint")}
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

/** A subject's sittings inside its panel: the person's own teachers first,
 * the rest behind one toggle, then a way to ask for what is missing. */
function SubjectSets({
  name,
  state,
  call,
  open,
  opening,
  reading,
  retry,
}: {
  name: string;
  state?: { page?: ExamSubjectPage; failed?: boolean };
  call: Call;
  open: (id: string) => void;
  opening?: string;
  reading?: string;
  retry: () => void;
}) {
  const [requesting, setRequesting] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [othersOpen, setOthersOpen] = useState(false);
  if (state?.failed)
    return (
      <ErrorBanner onRetry={retry}>
        {t("error.exam-share-response")}
      </ErrorBanner>
    );
  const page = state?.page;
  if (!page) return <Skeleton count={3} />;
  const grouped = page.myTeachers.length > 0;
  const mine = page.sets.filter((set) => !grouped || set.mine);
  const others = grouped ? page.sets.filter((set) => !set.mine) : [];
  const rows = (sets: ExamSet[]) => (
    <ul className="examSets">
      {sets.map((set) => (
        <SetRow
          key={set.documents[0].id}
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
      {!page.sets.length ? (
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
          onClick={() => setUploading(true)}
        >
          {t("examShare.uploadHere")}
        </Button>
        <Button
          variant="subtle"
          size="compact-sm"
          onClick={() => setRequesting(true)}
        >
          {t("examShare.newRequest")}
        </Button>
      </div>
      {uploading && (
        <UploadDialog
          target={{
            subjectCode: page.subject.code,
            subjectName: name,
            teachers: uniqueTeachers(page.rows.flatMap((row) => row.teachers)),
            mine: page.myTeachers,
          }}
          close={() => setUploading(false)}
          submit={async (draft) => {
            const done = await uploadExam(call, draft);
            if (done) retry();
            return done;
          }}
        />
      )}
      <RequestDialog
        opened={requesting}
        subject={page}
        close={() => setRequesting(false)}
        submit={async (draft) => {
          if (await call({ kind: "createRequest", draft })) {
            setRequesting(false);
            notify.info(t("examShare.requestCreated"));
          }
        }}
      />
    </>
  );
}

/** One sitting: the row opens its first document, and each document has its
 * own button (question, answer) beside it. */
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
  const occasion =
    [
      set.occasion && lookup(`examShare.occasion.${set.occasion}`),
      set.occasionNumber > 1 ? set.occasionNumber : null,
      set.variant,
    ]
      .filter(Boolean)
      .join(" ") || set.documents[0].fileName;
  const current = set.documents.some((d) => d.id === reading);
  // The sitting is a label; its documents are the buttons.
  return (
    <li className="examSet" aria-current={current || undefined}>
      <div className="examSetMain">
        <FileText size={15} strokeWidth={1.75} aria-hidden />
        <span className="examSetYear">
          {set.academicYear ?? t("examShare.unknownYearShort")}
        </span>
        <span className="examSetText">
          <strong>{occasion}</strong>
          {withTeachers && (
            <small>
              {set.teachers.map(({ name }) => name).join(" / ") ||
                t("examShare.unknownTeacher")}
            </small>
          )}
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
              <Loader2
                size={12}
                strokeWidth={2}
                className="examSpin"
                aria-hidden
              />
            )}
            {document.role
              ? lookup(`examShare.role.${document.role}`)
              : t("examShare.file")}
          </button>
        ))}
      </span>
    </li>
  );
}

function DocumentRow({
  document,
  labels,
  open,
  opening,
  current,
  withSubject,
  trailing,
}: {
  document: ExamDocument;
  labels: Labels;
  open: (id: string) => void;
  opening?: string;
  current?: boolean;
  withSubject?: boolean;
  trailing?: React.ReactNode;
}) {
  return (
    <Row
      title={
        <>
          {withSubject
            ? document.subjects.map(labels.subject).join(" / ")
            : labels.occasion(document)}
          {document.role && (
            <Tag tone={document.role === "answer" ? "ok" : "neutral"}>
              {lookup(`examShare.role.${document.role}`)}
            </Tag>
          )}
        </>
      }
      meta={[
        withSubject &&
          [document.academicYear, labels.occasion(document)]
            .filter(Boolean)
            .join(" "),
        document.teachers.map(({ name }) => name).join(" / ") ||
          t("examShare.unknownTeacher"),
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

/** The opened document in the shared LMS reader frame, with its own tools. */
function ExamReader({
  file,
  title,
  call,
  download,
  downloading,
  setFile,
  close,
}: {
  file: ExamLocalDocument;
  title: string;
  call: Call;
  download: () => void;
  downloading: boolean;
  setFile: (file: ExamLocalDocument) => void;
  close: () => void;
}) {
  const [reporting, setReporting] = useState(false);
  const [saving, setSaving] = useState(false);
  const { document } = file;
  const pdf = document.fileType === "application/pdf";
  // The LMS reader's own toolbar shape, so these sit in one row with the
  // PDF page and zoom controls (workspace-chrome.css, .lmsReaderToolSlot).
  const tool = (
    label: string,
    icon: React.ReactNode,
    onClick: () => void,
    { disabled = false, pressed = undefined as boolean | undefined } = {},
  ) => (
    <button
      type="button"
      aria-label={label}
      title={label}
      aria-pressed={pressed}
      disabled={disabled}
      onClick={onClick}
    >
      {icon}
    </button>
  );
  const toolbar = (
    <div className="lmsSourceBar lmsFileToolbar">
      <div className="lmsFileActions">
        {file.offline && <Tag>{t("examShare.offline")}</Tag>}
        {tool(
          t(file.downloaded ? "examShare.downloaded" : "examShare.download"),
          <Download size={15} strokeWidth={1.75} aria-hidden />,
          download,
          { disabled: file.downloaded || file.offline || downloading },
        )}
        {file.saved !== null &&
          tool(
            t(file.saved ? "examShare.unsave" : "examShare.save"),
            file.saved ? (
              <BookmarkCheck size={15} strokeWidth={1.75} aria-hidden />
            ) : (
              <Bookmark size={15} strokeWidth={1.75} aria-hidden />
            ),
            async () => {
              setSaving(true);
              const answer = await call({
                kind: "save",
                saved: !file.saved,
                id: document.id,
              });
              setSaving(false);
              if (answer?.kind === "saved")
                setFile({ ...file, saved: answer.value });
            },
            { disabled: file.offline || saving, pressed: file.saved },
          )}
        {tool(
          t("examShare.report"),
          <Flag size={15} strokeWidth={1.75} aria-hidden />,
          () => setReporting(true),
          { disabled: file.offline },
        )}
      </div>
    </div>
  );
  return (
    <ReaderPane title={title} close={close}>
      {!pdf && <ReaderTools>{toolbar}</ReaderTools>}
      {pdf ? (
        <Suspense fallback={<Skeleton count={6} />}>
          <Pdf path={file.path} title={title} fileToolbar={toolbar} />
        </Suspense>
      ) : document.fileType === "text/plain" ? (
        <PlainTextPreview key={file.path} path={file.path} />
      ) : (
        <ImageView src={localFileUrl(file.path)} title={title} />
      )}
      <ReportDialog
        opened={reporting}
        close={() => setReporting(false)}
        submit={async (reason, detail) => {
          if (await call({ kind: "report", id: document.id, reason, detail })) {
            setReporting(false);
            notify.info(t("examShare.reported"));
          }
        }}
      />
    </ReaderPane>
  );
}

function RequestList({
  call,
  labels,
  open,
  opening,
  reading,
  closeReader,
  pane,
}: {
  call: Call;
  labels: Labels;
  open: (id: string) => void;
  opening?: string;
  reading?: string;
  closeReader: () => void;
  pane: React.ReactNode;
}) {
  const [mine, setMine] = useState(false);
  const [selected, setSelected] = useState<string>();
  const [answering, setAnswering] = useState<UploadTarget>();
  const [page, setPage] = useState<ExamRequestPage & { mine: boolean }>();
  const [failed, setFailed] = useState(false);
  const [more, setMore] = useState(false);
  const load = useCallback(
    async (personal: boolean, cursor: ExamRequestCursor | null = null) => {
      setFailed(false);
      const answer = await call({ kind: "requests", mine: personal, cursor });
      if (answer?.kind !== "requests") return setFailed(!cursor);
      setPage((previous) => ({
        mine: personal,
        cursor: answer.value.cursor,
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
  const items = page?.mine === mine ? page.items : undefined;
  const chosen = items?.find((item) => item.id === selected);
  const facts = (item: ExamRequest) =>
    [
      item.academicYear,
      item.occasion && lookup(`examShare.occasion.${item.occasion}`),
      item.wantedRole && lookup(`examShare.role.${item.wantedRole}`),
    ]
      .filter(Boolean)
      .join(" · ");

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
              {t("error.exam-share-response")}
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
                        {labels.subject(item.subject)}
                        {item.status !== "open" && (
                          <Tag
                            tone={
                              item.status === "fulfilled" ? "ok" : "neutral"
                            }
                          >
                            {lookup(`examShare.requestStatus.${item.status}`)}
                          </Tag>
                        )}
                      </>
                    }
                    meta={
                      [facts(item), item.description]
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
        {answering && (
          <UploadDialog
            target={answering}
            close={() => setAnswering(undefined)}
            submit={async (draft) => {
              const done = await uploadExam(call, draft);
              if (done) await load(mine);
              return done;
            }}
          />
        )}
      </div>
      <div className="examPane examDetail">
        {pane ||
          (chosen ? (
            <RequestDetail
              item={chosen}
              labels={labels}
              facts={facts(chosen)}
              opening={opening === chosen.answerExamId}
              reading={!!chosen.answerExamId && reading === chosen.answerExamId}
              view={() => chosen.answerExamId && open(chosen.answerExamId)}
              answer={() =>
                setAnswering({
                  subjectCode: chosen.subject.code,
                  subjectName: labels.subject(chosen.subject),
                  teachers: chosen.teachers,
                  mine: chosen.teachers.map(({ id }) => id),
                  academicYear: chosen.academicYear,
                  occasion: chosen.occasion,
                  role: chosen.wantedRole,
                  requestId: chosen.id,
                })
              }
              setOpen={async (next) => {
                if (
                  await call({
                    kind: "setRequestOpen",
                    id: chosen.id,
                    open: next,
                  })
                )
                  await load(mine);
              }}
              remove={async () => {
                if (
                  (await confirmAction(
                    t("examShare.deleteRequest"),
                    t("common.delete"),
                  )) &&
                  (await call({ kind: "deleteRequest", id: chosen.id }))
                ) {
                  setSelected(undefined);
                  await load(mine);
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
 * the one thing to do next — share a file, see the shared one, or manage
 * one's own. */
function RequestDetail({
  item,
  labels,
  facts,
  opening,
  reading,
  view,
  answer,
  setOpen,
  remove,
}: {
  item: ExamRequest;
  labels: Labels;
  facts: string;
  opening: boolean;
  reading: boolean;
  view: () => void;
  answer: () => void;
  setOpen: (open: boolean) => Promise<void>;
  remove: () => Promise<void>;
}) {
  return (
    <article className="examRequest">
      <header>
        <small>{item.subject.code}</small>
        <h2>{labels.subject(item.subject)}</h2>
        <Tag
          tone={
            item.status === "fulfilled"
              ? "ok"
              : item.status === "open"
                ? "accent"
                : "neutral"
          }
        >
          {lookup(`examShare.requestStatus.${item.status}`)}
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
            {t("examShare.answerRequest")}
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
  labels,
  open,
  opening,
  reading,
  refreshKey,
  closeReader,
  pane,
}: {
  call: Call;
  labels: Labels;
  open: (id: string) => void;
  opening?: string;
  reading?: string;
  /** Reloads when the reader changes what is saved or downloaded. */
  refreshKey: unknown;
  closeReader: (id: string) => void;
  pane: React.ReactNode;
}) {
  const [library, setLibrary] = useState<ExamLibrary>();
  const [failed, setFailed] = useState(false);
  const load = useCallback(async () => {
    setFailed(false);
    const answer = await call({ kind: "library" });
    if (answer?.kind === "library") setLibrary(answer.value);
    else setFailed(true);
  }, [call]);
  useEffect(() => {
    void load();
  }, [load, refreshKey]);

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
          if (await call({ kind, id })) {
            closeReader(id);
            await load();
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
              labels={labels}
              open={open}
              opening={opening}
              current={reading === document.id}
              withSubject
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
            <ErrorBanner onRetry={() => void load()}>
              {t("error.exam-share-response")}
            </ErrorBanner>
          ) : !library ? (
            <Skeleton count={6} />
          ) : (
            <>
              {library.offline && (
                <ErrorBanner onRetry={() => void load()}>
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
  if (failed)
    return <ErrorBanner>{t("error.exam-share-response")}</ErrorBanner>;
  return text === undefined ? (
    <Skeleton count={6} />
  ) : (
    <pre className="examDocumentText">{text}</pre>
  );
}
