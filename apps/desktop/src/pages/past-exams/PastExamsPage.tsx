import {
  useCallback,
  useEffect,
  useEffectEvent,
  useRef,
  useState,
} from "react";
import { TitleRow } from "../../components/TitleRow";
import { Globe, LoaderCircle } from "lucide-react";
import { Button, Menu, SegmentedControl } from "../../components/ui";
import { confirmAction } from "../../components/confirm";
import { ExternalLink } from "../../components/ExternalLink";
import { desktopBridge } from "../../platform/native";
import { notify } from "../../components/notifications/notify";
import { errorCode } from "../../platform/native-error";
import { useRecords } from "../../platform/records";
import { t } from "../../i18n";
import type {
  ExamAccount,
  ExamAction,
  ExamChoices,
  ExamDocument,
  ExamLocalDocument,
  ExamSitePages,
  ExamUploadForm,
  ExamUploadPhase,
  ExamUploadStart,
} from "../../generated/persistence";
import { UploadDialog, UploadProgress, UploadRecovery } from "./ExamDialogs";
import { ExamReader } from "./ExamReader";
import { useExamLibrary } from "./useExamLibrary";
import "./past-exams.css";
import { AccountGate, type GateAction } from "./AccountGate";
import { BrowseList } from "./BrowseList";
import { LibraryList } from "./LibraryList";
import { RequestList } from "./RequestList";
import {
  documentOccasion,
  ExamChrome,
  loadChoices,
  roleName,
  subjectLabel,
  useReload,
  type Call,
} from "./ExamList";

const tabs = ["browse", "requests", "library"] as const;
type Tab = (typeof tabs)[number];
type AccountAction = GateAction | "disconnect" | "deleteAccount";
/** The upload phases in which a file is on its way; the account stays
 * until it has arrived or stopped. */
const uploading: ExamUploadPhase[] = [
  "preparing",
  "transferring",
  "registering",
  "cleaning",
];
/** The website's page for each tab. */
const tabPage = {
  browse: "search",
  requests: "requests",
  library: "library",
} as const satisfies Record<Tab, keyof ExamSitePages>;

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

