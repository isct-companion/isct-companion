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
import { notify } from "../../app/notify";
import { nativeOs, type SchoolSite } from "../../platform/desktop-bridge";
import { ExternalLink } from "../../components/ExternalLink";
import { t, type MessageKey } from "../../i18n";
import { downloadOnly, extension, fileFamily } from "./file-types";
import { FileTile, LmsIcon } from "./LmsIcon";
import {
  localFileUrl,
  copyStoredFile,
  openMaterial,
  openStoredFile,
  shareStoredFile,
  previewKind,
  type LmsFileReason,
  type SchoolPage,
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

const megabytes = (bytes: number) =>
  `${Math.max(0.1, bytes / 1024 / 1024).toFixed(bytes < 10 * 1024 * 1024 ? 1 : 0)} MB`;
const revealLabel: MessageKey =
  nativeOs() === "macos" ? "web.showInFinder" : "web.showInFolder";
// Only macOS and Windows have a system Copy and Share for a file (file_share.rs).
const systemShare = nativeOs() === "macos" || nativeOs() === "windows";
const deniedNote: MessageKey =
  nativeOs() === "windows"
    ? "lms.windowsBlockedSavingToThe"
    : "lms.theSystemBlockedSavingTo";

type View =
  | { state: "loading" }
  | { state: "missing"; page: SchoolPage }
  | {
      state: "ready";
      src: string;
      kind?: ReturnType<typeof previewKind>;
      text?: string;
      page: SchoolPage;
    }
  | { state: "folder"; files: { name: string; url: string }[] }
  | ({ state: "page" } & SchoolPage)
  | { state: "external"; url: string }
  | { state: "tooLarge"; bytes: number; page: SchoolPage }
  | {
      state: "failed";
      reason: LmsFileReason;
      page: SchoolPage | null;
      note?: MessageKey;
    };
type CourseFile = ReturnType<typeof useCourseFile>;
/** Shows a school page: the LMS in the reader, another site where it lives. */
type OpenPage = (page: SchoolPage) => void;

/** What a failure tells the person, in their language. */
const failureNote: Record<LmsFileReason, MessageKey> = {
  login: "lmsFile.signInToTheLMS",
  network: "lmsFile.theFileCouldNotBe",
  timeout: "lmsFile.theFileCouldNotBe",
  cancelled: "lmsFile.theFileCouldNotBe",
  denied: deniedNote,
  invalid: "lmsFile.thisCantBeShownHere",
  unresolved: "lmsFile.thisCantBeShownHere",
};

/** The LMS page Core named for the item, if any. */
const pageOf = (view: View): SchoolPage | null =>
  "page" in view ? view.page : null;

/**
 * What an item or link opens as, as Core decides (`open_material`), and a
 * file's state on disk. With `download`, a missing file within `limit` bytes
 * is downloaded at once; `fetchNow` downloads it at any size.
 */
function useCourseFile(
  url: string,
  courseId: string,
  limit: number,
  { download, readText = false }: { download: boolean; readText?: boolean },
) {
  const [view, setView] = useState<View>({ state: "loading" });
  const [request, setRequest] = useState({ download: false, attempt: 0 });
  const loadedRequest = useRef<string | undefined>(undefined);
  useEffect(() => {
    const requestKey = JSON.stringify([
      url,
      courseId,
      request,
      download,
      readText,
    ]);
    if (loadedRequest.current === requestKey) return;
    let disposed = false;
    setView({ state: "loading" });
    const load = async () => {
      try {
        const material = await openMaterial(courseId, url, {
          download: download || request.download,
          readText,
          limit: request.download ? undefined : limit,
        });
        if (disposed) return;
        if (material.state !== "failed") loadedRequest.current = requestKey;
        if (material.state === "ready")
          return setView({
            state: "ready",
            src: localFileUrl(material.file.path),
            kind: previewKind(material.file.name || url),
            text: material.file.text ?? undefined,
            page: material.page,
          });
        setView(material);
      } catch {
        // A command error is not the item's: Core refused the request.
        if (!disposed)
          setView({ state: "failed", reason: "invalid", page: null });
      }
    };
    void load();
    return () => {
      disposed = true;
    };
  }, [url, courseId, request, download, readText]);
  return {
    url,
    courseId,
    limit,
    view,
    setView,
    retry: () =>
      setRequest((value) => ({ ...value, attempt: value.attempt + 1 })),
    fetchNow: () =>
      setRequest((value) => ({ download: true, attempt: value.attempt + 1 })),
  };
}

/**
 * Opens an item or link where Core says it belongs: a file previews when the
 * app can show it and gets the file card otherwise, a school page opens in
 * its signed-in view (`openPage`), an outside page in the system browser.
 */
export function FilePreview({
  url,
  title,
  courseId,
  limit,
  inline = false,
  openPage,
  openFile,
}: {
  url: string;
  title: string;
  courseId: string;
  limit: number;
  /** The item's own text is shown above; a school page is then offered
   * rather than replacing it. */
  inline?: boolean;
  openPage: OpenPage;
  /** Opens one file of a folder in the reader. */
  openFile?: (url: string, name: string) => void;
}) {
  const expected = previewKind(url);
  // A file of a type the reader cannot show waits for Download.
  const course = useCourseFile(url, courseId, limit, {
    download: !downloadOnly(url),
    readText: true,
  });
  const { view } = course;
  const page = view.state === "page" ? view : undefined;
  useEffect(() => {
    if (page && !inline) openPage(page);
    // Opening replaces this preview; it runs once per answer.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page?.site, page?.url, inline]);
  const ready = view.state === "ready" ? view : undefined;
  const kind = ready?.kind ?? expected;
  const src = ready?.src;
  const textTooLarge =
    (kind === "text" || kind === "markdown") && ready && ready.text == null;
  const actions = <FileActions course={course} openPage={openPage} />;
  if (view.state === "loading") {
    return (
      <div className="lmsAttachmentPreview">
        <Toolbar title={title} url={url}>
          {actions}
        </Toolbar>
        <p className="lmsQuiet lmsLoading">{t("lms.loadingPreview")}</p>
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
            <small>{t("lms.countFiles", { count: view.files.length })}</small>
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
            <p className="lmsQuiet">{t("lms.thisFolderIsEmpty")}</p>
          )}
        </div>
      </div>
    );
  }
  if (view.state === "page") {
    return inline ? (
      <button
        type="button"
        className="lmsRefresh"
        onClick={() => openPage(view)}
      >
        {t("common.openInLMS")}
      </button>
    ) : null;
  }
  if (view.state === "external") {
    return (
      <div className="lmsFileCardStage">
        <ExternalCard title={title} url={view.url} />
      </div>
    );
  }
  if (!kind || !src || textTooLarge) {
    return (
      <div className="lmsFileCardStage">
        <FileCard
          course={course}
          title={title}
          openPage={openPage}
          previewable={Boolean(kind) && !textTooLarge}
          note={textTooLarge ? t("lms.thisTextFileExceedsThe") : undefined}
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
          <p className="lmsQuiet lmsLoading">{t("lms.loadingPreview")}</p>
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
              page: ready?.page ?? null,
              note: "lms.thisImageCantBeDisplayed",
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
          fallback={
            <p className="lmsQuiet lmsLoading">{t("common.loading")}</p>
          }
        >
          <MarkdownDocument value={ready?.text ?? ""} />
        </Suspense>
      ) : (
        <Suspense
          fallback={
            <p className="lmsQuiet lmsLoading">{t("lms.loadingPreview")}</p>
          }
        >
          <TextDocument value={ready?.text ?? ""} url={url} />
        </Suspense>
      )}
    </div>
  );
}

