import { ReaderTools } from "./ReaderPane";
import {
  lazy,
  Suspense,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { desktopBridge } from "../../app/native";
import { nativeOs } from "../../platform/desktop-bridge";
import { ExternalLink } from "../../components/ExternalLink";
import { t } from "../../i18n";
import { extension, fileFamily } from "./file-types";
import { FileTile, LmsIcon } from "./LmsIcon";
import {
  LmsFileError,
  localFileUrl,
  openMaterial,
  openStoredFile,
  previewKind,
  type CourseFolder,
  type LmsFileReason,
} from "./files";
const TextDocument = lazy(() =>
  import("./TextDocument").then((m) => ({ default: m.TextDocument })),
);
const PdfDocument = lazy(() =>
  import("./PdfDocument").then((m) => ({ default: m.PdfDocument })),
);
const MarkdownDocument = lazy(() =>
  import("./MarkdownDocument").then((m) => ({ default: m.MarkdownDocument })),
);

const isLmsResource = (url: string) => {
  try {
    const parsed = new URL(url);
    const parts = parsed.pathname.split("/").filter(Boolean);
    return (
      parsed.protocol === "https:" &&
      parsed.hostname === "lms.s.isct.ac.jp" &&
      !parsed.port &&
      (parts.includes("pluginfile.php") ||
        /\/mod\/(resource|url|folder)\/view\.php$/.test(parsed.pathname))
    );
  } catch {
    return false;
  }
};
const megabytes = (bytes: number) =>
  `${Math.max(0.1, bytes / 1024 / 1024).toFixed(bytes < 10 * 1024 * 1024 ? 1 : 0)} MB`;
const revealLabel =
  nativeOs() === "macos" ? "Show in Finder" : "Show in folder";
const deniedNote =
  nativeOs() === "windows"
    ? "Windows blocked saving to the course folder. If Controlled folder access is on, allow IsctCompanion in Windows Security, or choose another course folder in Settings."
    : "The system blocked saving to the course folder. Check its permissions, or choose another course folder in Settings.";

type View =
  | { state: "loading" }
  | { state: "missing" }
  | {
      state: "ready";
      src: string;
      kind?: ReturnType<typeof previewKind>;
      text?: string;
    }
  | { state: "folder"; files: { name: string; url: string }[] }
  | { state: "external"; url: string }
  | { state: "tooLarge"; bytes: number }
  | { state: "failed"; reason: LmsFileReason; error: string };
type CourseFile = ReturnType<typeof useCourseFile>;

/**
 * A course file's state on disk. With `autoDownload`, a missing file within
 * `limit` bytes is downloaded at once; `fetchNow` downloads at any size.
 */
function useCourseFile(
  url: string,
  context: CourseFolder,
  limit: number,
  autoDownload: boolean,
  readText = false,
) {
  const managed = isLmsResource(url) && Boolean(desktopBridge);
  const [view, setView] = useState<View>({ state: "loading" });
  const [request, setRequest] = useState({ download: false, attempt: 0 });
  const loadedRequest = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (!managed) {
      setView(
        autoDownload
          ? { state: "ready", src: url, kind: previewKind(url) }
          : { state: "missing" },
      );
      return;
    }
    if (!autoDownload && !request.download) {
      setView({ state: "missing" });
      return;
    }
    const requestKey = JSON.stringify([
      url,
      context.term,
      context.course,
      request,
      managed,
      autoDownload,
      readText,
    ]);
    if (loadedRequest.current === requestKey) return;
    let disposed = false;
    setView({ state: "loading" });
    const load = async () => {
      try {
        const material = await openMaterial(context, url, {
          download: true,
          readText,
          limit: request.download ? undefined : limit,
        });
        if (disposed) return;
        if (material.state !== "failed") loadedRequest.current = requestKey;
        if (material.state === "folder")
          return setView({ state: "folder", files: material.files });
        if (material.state === "external")
          return setView({ state: "external", url: material.url });
        if (material.state === "tooLarge")
          return setView({ state: "tooLarge", bytes: material.bytes });
        if (material.state === "missing") return setView({ state: "missing" });
        if (material.state === "failed")
          throw new LmsFileError(material.reason);
        setView({
          state: "ready",
          src: localFileUrl(material.file.path),
          kind: previewKind(material.file.name || url),
          text: material.file.text ?? undefined,
        });
      } catch (error) {
        if (disposed) return;
        if (error instanceof LmsFileError && error.reason === "tooLarge")
          setView({ state: "tooLarge", bytes: error.bytes ?? 0 });
        else
          setView({
            state: "failed",
            reason: error instanceof LmsFileError ? error.reason : "network",
            error: String(error),
          });
      }
    };
    void load();
    return () => {
      disposed = true;
    };
  }, [
    url,
    context.term,
    context.course,
    request,
    managed,
    autoDownload,
    readText,
  ]);
  return {
    url,
    context,
    limit,
    managed,
    view,
    setView,
    retry: () =>
      setRequest((value) => ({ ...value, attempt: value.attempt + 1 })),
    fetchNow: () =>
      setRequest((value) => ({ download: true, attempt: value.attempt + 1 })),
  };
}

