import { byName, FileName, fileSize, megabytes } from "./file-row";
import {
  pageOf,
  useCourseFile,
  type CourseFile,
  type OpenPage,
} from "./course-file";
import { ReaderTools } from "../../components/files/ReaderPane";
import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { notify } from "../../components/notifications/notify";
import { nativeOs } from "../../platform/desktop-bridge";
import { ExternalLink } from "../../components/ExternalLink";
import { t, type MessageKey } from "../../i18n";
import {
  downloadOnly,
  extension,
  fileFamily,
  previewOf,
  type FileFamily,
} from "../../lib/file-types";
import { FileView } from "../../components/files/FileView";
import { FileTile, LmsIcon } from "./LmsIcon";
import {
  copyStoredFile,
  openMaterial,
  openStoredFile,
  shareStoredFile,
  previewKind,
  type LmsFileReason,
  type Material,
} from "./files";
const revealLabel: MessageKey =
  nativeOs() === "macos" ? "web.showInFinder" : "web.showInFolder";
// Only macOS and Windows have a system Copy and Share for a file (file_share.rs).
const systemShare = nativeOs() === "macos" || nativeOs() === "windows";
const deniedNote: MessageKey =
  nativeOs() === "windows"
    ? "lms.windowsBlockedSavingToThe"
    : "lms.theSystemBlockedSavingTo";

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

/** The downloaded file an answer names: a file, or a ZIP shown as its files. */
const storedPath = (material: Material) =>
  material.state === "ready" || material.state === "archive"
    ? material.file.path
    : undefined;

/**
 * Opens an item or link where Core says it belongs: a file previews when the
 * app can show it and gets the file card otherwise, a school page opens in
 * its signed-in view (`openPage`), an outside page in the reader's web view
 * (`openWeb`).
 */
