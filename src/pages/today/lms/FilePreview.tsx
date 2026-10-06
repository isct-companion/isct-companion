import { lazy, Suspense, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { desktopBridge } from "../../../app/native";
import { ExternalLink } from "../../../components/ExternalLink";
import { t } from "../../../i18n";
import { onSignedIn } from "./thumbnails";
import { extension, fileFamily } from "./file-types";
import { FileTile, LmsIcon } from "./LmsIcon";
import {
  fileSrc,
  openMaterial,
  openStoredFile,
  previewKind,
  type CourseFolder,
  type LocalFile,
} from "./files";
const TextDocument = lazy(() => import("./TextDocument").then(m => ({ default: m.TextDocument })));
const PdfDocument = lazy(() => import("./PdfDocument").then(m => ({ default: m.PdfDocument })));
const MarkdownDocument = lazy(() => import("./MarkdownDocument").then(m => ({ default: m.MarkdownDocument })));

const isAttachment = (url: string) => new URL(url).pathname.includes("/pluginfile.php/");
const megabytes = (bytes: number) => `${Math.max(0.1, bytes / 1024 / 1024).toFixed(bytes < 10 * 1024 * 1024 ? 1 : 0)} MB`;
const revealLabel = /Mac/i.test(navigator.platform) ? "Show in Finder" : "Show in folder";

type View =
  | { state: "loading" }
  | { state: "missing" }
  | { state: "ready"; file: LocalFile }
  | { state: "tooLarge"; bytes: number }
  | { state: "failed"; error: string };
type CourseFile = ReturnType<typeof useCourseFile>;

/**
 * A course file's state on disk. With `autoDownload`, a missing file within
 * `limit` bytes is downloaded at once; `fetchNow` downloads at any size.
 */
function useCourseFile(url: string, context: CourseFolder, limit: number, autoDownload: boolean, readText = false) {
  const managed = isAttachment(url) && Boolean(desktopBridge);
  const [view, setView] = useState<View>({ state: "loading" });
  const [request, setRequest] = useState({ download: autoDownload, limit: limit as number | undefined });
  useEffect(() => {
    if (!managed) { setView({ state: "missing" }); return; }
    let disposed = false;
    setView({ state: "loading" });
    openMaterial(context, url, { ...request, readText })
      .then(material => { if (!disposed) setView(material); })
      .catch(error => { if (!disposed) setView({ state: "failed", error: String(error) }); });
    return () => { disposed = true; };
  }, [url, request, managed]);
  // A download that failed for want of a session runs again after sign-in.
  useEffect(() => view.state === "failed" ? onSignedIn(() => setRequest(value => ({ ...value }))) : undefined, [view.state]);
  return {
    url, limit, managed, view, setView,
    file: view.state === "ready" ? view.file : undefined,
    retry: () => setRequest(value => ({ ...value })),
    fetchNow: () => setRequest({ download: true, limit: undefined }),
  };
}

/** Previews what the app can show; anything else gets the file card. */
export function FilePreview({
  url,
  title,
  context,
  limit,
  openInApp,
}: {
  url: string;
  title: string;
  context: CourseFolder;
  limit: number;
  openInApp?: () => void;
}) {
  const kind = previewKind(url);
  const course = useCourseFile(url, context, limit, Boolean(kind), kind === "text" || kind === "markdown");
  const { view, file } = course;
  // Development in a plain browser previews the LMS URL directly.
  const src = file ? fileSrc(file) : !desktopBridge && kind ? url : undefined;
  const textTooLarge = (kind === "text" || kind === "markdown") && file && file.text == null;
  const actions = <FileActions course={course} openInApp={openInApp} />;
  if (kind && view.state === "loading") {
    return <div className="lmsAttachmentPreview"><Toolbar title={title} url={url}>{actions}</Toolbar><p className="lmsQuiet lmsLoading">{t("Loading preview…")}</p></div>;
  }
  if (!kind || !src || textTooLarge) {
    return (
      <div className="lmsFileCardStage">
        <FileCard course={course} title={title} openInApp={openInApp} previewable={Boolean(kind) && !textTooLarge}
          note={textTooLarge ? t("This text file exceeds the 1 MB preview limit.") : undefined} />
      </div>
    );
  }
  const toolbar = <Toolbar title={title} url={url}>{actions}</Toolbar>;
  if (kind === "pdf")
    return <Suspense fallback={<p className="lmsQuiet lmsLoading">{t("Loading preview…")}</p>}><PdfDocument src={src} title={title} fileToolbar={toolbar} /></Suspense>;
  return (
    <div className="lmsAttachmentPreview">
      {toolbar}
      {kind === "image" ? <ImageView src={src} title={title} onBroken={() => course.setView({ state: "failed", error: t("This image can't be displayed here.") })} />
        : kind === "video" ? <div className="lmsImageStage"><video className="lmsMediaPreview" src={src} controls preload="metadata" /></div>
        : kind === "audio" ? <div className="lmsAudioStage"><audio src={src} controls preload="metadata" /></div>
        : kind === "markdown" ? <Suspense fallback={<p className="lmsQuiet lmsLoading">{t("Loading…")}</p>}><MarkdownDocument value={file?.text ?? ""} /></Suspense>
        : <Suspense fallback={<p className="lmsQuiet lmsLoading">{t("Loading preview…")}</p>}><TextDocument value={file?.text ?? ""} url={url} /></Suspense>}
    </div>
  );
}

/** Secondary actions: Show in Finder (once on disk), Open in app, Open in browser. */
function FileActions({ course, openInApp }: { course: CourseFile; openInApp?: () => void }) {
  const { file, url } = course;
  return <>
    {file && <button type="button" onClick={() => void openStoredFile(file.path, true)}><LmsIcon name="folder" size={14} />{t(revealLabel)}</button>}
    {openInApp && <button type="button" onClick={openInApp}><LmsIcon name="page" size={14} />{t("Open in app")}</button>}
    <ExternalLink href={url}><LmsIcon name="external" size={14} />{t("Open in browser")}</ExternalLink>
  </>;
}

/** Name, size, state and one primary action for the file's current state. */
function FileCard({ course, title, openInApp, previewable, note: override }: { course: CourseFile; title: string; openInApp?: () => void; previewable: boolean; note?: string }) {
  const { url, view, file, managed, limit } = course;
  const attachment = isAttachment(url);
  const size = file ? megabytes(file.bytes) : view.state === "tooLarge" ? megabytes(view.bytes) : "";
  const primary =
    !managed ? null
    : view.state === "failed" ? <button type="button" onClick={course.retry}>{t("Retry")}</button>
    : view.state === "tooLarge" ? <button type="button" onClick={course.fetchNow}><LmsIcon name="download" size={15} />{previewable ? t("Preview") : t("Download")} · {size}</button>
    : file ? <button type="button" onClick={() => void openStoredFile(file.path)}><LmsIcon name="external" size={15} />{t("Open")}</button>
    : <button type="button" disabled={view.state === "loading"} onClick={course.fetchNow}><LmsIcon name="download" size={15} />{t(view.state === "loading" ? "Downloading…" : "Download")}</button>;
  const note = override ??
    (!attachment ? t("This page opens outside the app.")
    : view.state === "failed" ? view.error
    : view.state === "tooLarge" ? t("Larger than the automatic download limit ({{size}}).", { size: megabytes(limit) })
    : file ? t("Saved in your course folder.")
    : t("This file type can't be previewed here."));
  return (
    <div className="lmsFileCard">
      <FileTile family={attachment ? fileFamily(url) : "link"} />
      <div className="lmsFileCardText">
        <h3>{title}</h3>
        <small>{[extension(url).toUpperCase(), size].filter(Boolean).join(" · ")}</small>
      </div>
      <p role={view.state === "failed" ? "alert" : undefined}>{note}</p>
      <div className="lmsFileCardActions">
        {primary && <span className="lmsPrimaryAction">{primary}</span>}
        <FileActions course={course} openInApp={openInApp} />
      </div>
    </div>
  );
}

/**
 * Files the app cannot preview stay in the outline: their card opens as a
 * popover at the clicked item instead of taking over the reading pane. It
 * follows the item while content above it loads or the outline scrolls.
 */
export function FilePopover({
  url,
  title,
  context,
  limit,
  openInApp,
  anchor,
  onClose,
}: {
  url: string;
  title: string;
  context: CourseFolder;
  limit: number;
  openInApp?: () => void;
  anchor: Element;
  onClose: () => void;
}) {
  const course = useCourseFile(url, context, limit, false);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const outside = (event: MouseEvent) => { if (!box.current?.contains(event.target as Node)) onClose(); };
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") onClose(); };
    document.addEventListener("mousedown", outside);
    document.addEventListener("keydown", escape);
    return () => { document.removeEventListener("mousedown", outside); document.removeEventListener("keydown", escape); };
  }, [onClose]);
  useLayoutEffect(() => {
    const popover = box.current;
    const pane = popover?.offsetParent as HTMLElement | null;
    if (!popover || !pane) return;
    // Below the item, or above it when it sits low in the visible outline.
    const above = anchor.getBoundingClientRect().bottom - pane.getBoundingClientRect().top > pane.clientHeight * 0.6;
    let frame = 0;
    const place = () => {
      if (!anchor.isConnected) return onClose();
      const area = pane.getBoundingClientRect();
      const item = anchor.getBoundingClientRect();
      const top = (above ? item.top - popover.offsetHeight - 6 : item.bottom + 6) - area.top + pane.scrollTop;
      popover.style.top = `${top}px`;
      popover.style.left = `${Math.max(8, Math.min(item.left - area.left, pane.clientWidth - popover.offsetWidth - 8))}px`;
      frame = requestAnimationFrame(place);
    };
    place();
    return () => cancelAnimationFrame(frame);
  }, [anchor, onClose]);
  return (
    <div ref={box} className="lmsFilePopover" role="dialog" aria-label={title}>
      <FileCard course={course} title={title} openInApp={openInApp} previewable={false} />
    </div>
  );
}

function Toolbar({ title, url, children }: { title: string; url: string; children: ReactNode }) {
  return (
    <div className="lmsSourceBar lmsFileToolbar">
      <div className="lmsFileAddress" title={url}><span>{title}</span></div>
      {children}
    </div>
  );
}

/** Fit to the pane; click toggles actual size for large scans. */
function ImageView({ src, title, onBroken }: { src: string; title: string; onBroken: () => void }) {
  const [actual, setActual] = useState(false);
  return (
    <div className={`lmsImageStage${actual ? " actual" : ""}`}>
      <img className="lmsImagePreview" src={src} alt={title} title={t(actual ? "Fit to window" : "Actual size")}
        onClick={() => setActual(value => !value)} onError={onBroken} />
    </div>
  );
}