/** Previews what the app can show; anything else gets the file card. */
export function FilePreview({
  url,
  title,
  context,
  limit,
  openInApp,
  openFile,
}: {
  url: string;
  title: string;
  context: CourseFolder;
  limit: number;
  openInApp?: () => void;
  /** Opens one file of a folder in the reader. */
  openFile?: (url: string, name: string) => void;
}) {
  const expected = previewKind(url);
  const lazyResource = /\/mod\/(resource|url|folder)\/view\.php/.test(url);
  const course = useCourseFile(
    url,
    context,
    limit,
    Boolean(expected) || lazyResource,
    true,
  );
  const { view } = course;
  const ready = view.state === "ready" ? view : undefined;
  const kind = ready?.kind ?? expected;
  const src = ready?.src;
  const textTooLarge =
    (kind === "text" || kind === "markdown") && ready && ready.text == null;
  const actions = <FileActions course={course} openInApp={openInApp} />;
  if ((kind || lazyResource) && view.state === "loading") {
    return (
      <div className="lmsAttachmentPreview">
        <Toolbar title={title} url={url}>
          {actions}
        </Toolbar>
        <p className="lmsQuiet lmsLoading">{t("Loading preview…")}</p>
      </div>
    );
  }
  if (view.state === "folder") {
    // Names carry the extension; folder URLs may not.
    const byName = (name: string) =>
      `https://file.invalid/${encodeURIComponent(name)}`;
    return (
      <div className="lmsFolderView">
        <header className="lmsFolderHeader">
          <FileTile family="folder" />
          <div>
            <h3>{title}</h3>
            <small>{t("{{count}} files", { count: view.files.length })}</small>
          </div>
        </header>
        <div className="lmsFolderList">
          {view.files.map((file) => {
            const ext = extension(byName(file.name));
            const body = (
              <>
                <FileTile family={fileFamily(byName(file.name))} />
                <span>
                  <b>{file.name}</b>
                  {ext && <small>{ext.toUpperCase()}</small>}
                </span>
                <LmsIcon name="chevron" className="lmsRowAction" />
              </>
            );
            return openFile ? (
              <button
                type="button"
                className="lmsItemRow"
                key={file.url}
                onClick={() => openFile(file.url, file.name)}
              >
                {body}
              </button>
            ) : (
              <a className="lmsItemRow" key={file.url} href={file.url} download>
                {body}
              </a>
            );
          })}
          {!view.files.length && (
            <p className="lmsQuiet">{t("This folder is empty.")}</p>
          )}
        </div>
      </div>
    );
  }
  if (view.state === "external") {
    return (
      <div className="lmsFileCardStage">
        <div className="lmsFileCard">
          <FileTile family="link" />
          <div className="lmsFileCardText">
            <h3>{title}</h3>
          </div>
          <ExternalLink href={view.url}>{t("Open in browser")}</ExternalLink>
        </div>
      </div>
    );
  }
  if (!kind || !src || textTooLarge) {
    return (
      <div className="lmsFileCardStage">
        <FileCard
          course={course}
          title={title}
          openInApp={openInApp}
          previewable={Boolean(kind) && !textTooLarge}
          note={
            textTooLarge
              ? t("This text file exceeds the 1 MB preview limit.")
              : undefined
          }
        />
      </div>
    );
  }
  const toolbar = (
    <Toolbar title={title} url={url} inline={kind === "pdf"}>
      {actions}
    </Toolbar>
  );
  if (kind === "pdf")
    return (
      <Suspense
        fallback={
          <p className="lmsQuiet lmsLoading">{t("Loading preview…")}</p>
        }
      >
        <PdfDocument src={src} title={title} fileToolbar={toolbar} />
      </Suspense>
    );
  return (
    <div className="lmsAttachmentPreview">
      {toolbar}
      {kind === "image" ? (
        <ImageView
          src={src}
          title={title}
          onBroken={() =>
            course.setView({
              state: "failed",
              reason: "network",
              error: t("This image can't be displayed here."),
            })
          }
        />
      ) : kind === "video" ? (
        <div className="lmsImageStage">
          <MediaView kind="video" src={src} />
        </div>
      ) : kind === "audio" ? (
        <div className="lmsAudioStage">
          <MediaView kind="audio" src={src} />
        </div>
      ) : kind === "markdown" ? (
        <Suspense
          fallback={<p className="lmsQuiet lmsLoading">{t("Loading…")}</p>}
        >
          <MarkdownDocument value={ready?.text ?? ""} />
        </Suspense>
      ) : (
        <Suspense
          fallback={
            <p className="lmsQuiet lmsLoading">{t("Loading preview…")}</p>
          }
        >
          <TextDocument value={ready?.text ?? ""} url={url} />
        </Suspense>
      )}
    </div>
  );
}