export function FilePreview({
  url,
  title,
  courseId,
  limit,
  inline = false,
  openPage,
  openWeb,
  openFile,
  onArchive,
}: {
  url: string;
  title: string;
  courseId: string;
  limit: number;
  /** The item's own text is shown above; a school page is then offered
   * rather than replacing it. */
  inline?: boolean;
  openPage: OpenPage;
  /** Shows a page outside the school sites in the reader. */
  openWeb: (url: string) => void;
  /** Opens one file of a folder or a ZIP in the reader. */
  openFile?: (url: string, name: string) => void;
  /** Core answered a ZIP whose address did not say so: its files belong in
   * the outline, as a folder's do. True when the outline took it. */
  onArchive?: () => boolean;
}) {
  const expected = previewKind(url);
  // A file of a type the reader cannot show waits for Download.
  const course = useCourseFile(url, courseId, limit, {
    download: !downloadOnly(url),
    readText: true,
  });
  const { view } = course;
  const page = view.state === "page" ? view : undefined;
  const web = view.state === "external" ? view.url : undefined;
  const archive = view.state === "archive";
  const [handedOff, setHandedOff] = useState(false);
  useEffect(() => {
    if (inline) return;
    if (page) openPage(page);
    else if (web) openWeb(web);
    else if (archive && onArchive?.()) setHandedOff(true);
    // Opening replaces this preview; it runs once per answer.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page?.site, page?.url, web, archive, inline]);
  const ready = view.state === "ready" ? view : undefined;
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
  if (view.state === "archive" && handedOff) return null;
  if (view.state === "folder" || view.state === "archive") {
    const rows = view.files.map((file) => ({
      key: file.url,
      name: file.name,
      detail: "bytes" in file ? fileSize(file.bytes) : undefined,
      ...(openFile
        ? { open: () => openFile(file.url, file.name) }
        : { href: file.url }),
    }));
    const list = (
      <div className="lmsFolderView">
        <FileRows family={view.state} title={title} rows={rows} />
      </div>
    );
    return view.state === "archive" ? (
      <div className="lmsAttachmentPreview">
        <Toolbar title={title} url={url}>
          {actions}
        </Toolbar>
        {list}
      </div>
    ) : (
      list
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
    return inline ? (
      <button
        type="button"
        className="lmsRefresh"
        onClick={() => openWeb(view.url)}
      >
        {t("lms.openPage")}
      </button>
    ) : null;
  }
  // Before the file is on disk its address says whether it will preview;
  // once it is, Core's viewer does. A ZIP Core did not list (too many files,
  // too large, unreadable) and text over the limit open with another app.
  const card = (
    <div className="lmsFileCardStage">
      <FileCard
        course={course}
        title={title}
        openPage={openPage}
        previewable={
          ready
            ? ready.viewer !== "tooLarge" && previewOf(ready.viewer) != null
            : Boolean(expected) && expected !== "archive"
        }
        note={
          ready?.viewer === "tooLarge"
            ? t("lms.thisTextFileExceedsThe")
            : ready && expected === "archive"
              ? t("lms.thisArchiveIsTooLarge")
              : undefined
        }
      />
    </div>
  );
  if (!ready) return card;
  return (
    <FileView
      viewer={ready.viewer}
      path={ready.path}
      name={ready.name}
      title={title}
      text={ready.text}
      toolbar={
        <Toolbar title={title} url={url} inline>
          {actions}
        </Toolbar>
      }
      fallback={card}
      onBroken={() =>
        course.setView({
          state: "failed",
          reason: "network",
          page: ready.page,
          note: "lms.thisImageCantBeDisplayed",
        })
      }
    />
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
  const local = view.state === "ready" || view.state === "archive";
  const page = pageOf(view);
  const withStored = async (act: (path: string) => Promise<void>) => {
    try {
      const path = storedPath(
        await openMaterial(courseId, url, { download: true }),
      );
      if (path) await act(path);
    } catch (error) {
      notify.error(error);
    }
  };
  const reveal = (open = false) =>
    withStored((path) => openStoredFile(path, !open));
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
      {/* A file card offers it as its primary action instead. */}
      {local && !menu && (
        <button
          type="button"
          {...label("lms.openWithDefaultApp")}
          onClick={() => void reveal(true)}
        >
          <LmsIcon name="open" size={15} />
          <span className="lmsToolLabel">{t("lms.openWithDefaultApp")}</span>
        </button>
      )}
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
    ) : view.state === "ready" ? (
      <button
        type="button"
        onClick={() =>
          void openStoredFile(view.path, false).catch((error) =>
            notify.error(error),
          )
        }
      >
        <LmsIcon name="open" size={15} />
        {t("lms.openWithDefaultApp")}
      </button>
    ) : (
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
 * Files listed under their container, a folder or a ZIP, as rows that open
 * in the reader (`open`) or download (`href`).
 */
function FileRows({
  family,
  title,
  rows,
}: {
  family: FileFamily;
  title: string;
  rows: ({ key: string; name: string; detail?: string } & (
    { open: () => void } | { href: string }
  ))[];
}) {
  return (
    <>
      <header className="lmsFolderHeader">
        <FileTile family={family} />
        <div>
          <h3>{title}</h3>
          <small>{t("lms.countFiles", { count: rows.length })}</small>
        </div>
      </header>
      <div className="lmsFolderList">
        {rows.map((row) => {
          const body = (
            <>
              <FileTile family={fileFamily(byName(row.name))} />
              <FileName name={row.name} detail={row.detail} />
              <LmsIcon name="chevron" className="lmsRowAction" />
            </>
          );
          return "open" in row ? (
            <button
              type="button"
              className="lmsItemRow"
              key={row.key}
              onClick={row.open}
            >
              {body}
            </button>
          ) : (
            <a className="lmsItemRow" key={row.key} href={row.href} download>
              {body}
            </a>
          );
        })}
        {!rows.length && (
          <p className="lmsQuiet">{t("lms.thisFolderIsEmpty")}</p>
        )}
      </div>
    </>
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
