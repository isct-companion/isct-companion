import { notify } from "../../components/notifications/notify";
import { byName, FileName, fileSize, megabytes } from "./file-row";
import { fileFamily, listsAsFolder } from "./file-types";
import { pageOf, useCourseFile, type OpenPage } from "./course-file";
import { openStoredFile, type SchoolPage } from "./files";
import { useState } from "react";
import { t } from "../../i18n";
import { FileTile, LmsIcon } from "./LmsIcon";

/**
 * A folder in the outline, and a ZIP shown exactly as one: the folder tile,
 * its name and a chevron, its files listed under it while open
 * (`InlineFolder`). `compact` is a folder's own file row, for a ZIP among a
 * folder's files. Without `open`/`onToggle` the row keeps its own state.
 */
export function FolderRow({
  title,
  url,
  archive,
  courseId,
  limit,
  openFile,
  openPage,
  open,
  onToggle,
  compact = false,
}: {
  title: string;
  url: string;
  /** A ZIP rather than an LMS folder (`InlineFolder`). */
  archive: boolean;
  courseId: string;
  limit: number;
  openFile: (url: string, name: string, anchor: Element) => void;
  openPage: (page: SchoolPage) => void;
  open?: boolean;
  onToggle?: () => void;
  compact?: boolean;
}) {
  const [own, setOwn] = useState(false);
  const isOpen = open ?? own;
  return (
    <div className={`lmsFolder${isOpen ? " open" : ""}`}>
      <button
        type="button"
        className={compact ? "lmsFolderFile" : "lmsItemRow"}
        aria-expanded={isOpen}
        onClick={onToggle ?? (() => setOwn((value) => !value))}
      >
        <FileTile family="folder" />
        <span>
          <b>{title}</b>
          <small>{t("lms.folder")}</small>
        </span>
        <LmsIcon name="chevron" className="lmsRowAction lmsFolderChevron" />
      </button>
      {isOpen && (
        <InlineFolder
          url={url}
          courseId={courseId}
          limit={limit}
          openFile={openFile}
          openPage={openPage}
          archive={archive}
        />
      )}
    </div>
  );
}

/**
 * A folder's or a ZIP's files listed in the outline, under its own row, as
 * Core lists them (a folder's kept with it, else read from the LMS; a ZIP's
 * from the downloaded archive). Each file opens the way an outline item
 * does: previews in the reader, anything else as the popover card at the
 * clicked file.
 */
function InlineFolder({
  url,
  courseId,
  limit,
  openFile,
  openPage,
  archive = false,
}: {
  url: string;
  courseId: string;
  limit: number;
  openFile: (url: string, name: string, anchor: Element) => void;
  openPage: OpenPage;
  /** A ZIP rather than an LMS folder: downloaded to read its files, within
   * the automatic limit. A folder lists what Core kept or reads from the LMS. */
  archive?: boolean;
}) {
  const course = useCourseFile(url, courseId, limit, { download: archive });
  const { view } = course;
  if (view.state === "loading")
    return <p className="lmsQuiet lmsFolderNote">{t("common.loading")}</p>;
  if (view.state === "tooLarge")
    return (
      <p className="lmsQuiet lmsFolderNote">
        {t("lms.largerThanTheAutomaticDownload", { size: megabytes(limit) })}{" "}
        <button type="button" className="lmsTextLink" onClick={course.fetchNow}>
          {t("lms.download")} · {megabytes(view.bytes)}
        </button>
      </p>
    );
  if (view.state === "ready")
    return (
      <p className="lmsQuiet lmsFolderNote">
        {t("lms.thisArchiveIsTooLarge")}{" "}
        <button
          type="button"
          className="lmsTextLink"
          onClick={() =>
            void openStoredFile(view.path, false).catch((error) =>
              notify.error(error),
            )
          }
        >
          {t("lms.openWithDefaultApp")}
        </button>
      </p>
    );
  if (view.state !== "folder" && view.state !== "archive")
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
      {view.files.map((file) =>
        listsAsFolder(file.url) ? (
          <FolderRow
            key={file.url}
            title={file.name}
            url={file.url}
            archive
            courseId={courseId}
            limit={limit}
            openFile={openFile}
            openPage={openPage}
            compact
          />
        ) : (
          <button
            type="button"
            className="lmsFolderFile"
            key={file.url}
            onClick={(event) =>
              openFile(file.url, file.name, event.currentTarget)
            }
          >
            <FileTile family={fileFamily(byName(file.name))} />
            <FileName
              name={file.name}
              detail={"bytes" in file ? fileSize(file.bytes) : undefined}
            />
          </button>
        ),
      )}
    </div>
  );
}