/** Secondary actions: Show in Finder (once on disk), Open in LMS, Open in browser. */
function FileActions({
  course,
  openInApp,
}: {
  course: CourseFile;
  openInApp?: () => void;
}) {
  const { url, context, view } = course;
  const reveal = async (open = false) => {
    const material = await openMaterial(context, url, { download: true });
    if (material.state === "ready")
      await openStoredFile(material.file.path, !open);
  };
  // In the reader panel these collapse to icons; the label stays as tooltip and name.
  const label = (text: string) => ({ "aria-label": t(text), title: t(text) });
  return (
    <div className="lmsFileActions">
      {view.state === "ready" && course.managed && (
        <button
          type="button"
          {...label(revealLabel)}
          onClick={() => void reveal()}
        >
          <LmsIcon name="folder" size={15} />
          <span className="lmsToolLabel">{t(revealLabel)}</span>
        </button>
      )}
      {openInApp && (
        <button type="button" {...label("Open in LMS")} onClick={openInApp}>
          <LmsIcon name="page" size={15} />
          <span className="lmsToolLabel">{t("Open in LMS")}</span>
        </button>
      )}
      <ExternalLink href={url} {...label("Open in browser")}>
        <LmsIcon name="external" size={15} />
        <span className="lmsToolLabel">{t("Open in browser")}</span>
      </ExternalLink>
    </div>
  );
}

