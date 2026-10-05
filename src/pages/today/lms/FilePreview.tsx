import { lazy, Suspense, useEffect, useState } from "react";
import { desktopBridge } from "../../../app/native";
import { schoolSessions } from "../../../platform/school-session";
import { ExternalLink } from "../../../components/ExternalLink";
import { t } from "../../../i18n";
import {
  downloadMaterial,
  fileSrc,
  openMaterial,
  previewKind,
  type CourseFolder,
  type LocalFile,
} from "./files";
const TextDocument = lazy(() => import("./TextDocument").then(m => ({ default: m.TextDocument })));
const PdfDocument = lazy(() => import("./PdfDocument").then(m => ({ default: m.PdfDocument })));
const MarkdownDocument = lazy(() =>
  import("./MarkdownDocument").then((m) => ({ default: m.MarkdownDocument })),
);
export function FilePreview({
  url,
  title,
  context,
  autoSave = false,
}: {
  url: string;
  title: string;
  context: CourseFolder;
  autoSave?: boolean;
}) {
  const kind = previewKind(url);
  const attachment = new URL(url).pathname.includes("/pluginfile.php/");
  const [file, setFile] = useState<LocalFile | null>(null);
  const [status, setStatus] = useState(
    desktopBridge
      ? "Loading…"
      : "Browser preview · offline cache requires the desktop app",
  );
  const [failed, setFailed] = useState(false);
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    if (!desktopBridge || !attachment) { if (!attachment) setStatus("External link"); return; }
    let disposed = false;
    setFile(null);
    setFailed(false);
    setStatus("Loading…");
    const readText = kind === "text" || kind === "markdown";
    void (async () => {
      const cached = await openMaterial(context, url, false, readText, autoSave);
      if (disposed) return;
      if (cached) {
        setFile(cached);
        setStatus(autoSave ? "Available offline" : "Preview ready");
        return;
      }
      setStatus("Downloading…");
      const downloaded = await openMaterial(context, url, true, readText, autoSave);
      if (!disposed) {
        setFile(downloaded);
        setStatus(autoSave ? "Saved for offline use" : "Preview ready");
      }
    })().catch((error) => {
      if (!disposed) {
        setStatus(String(error));
        setFailed(true);
      }
    });
    return () => {
      disposed = true;
    };
  }, [url, retry, autoSave]);
  useEffect(() => {
    if (!failed || !attachment || !/sign.in|Open LMS/i.test(status)) return;
    return schoolSessions.subscribe(() => {
      if (schoolSessions.get("science") === "active") setRetry(value => value + 1);
    });
  }, [failed, attachment, status]);
  const src = file ? fileSrc(file) : !desktopBridge ? url : undefined;
  const toolbar = <>
      <div className="lmsSourceBar lmsFileToolbar">
        <div className="lmsFileAddress" title={url}><span>{title}</span><small role="status" title={status}>{t(status)}</small></div>
        {failed && (
          <button type="button" onClick={() => setRetry((value) => value + 1)}>
            {t("Retry")}
          </button>
        )}
        {desktopBridge && attachment && <button type="button" onClick={() => { setStatus("Saving…"); void downloadMaterial(context, url).then(path => setStatus(path ? `${t("Saved locally")} · ${path}` : t("Save cancelled"))).catch(error => setStatus(String(error))); }}>{t("Download")}</button>}
        <ExternalLink href={url}>{t("Open original")} ↗</ExternalLink>
      </div>
    </>;
  return (
    <div className="lmsAttachmentPreview">
      {!(src && kind === "pdf") && toolbar}
      {failed && <p className="lmsFileError" role="alert">{t(status)}</p>}
      {src &&
        (kind === "pdf" ? (
          <Suspense fallback={<p>{t("Loading preview…")}</p>}>
            <PdfDocument src={src} title={title} fileToolbar={toolbar} />
          </Suspense>
        ) : kind === "image" ? (
          <img className="lmsImagePreview" src={src} alt={title} />
        ) : kind === "markdown" && file?.text != null ? (
          <Suspense fallback={<p>{t("Loading…")}</p>}>
            <MarkdownDocument value={file.text} />
          </Suspense>
        ) : kind === "text" && file?.text != null ? (
          <Suspense fallback={<p>{t("Loading preview…")}</p>}><TextDocument value={file.text} url={url} /></Suspense>
        ) : (
          <p className="lmsQuiet">
            {t(
              !attachment ? "Open this page in your browser." : file && kind
                ? "This text file exceeds the 1 MB preview limit. Download to open it."
                : "Download this file to open it.",
            )}
          </p>
        ))}
    </div>
  );
}