const documentTitle = (document: ExamDocument) =>
  [
    document.subjects.map(subjectLabel).join(" / "),
    document.academicYear,
    documentOccasion(document),
    roleName(document.role, document.roleText),
  ]
    .filter(Boolean)
    .join(" · ");

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
  const [site, setSite] = useState<ExamSitePages>();
  const [tab, setTab] = useState<Tab>("browse");
  const [pending, setPending] = useState<AccountAction>();
  const [reader, setReader] = useState<ExamLocalDocument>();
  /** The document being opened; only its button spins. */
  const [opening, setOpening] = useState<string>();
  /** Bumped when every view should read Core again: back in the
   * foreground, or after an upload changed what the site lists. */
  const [generation, setGeneration] = useState(0);
  /** Core's address for the subject on screen, once it has loaded. */
  const [viewUrl, setViewUrl] = useState<string>();
  const [upload, setUpload] = useState<{
    form: ExamUploadForm;
    choices: ExamChoices;
  }>();
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
      setViewUrl(undefined);
      openSeq.current += 1;
      setOpening(undefined);
      setReader(undefined);
      replaceLibrary(undefined);
      closeUpload();
    },
    [closeUpload, replaceLibrary],
  );
  const accountAction = useCallback(
    async (kind: "account" | AccountAction) => {
      setPending(kind === "account" ? undefined : kind);
      const answer = await call({ kind });
      setPending(undefined);
      if (answer?.kind === "account") applyAccount(answer.value);
      return answer?.kind === "account" ? answer.value : undefined;
    },
    [applyAccount, call],
  );
  // Every bump of `generation` reads the library again too. The first
  // activation bumps nothing, so the library's own first read is the only one.
  useReload(generation, refreshLibrary);
  // Back in the foreground: read the account, then the views again.
  const returnToForeground = useEffectEvent(async () => {
    if (inFlight.current > 0 || document.visibilityState === "hidden") return;
    const was = account?.status;
    if ((await accountAction("account"))?.status === "connected" && was)
      setGeneration((value) => value + 1);
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
  // Core answers the website's pages locally; the page links to them in
  // every account state. The forms' choices wait for a form to open.
  useEffect(() => {
    if (!active || site) return;
    void call({ kind: "site" }).then((answer) => {
      if (answer?.kind === "site") setSite(answer.value);
    });
  }, [active, site, call]);

  const open = useCallback(
    async (id: string) => {
      const seq = ++openSeq.current;
      setOpening(id);
      const answer = await call({ kind: "open", id, download: false });
      if (seq !== openSeq.current) return;
      setOpening(undefined);
      if (answer?.kind === "document") setReader(answer.value);
    },
    [call],
  );
  // Core names subjects in the content language, which these two settings
  // decide (`approach/content-language-in-core`). When either changes, every
  // view and the open document are read from Core again; the page keeps what
  // it shows until the new answers arrive.
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
      // One after the other, so a failure is told once.
      const choices = await loadChoices(call);
      if (!choices) return;
      const answer = await call({ kind: "uploadForm", start });
      if (answer?.kind === "uploadForm")
        setUpload({ form: answer.value, choices });
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
      if (answer.kind === "uploaded") closeUpload();
      // The upload changed what every view lists.
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

  // Opening another document shows that it is coming; the one on screen
  // stays while its own tools act on it.
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
        <ExamReader
          key={reader.document.id}
          file={reader}
          title={documentTitle(reader.document)}
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
  // The object on screen first, else the tab's page; before connecting the
  // tabs are hidden, so the search.
  const webUrl = connected
    ? (reader?.webUrl ?? viewUrl ?? site?.[tabPage[tab]])
    : site?.search;
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
  };

  const partner = <span className="examPartner">{t("examShare.partner")}</span>;
  const siteLink = webUrl && (
    <ExternalLink className="examSiteLink" href={webUrl}>
      <Globe size={14} strokeWidth={1.75} aria-hidden />
      {t("examShare.openOnWeb")}
    </ExternalLink>
  );
  const chrome = account && {
    head: (
      <>
        <TitleRow nav>
          <h1>{t("examShare.title")}</h1>
          <span className="titleRowSpacer" />
          {siteLink}
        </TitleRow>
        <SegmentedControl
          className="examTabs"
          value={tab}
          onChange={switchTab}
          data={tabs.map((value) => ({
            value,
            label: t(`examShare.${value}`),
          }))}
        />
        {!upload &&
          (uploadRunning ? (
            <UploadProgress status={account.upload} cancel={cancelUpload} />
          ) : (
            account.upload.phase === "pending" && (
              <UploadRecovery
                recover={() => void submitUpload({ kind: "recoverUpload" })}
              />
            )
          ))}
      </>
    ),
    foot: (
      <footer className="examFooter">
        {partner}
        <Menu position="top-start">
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
            <Menu.Item
              className="examDanger"
              disabled={uploading.includes(account.upload.phase)}
              onClick={async () => {
                if (
                  (await confirmAction(
                    t("examShare.deleteAccountBody"),
                    t("examShare.deleteAccount"),
                    { title: t("examShare.deleteAccountTitle") },
                  )) &&
                  (await accountAction("deleteAccount"))
                )
                  notify.info(t("examShare.accountDeleted"));
              }}
            >
              {t("examShare.deleteAccount")}
            </Menu.Item>
          </Menu.Dropdown>
        </Menu>
      </footer>
    ),
  };
  return (
    <section className="page pastExams" data-active={active || undefined}>
      {!connected && (
        <TitleRow nav className="pastExamsTitle">
          <h1>{t("examShare.title")}</h1>
          <span className="titleRowSpacer" />
          {partner}
          {siteLink}
        </TitleRow>
      )}
      <ExamChrome.Provider value={connected ? chrome || null : null}>
        {!connected ? (
          <AccountGate
            account={account}
            site={site}
            pending={
              pending === "disconnect" || pending === "deleteAccount"
                ? undefined
                : pending
            }
            run={(action) => void accountAction(action)}
          />
        ) : tab === "browse" ? (
          <BrowseList
            {...shared}
            active={active}
            setViewUrl={setViewUrl}
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
      </ExamChrome.Provider>
      {upload && connected && (
        <UploadDialog
          form={upload.form}
          choices={upload.choices}
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