/** Name, size, state and one primary action for the file's current state. */
function FileCard({
  course,
  title,
  openInApp,
  previewable,
  note: override,
}: {
  course: CourseFile;
  title: string;
  openInApp?: () => void;
  previewable: boolean;
  note?: string;
}) {
  const { url, view, managed, limit } = course;
  const attachment = isLmsResource(url);
  const size = view.state === "tooLarge" ? megabytes(view.bytes) : "";
  const primary = !managed ? null : view.state === "failed" &&
    view.reason === "login" &&
    openInApp ? (
    <button type="button" onClick={openInApp}>
      {t("Sign in to LMS")}
    </button>
  ) : view.state === "failed" &&
    ["unresolved", "network", "timeout"].includes(view.reason) &&
    openInApp ? (
    <button type="button" onClick={openInApp}>
      {t("Open in LMS")}
    </button>
  ) : view.state === "failed" ? (
    <button type="button" onClick={course.retry}>
      {t("Retry")}
    </button>
  ) : view.state === "tooLarge" ? (
    <button type="button" onClick={course.fetchNow}>
      <LmsIcon name="download" size={15} />
      {previewable ? t("Preview") : t("Download")} · {size}
    </button>
  ) : view.state === "ready" ? null : (
    <button
      type="button"
      disabled={view.state === "loading"}
      onClick={course.fetchNow}
    >
      <LmsIcon name="download" size={15} />
      {t(view.state === "loading" ? "Downloading…" : "Download")}
    </button>
  );
  const note =
    override ??
    (!attachment
      ? t("This page opens outside the app.")
      : view.state === "failed"
        ? view.reason === "denied"
          ? t(deniedNote)
          : view.error
        : view.state === "tooLarge"
          ? t("Larger than the automatic download limit ({{size}}).", {
              size: megabytes(limit),
            })
          : view.state === "ready"
            ? t("Saved in your course folder.")
            : t("This file type can't be previewed here."));
  return (
    <div className="lmsFileCard">
      <FileTile family={attachment ? fileFamily(url) : "link"} />
      <div className="lmsFileCardText">
        <h3>{title}</h3>
        <small>
          {[extension(url).toUpperCase(), size].filter(Boolean).join(" · ")}
        </small>
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
    const outside = (event: MouseEvent) => {
      if (!box.current?.contains(event.target as Node)) onClose();
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", outside);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("mousedown", outside);
      document.removeEventListener("keydown", escape);
    };
  }, [onClose]);
  useLayoutEffect(() => {
    const popover = box.current;
    const pane = popover?.offsetParent as HTMLElement | null;
    if (!popover || !pane) return;
    // Below the item, or above it when it sits low in the visible outline.
    const above =
      anchor.getBoundingClientRect().bottom - pane.getBoundingClientRect().top >
      pane.clientHeight * 0.6;
    let frame = 0;
    const place = () => {
      if (!anchor.isConnected) return onClose();
      const area = pane.getBoundingClientRect();
      const item = anchor.getBoundingClientRect();
      const top =
        (above ? item.top - popover.offsetHeight - 6 : item.bottom + 6) -
        area.top +
        pane.scrollTop;
      popover.style.top = `${top}px`;
      popover.style.left = `${Math.max(8, Math.min(item.left - area.left, pane.clientWidth - popover.offsetWidth - 8))}px`;
      frame = requestAnimationFrame(place);
    };
    place();
    return () => cancelAnimationFrame(frame);
  }, [anchor, onClose]);
  return (
    <div ref={box} className="lmsFilePopover" role="dialog" aria-label={title}>
      <FileCard
        course={course}
        title={title}
        openInApp={openInApp}
        previewable={false}
      />
    </div>
  );
}

function Toolbar({
  title,
  url,
  children,
  inline = false,
}: {
  title: string;
  url: string;
  children: ReactNode;
  inline?: boolean;
}) {
  const tools = (
    <div className="lmsSourceBar lmsFileToolbar">
      <div className="lmsFileAddress" title={url}>
        <span>{title}</span>
      </div>
      {children}
    </div>
  );
  return inline ? tools : <ReaderTools>{tools}</ReaderTools>;
}

/**
 * WebKit keeps a detached element's decoded image or media player until its
 * garbage collector runs, which can be long after the viewer closed. Dropping
 * the source on unmount releases them at once, like the PDF canvases.
 */
