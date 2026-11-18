import { useState } from "react";
import { Bookmark, BookmarkCheck, Download, Flag } from "lucide-react";
import { desktopBridge } from "../../app/native";
import { notify } from "../../app/notify";
import { notifyDownload } from "../../components/DownloadNotice";
import { Button, Tag } from "../../components/ui";
import { ReaderPane } from "../../features/lms/ReaderPane";
import type {
  ExamAction,
  ExamAnswer,
  ExamChoices,
  ExamLocalDocument,
} from "../../generated/persistence";
import { t } from "../../i18n";
import { ReportDialog } from "./ExamDialogs";

export async function openExamFile(file: ExamLocalDocument) {
  if (!desktopBridge?.examOpenFile) throw new Error(t("error.unavailable"));
  await desktopBridge.examOpenFile(file.path);
}

/** The app owns document actions; the system default application views the file. */
export function ExamDocumentPanel({
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
  const download = async () => {
    setDownloading(true);
    try {
      if (!file.downloaded && !file.offline) {
        if (!desktopBridge?.examRetainFile)
          throw new Error(t("error.unavailable"));
        const changed = await desktopBridge.examRetainFile(
          file.document.id,
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
      // This save belongs to the click, even if its panel has since closed.
      const saved = await desktopBridge.examSaveCopy(
        file.path,
        file.document.fileName,
      );
      notifyDownload(saved);
    } catch (error) {
      notify.error(error);
    } finally {
      setDownloading(false);
    }
  };
  return (
    <ReaderPane title={title} close={close}>
      <section className="examRequest">
        <header>
          <h2>{file.document.fileName}</h2>
          {file.offline && <Tag>{t("examShare.offline")}</Tag>}
        </header>
        <div className="examRequestActions">
          <Button onClick={() => void openExamFile(file).catch(notify.error)}>
            {t("common.openInDefaultApp")}
          </Button>
          <Button
            variant="subtle"
            loading={downloading}
            onClick={() => void download()}
          >
            <Download size={15} aria-hidden /> {t("examShare.download")}
          </Button>
          {file.saved !== null && (
            <Button
              variant="subtle"
              loading={saving}
              disabled={file.offline}
              aria-pressed={file.saved}
              onClick={async () => {
                setSaving(true);
                const answer = await call({
                  kind: "save",
                  saved: !file.saved,
                  id: file.document.id,
                });
                setSaving(false);
                if (answer?.kind === "saved") {
                  setFile((current) => ({ ...current, saved: answer.value }));
                  if (answer.value !== file.saved) onLibraryChanged();
                }
              }}
            >
              {file.saved ? (
                <BookmarkCheck size={15} aria-hidden />
              ) : (
                <Bookmark size={15} aria-hidden />
              )}
              {t(file.saved ? "examShare.unsave" : "examShare.save")}
            </Button>
          )}
          <Button
            variant="subtle"
            disabled={file.offline || !choices}
            onClick={() => setReporting(true)}
          >
            <Flag size={15} aria-hidden /> {t("examShare.report")}
          </Button>
        </div>
      </section>
      {choices && (
        <ReportDialog
          opened={reporting}
          choices={choices}
          close={() => setReporting(false)}
          submit={async (reason, detail) => {
            if (
              await call({
                kind: "report",
                id: file.document.id,
                reason,
                detail,
              })
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
