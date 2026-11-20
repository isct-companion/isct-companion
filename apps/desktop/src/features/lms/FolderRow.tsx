import { lazy, Suspense, useState } from "react";
import { t } from "../../i18n";
import type { SchoolPage } from "./files";
import { FileTile, LmsIcon } from "./LmsIcon";

const InlineFolder = lazy(() =>
  import("./FilePreview").then((m) => ({ default: m.InlineFolder })),
);

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
        <Suspense
          fallback={
            <p className="lmsQuiet lmsFolderNote">{t("common.loading")}</p>
          }
        >
          <InlineFolder
            url={url}
            courseId={courseId}
            limit={limit}
            openFile={openFile}
            openPage={openPage}
            archive={archive}
          />
        </Suspense>
      )}
    </div>
  );
}