function useReleaseSource<T extends HTMLImageElement | HTMLMediaElement>() {
  const element = useRef<T>(null);
  useEffect(() => {
    const current = element.current;
    return () => {
      if (!current) return;
      if (current instanceof HTMLMediaElement) current.pause();
      current.removeAttribute("src");
      if (current instanceof HTMLMediaElement) current.load();
    };
  }, []);
  return element;
}

/** Fit to the pane; click toggles actual size for large scans. */
function ImageView({
  src,
  title,
  onBroken,
}: {
  src: string;
  title: string;
  onBroken: () => void;
}) {
  const [actual, setActual] = useState(false);
  const image = useReleaseSource<HTMLImageElement>();
  return (
    <div className={`lmsImageStage${actual ? " actual" : ""}`}>
      <img
        ref={image}
        className="lmsImagePreview"
        src={src}
        alt={title}
        title={t(actual ? "Fit to window" : "Actual size")}
        onClick={() => setActual((value) => !value)}
        onError={onBroken}
      />
    </div>
  );
}

function MediaView({ kind, src }: { kind: "video" | "audio"; src: string }) {
  const media = useReleaseSource<HTMLMediaElement>();
  return kind === "video" ? (
    <video
      ref={media as React.RefObject<HTMLVideoElement>}
      className="lmsMediaPreview"
      src={src}
      controls
      preload="metadata"
    />
  ) : (
    <audio
      ref={media as React.RefObject<HTMLAudioElement>}
      src={src}
      controls
      preload="metadata"
    />
  );
}

/**
 * A folder's files listed in the outline, under the folder's own row. Each
 * file opens the way an outline item does: previews in the reader, anything
 * else as the popover card at the clicked file.
 */
type FolderFile = { name: string; url: string };
/** Folder listings resolved in this session, so reopening a folder is instant. */
const folderListings = new Map<string, FolderFile[]>();

export function InlineFolder({
  url,
  context,
  limit,
  known,
  openFile,
  openInApp,
}: {
  url: string;
  context: CourseFolder;
  limit: number;
  /** Files already stored with the folder; resolution runs only without them. */
  known?: FolderFile[];
  openFile: (url: string, name: string, anchor: Element) => void;
  openInApp?: () => void;
}) {
  const listed = folderListings.get(url) ?? known;
  const { view: resolved } = useCourseFile(url, context, limit, !listed);
  useEffect(() => {
    if (resolved.state === "folder") folderListings.set(url, resolved.files);
  }, [resolved, url]);
  const view: View = listed ? { state: "folder", files: listed } : resolved;
  // Names carry the extension; folder URLs may not.
  const byName = (name: string) =>
    `https://file.invalid/${encodeURIComponent(name)}`;
  if (view.state === "loading")
    return <p className="lmsQuiet lmsFolderNote">{t("Loading…")}</p>;
  if (view.state !== "folder")
    return (
      <p className="lmsQuiet lmsFolderNote">
        {t("This folder could not be read.")}{" "}
        {openInApp && (
          <button type="button" className="lmsTextLink" onClick={openInApp}>
            {t("Open in LMS")}
          </button>
        )}
      </p>
    );
  if (!view.files.length)
    return (
      <p className="lmsQuiet lmsFolderNote">{t("This folder is empty.")}</p>
    );
  return (
    <div className="lmsFolderFiles">
      {view.files.map((file) => {
        const ext = extension(byName(file.name));
        return (
          <button
            type="button"
            className="lmsFolderFile"
            key={file.url}
            onClick={(event) =>
              openFile(file.url, file.name, event.currentTarget)
            }
          >
            <FileTile family={fileFamily(byName(file.name))} />
            <span>
              <b>{file.name}</b>
              {ext && <small>{ext.toUpperCase()}</small>}
            </span>
          </button>
        );
      })}
    </div>
  );
}