/**
 * Secondary actions: Show in Finder, Copy and Share (once on disk), Open in
 * LMS, Open in browser.
 */
function FileActions({
  course,
  openPage,
  menu = false,
}: {
  course: CourseFile;
  openPage: OpenPage;
  /** In a file card: one labelled row per action, like a native menu. */
  menu?: boolean;
}) {
  const { url, courseId, view } = course;
  const local = view.state === "ready";
  const page = pageOf(view);
  const reveal = async (open = false) => {
    const material = await openMaterial(courseId, url, { download: true });
    if (material.state === "ready")
      await openStoredFile(material.file.path, !open);
  };
  const withStored = async (act: (path: string) => Promise<void>) => {
    try {
      const material = await openMaterial(courseId, url, { download: true });
      if (material.state === "ready") await act(material.file.path);
    } catch (error) {
      notify.error(error);
    }
  };
  const copy = () =>
    withStored(async (path) => {
      await copyStoredFile(path);
      notify.info(t("lms.fileCopied"));
    });
  const share = (button: HTMLElement) => {
    const { x, y, width, height } = button.getBoundingClientRect();
    return withStored((path) => shareStoredFile(path, { x, y, width, height }));
  };
  // In the reader panel these collapse to icons; the label stays as tooltip
  // and name. As menu rows the visible label is both.
  const label = (text: MessageKey) =>
    menu
      ? { className: "uiMenuItem" }
      : { "aria-label": t(text), title: t(text) };
  return (
    <div className="lmsFileActions" data-layout={menu ? "menu" : undefined}>
      {local && (
        <button
          type="button"
          {...label(revealLabel)}
          onClick={() => void reveal()}
        >
          <LmsIcon name="folder" size={15} />
          <span className="lmsToolLabel">{t(revealLabel)}</span>
        </button>
      )}
      {local && systemShare && (
        <>
          <button
            type="button"
            {...label("lms.copyFile")}
            onClick={() => void copy()}
          >
            <LmsIcon name="copy" size={15} />
            <span className="lmsToolLabel">{t("lms.copyFile")}</span>
          </button>
          <button
            type="button"
            {...label("common.share")}
            onClick={(event) => void share(event.currentTarget)}
          >
            <LmsIcon name="share" size={15} />
            <span className="lmsToolLabel">{t("common.share")}</span>
          </button>
        </>
      )}
      {page && (
        <button
          type="button"
          {...label("common.openInLMS")}
          onClick={() => openPage(page)}
        >
          <LmsIcon name="page" size={15} />
          <span className="lmsToolLabel">{t("common.openInLMS")}</span>
        </button>
      )}
      <ExternalLink href={url} {...label("common.openInBrowser")}>
        <LmsIcon name="external" size={15} />
        <span className="lmsToolLabel">{t("common.openInBrowser")}</span>
      </ExternalLink>
    </div>
  );
}

