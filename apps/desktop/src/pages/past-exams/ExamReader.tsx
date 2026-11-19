import { lazy, Suspense, useEffect, useState, type ReactNode } from "react";
import {
  Bookmark,
  BookmarkCheck,
  Download,
  Flag,
  SquareArrowOutUpRight,
} from "lucide-react";
import { desktopBridge } from "../../app/native";
import { notify } from "../../app/notify";
import { notifyDownload } from "../../components/DownloadNotice";
import { ErrorBanner, Skeleton, Tag } from "../../components/ui";
import { localFileUrl } from "../../features/lms/files";
import { normalizeMathDelimiters } from "../../features/lms/markdown-math";
import { ImageView } from "../../features/lms/ImageView";
import { ReaderPane, ReaderTools } from "../../features/lms/ReaderPane";
import type {
  ExamAction,
  ExamAnswer,
  ExamChoices,
  ExamLocalDocument,
} from "../../generated/persistence";
import { t } from "../../i18n";
import { ReportDialog } from "./ExamDialogs";

const MarkdownDocument = lazy(() =>
  import("../../features/lms/MarkdownDocument").then((m) => ({
    default: m.MarkdownDocument,
  })),
);
/** The site stores Markdown as text/plain; the uploaded name tells it apart. */
const isMarkdown = (fileName: string) => /\.(md|markdown)$/i.test(fileName);

const Pdf = lazy(() =>
  import("../../features/lms/PdfDocument").then((m) => ({
    default: m.PdfDocument,
  })),
);

/** The opened document in the shared LMS reader frame, with its own tools.
 * Every type Core accepts shows here; the system application is one of
 * the tools. */
export function ExamReader({
  file,
  title,
  choices,
  call,
  setFile,
  onLibraryChanged,
  close,
}: {
  file: ExamLocalDocument;
  title: string;
  choices?: ExamChoices;
  call: (action: ExamAction) => Promise<ExamAnswer | undefined>;
  setFile: (update: (current: ExamLocalDocument) => ExamLocalDocument) => void;
  onLibraryChanged: () => void;
  close: () => void;
}) {
  const [reporting, setReporting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [downloading, setDownloading] = useState(false);
  const { document } = file;
  const pdf = document.fileType === "application/pdf";
  const openInApp = async () => {
    try {
      if (!desktopBridge?.examOpenFile) throw new Error(t("error.unavailable"));
      await desktopBridge.examOpenFile(file.path);
    } catch (error) {
      notify.error(error);
    }
  };
  // Keeps the opened version for offline use, then saves a copy to
  // Downloads under the name Core gives it.
  const download = async () => {
    setDownloading(true);
    try {
      if (!file.downloaded && !file.offline) {
        if (!desktopBridge?.examRetainFile)
          throw new Error(t("error.unavailable"));
        const changed = await desktopBridge.examRetainFile(
          document.id,
          file.path,
        );
        setFile((current) =>
          current.path === file.path
            ? { ...current, downloaded: true }
            : current,
        );
        if (changed) onLibraryChanged();
      }
      if (!desktopBridge?.examSaveCopy) throw new Error(t("error.unavailable"));
      // This save belongs to the click, even if its reader has since closed.
      notifyDownload(await desktopBridge.examSaveCopy(document.id, file.path));
    } catch (error) {
      notify.error(error);
    } finally {
      setDownloading(false);
    }
  };
  const save = async () => {
    setSaving(true);
    const answer = await call({
      kind: "save",
      saved: !file.saved,
      id: document.id,
    });
    setSaving(false);
    if (answer?.kind === "saved") {
      setFile((current) => ({ ...current, saved: answer.value }));
      if (answer.value !== file.saved) onLibraryChanged();
    }
  };
  // The LMS reader's own toolbar shape, so these sit in one row with the
  // PDF page and zoom controls (workspace-chrome.css, .lmsReaderToolSlot).
  const tool = (
    label: string,
    icon: ReactNode,
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
          t("common.openInDefaultApp"),
          <SquareArrowOutUpRight size={15} strokeWidth={1.75} aria-hidden />,
          () => void openInApp(),
        )}
        {tool(
          t("examShare.download"),
          <Download size={15} strokeWidth={1.75} aria-hidden />,
          () => void download(),
          { disabled: downloading },
        )}
        {file.saved !== null &&
          tool(
            t(file.saved ? "examShare.unsave" : "examShare.save"),
            file.saved ? (
              <BookmarkCheck size={15} strokeWidth={1.75} aria-hidden />
            ) : (
              <Bookmark size={15} strokeWidth={1.75} aria-hidden />
            ),
            () => void save(),
            { disabled: file.offline || saving, pressed: file.saved },
          )}
        {tool(
          t("examShare.report"),
          <Flag size={15} strokeWidth={1.75} aria-hidden />,
          () => setReporting(true),
          { disabled: file.offline || !choices },
        )}
      </div>
    </div>
  );
  return (
    <ReaderPane title={title} close={close}>
      {!pdf && <ReaderTools>{toolbar}</ReaderTools>}
      {pdf ? (
        <Suspense fallback={<Skeleton count={6} />}>
          <Pdf
            src={localFileUrl(file.path)}
            title={title}
            fileToolbar={toolbar}
          />
        </Suspense>
      ) : document.fileType === "text/plain" ? (
        <PlainTextPreview
          key={file.path}
          path={file.path}
          markdown={isMarkdown(document.fileName) || isMarkdown(file.path)}
        />
      ) : (
        <ImageView src={localFileUrl(file.path)} title={title} />
      )}
      {choices && (
        <ReportDialog
          opened={reporting}
          choices={choices}
          close={() => setReporting(false)}
          submit={async (reason, detail) => {
            if (
              await call({ kind: "report", id: document.id, reason, detail })
            ) {
              setReporting(false);
              notify.info(t("examShare.reported"));
            }
          }}
        />
      )}
    </ReaderPane>
  );
}

/** A text document read whole; Markdown is shown formatted and read-only,
 * its formulas set with KaTeX. */
function PlainTextPreview({
  path,
  markdown,
}: {
  path: string;
  markdown: boolean;
}) {
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
  if (failed) return <ErrorBanner>{t("error.storage")}</ErrorBanner>;
  if (text === undefined) return <Skeleton count={6} />;
  return markdown ? (
    <Suspense fallback={<Skeleton count={6} />}>
      <MarkdownDocument value={normalizeMathDelimiters(text)} />
    </Suspense>
  ) : (
    <pre className="examDocumentText">{text}</pre>
  );
}