/** A page outside the school sites, for the system browser. */
function ExternalCard({ title, url }: { title: string; url: string }) {
  return (
    <div className="lmsFileCard">
      <FileTile family="link" />
      <div className="lmsFileCardText">
        <h3>{title}</h3>
      </div>
      <ExternalLink href={url}>{t("common.openInBrowser")}</ExternalLink>
    </div>
  );
}

/** Name, size, state and one primary action for the file's current state. */
function FileCard({
  course,
  title,
  openPage,
  previewable,
  note: override,
}: {
  course: CourseFile;
  title: string;
  openPage: OpenPage;
  previewable: boolean;
  note?: string;
}) {
  const { url, view, limit } = course;
  const size = view.state === "tooLarge" ? megabytes(view.bytes) : "";
  const page = pageOf(view);
  const primary =
    view.state === "failed" && view.reason === "login" && page ? (
      <button type="button" onClick={() => openPage(page)}>
        {t("lms.signInToLMS")}
      </button>
    ) : view.state === "failed" &&
      ["unresolved", "invalid"].includes(view.reason) &&
      page ? (
      <button type="button" onClick={() => openPage(page)}>
        {t("common.openInLMS")}
      </button>
    ) : view.state === "failed" ? (
      <button type="button" onClick={course.retry}>
        {t("common.retry")}
      </button>
    ) : view.state === "tooLarge" ? (
      <button type="button" onClick={course.fetchNow}>
        <LmsIcon name="download" size={15} />
        {previewable ? t("lms.preview") : t("lms.download")} · {size}
      </button>
    ) : view.state === "ready" ? null : (
      <button
        type="button"
        disabled={view.state === "loading"}
        onClick={course.fetchNow}
      >
        <LmsIcon name="download" size={15} />
        {t(view.state === "loading" ? "common.downloading" : "lms.download")}
      </button>
    );
  const note =
    override ??
    (view.state === "failed"
      ? t(view.note ?? failureNote[view.reason])
      : view.state === "tooLarge"
        ? t("lms.largerThanTheAutomaticDownload", {
            size: megabytes(limit),
          })
        : view.state === "ready"
          ? t("lms.savedInYourCourseFolder")
          : t("lms.thisFileTypeCantBe"));
  return (
    <div className="lmsFileCard">
      <FileTile family={fileFamily(url)} />
      <div className="lmsFileCardText">
        <h3>{title}</h3>
        <small>
          {[extension(url).toUpperCase(), size].filter(Boolean).join(" · ")}
        </small>
      </div>
      <p role={view.state === "failed" ? "alert" : undefined}>{note}</p>
      <div className="lmsFileCardActions">
        {primary && <span className="lmsPrimaryAction">{primary}</span>}
        <FileActions course={course} openPage={openPage} menu />
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
  courseId,
  limit,
  openPage,
  anchor,
  onClose,
}: {
  url: string;
  title: string;
  courseId: string;
  limit: number;
  openPage: OpenPage;
  anchor: Element;
  onClose: () => void;
}) {
  const course = useCourseFile(url, courseId, limit, { download: false });
  const { view } = course;
  const box = useRef<HTMLDivElement>(null);
  // A school page opens where it belongs, as from the reader.
  useEffect(() => {
    if (view.state !== "page") return;
    onClose();
    openPage(view);
    // It runs once per answer.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view]);
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
      {view.state === "external" ? (
        <ExternalCard title={title} url={view.url} />
      ) : (
        <FileCard
          course={course}
          title={title}
          openPage={(page) => {
            onClose();
            openPage(page);
          }}
          previewable={false}
        />
      )}
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
 * React can run the cleanup while the element stays mounted (StrictMode's
 * extra effect pass in development, or a new `src`), so each run puts the
 * source back; otherwise the element would stay empty.
 */
function useReleaseSource<T extends HTMLImageElement | HTMLMediaElement>(
  src: string,
) {
  const element = useRef<T>(null);
  useEffect(() => {
    const current = element.current;
    if (current && current.getAttribute("src") !== src) {
      current.setAttribute("src", src);
      if (current instanceof HTMLMediaElement) current.load();
    }
    return () => {
      if (!current) return;
      if (current instanceof HTMLMediaElement) current.pause();
      current.removeAttribute("src");
      if (current instanceof HTMLMediaElement) current.load();
    };
  }, [src]);
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
  const image = useReleaseSource<HTMLImageElement>(src);
  return (
    <div className={`lmsImageStage${actual ? " actual" : ""}`}>
      <img
        ref={image}
        className="lmsImagePreview"
        src={src}
        alt={title}
        title={t(actual ? "lms.fitToWindow" : "lms.actualSize")}
        onClick={() => setActual((value) => !value)}
        onError={onBroken}
      />
    </div>
  );
}

function MediaView({ kind, src }: { kind: "video" | "audio"; src: string }) {
  const media = useReleaseSource<HTMLMediaElement>(src);
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
 * A folder's files listed in the outline, under the folder's own row, as
 * Core lists them (kept with the folder, else read from the LMS). Each file
 * opens the way an outline item does: previews in the reader, anything else
 * as the popover card at the clicked file.
 */
export function InlineFolder({
  url,
  courseId,
  limit,
  openFile,
  openPage,
}: {
  url: string;
  courseId: string;
  limit: number;
  openFile: (url: string, name: string, anchor: Element) => void;
  openPage: OpenPage;
}) {
  const { view } = useCourseFile(url, courseId, limit, { download: false });
  // Names carry the extension; folder URLs may not.
  const byName = (name: string) =>
    `https://file.invalid/${encodeURIComponent(name)}`;
  if (view.state === "loading")
    return <p className="lmsQuiet lmsFolderNote">{t("common.loading")}</p>;
  if (view.state !== "folder")
    return (
      <p className="lmsQuiet lmsFolderNote">
        {t("lms.thisFolderCouldNotBe")}{" "}
        {pageOf(view) && (
          <button
            type="button"
            className="lmsTextLink"
            onClick={() => openPage(pageOf(view)!)}
          >
            {t("common.openInLMS")}
          </button>
        )}
      </p>
    );
  if (!view.files.length)
    return (
      <p className="lmsQuiet lmsFolderNote">{t("lms.thisFolderIsEmpty")}</p>
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
