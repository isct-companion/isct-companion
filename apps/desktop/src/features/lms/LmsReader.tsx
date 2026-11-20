import { downloadLimit } from "../../lib/course-file-policy";
import { ReaderPane } from "../../components/files/ReaderPane";
import { type OpenedContent } from "./reading-state";
import {
  RichContent,
  type FileAccess,
  type LinkedFolders,
} from "./RichContent";
import { lazy, Suspense } from "react";
import type { Course, Material } from "../../lib/model";
import { t } from "../../i18n";
import { announcements, formatLmsDate, safeUrl } from "./content";
import { OfficialLmsView } from "./OfficialLmsView";
import { SyllabusFrame } from "../../components/courses/syllabus-frame";
import type { useLmsReader } from "./useLmsReader";

const FilePreview = lazy(() =>
  import("./FilePreview").then((m) => ({ default: m.FilePreview })),
);
const NoteEditor = lazy(() =>
  import("./NoteEditor").then((m) => ({ default: m.NoteEditor })),
);

export function LmsReader({
  opened,
  shown,
  viewerActive,
  course,
  locale,
  announcement,
  files,
  linkedFolders,
  material,
  materialBody,
  limit,
  schoolWebAvailable,
  openAccounts,
  refreshNotes,
  setDeletedNote,
  reader,
}: {
  opened: OpenedContent;
  shown: OpenedContent;
  viewerActive: boolean;
  course: Course;
  locale: string;
  announcement?: ReturnType<typeof announcements>[number];
  files: FileAccess;
  linkedFolders: LinkedFolders;
  material?: Material;
  materialBody?: string | null;
  limit: ReturnType<typeof downloadLimit>;
  schoolWebAvailable: boolean;
  openAccounts?: () => void;
  refreshNotes: () => void;
  setDeletedNote: (note: { id: string; section: string; title: string }) => void;
  reader: ReturnType<typeof useLmsReader>;
}) {
  const { closeContent, openLink, openPage, openWebPage, archiveInOutline, open } =
    reader;
  return (
    <ReaderPane title={shown.title} close={closeContent}>
      {/* Hidden readers release their documents; the official view keeps its webview. */}
      {!viewerActive &&
      opened.kind !== "official" ? null : opened.kind ===
        "announcement" ? (
        <div className="lmsDocument">
          <div className="lmsDocumentHeading">
            <small>
              {formatLmsDate(
                announcement?.date || "",
                locale,
              )}
            </small>
            <h2>{opened.title}</h2>
          </div>
          {announcement && (
            <RichContent
              files={files}
              html={announcement.html}
              openLink={openLink}
              folders={linkedFolders}
            />
          )}
        </div>
      ) : opened.kind === "syllabus" ? (
        <div className="lmsDocument lmsSyllabus">
          <SyllabusFrame url={shown.url} />
        </div>
      ) : opened.kind === "note" ? (
        <Suspense fallback={<p>{t("lms.loadingEditor")}</p>}>
          <NoteEditor
            key={opened.id}
            id={opened.id}
            context={{ courseId: course.id, section: opened.section! }}
            onSaved={refreshNotes}
            onDeleted={() => {
              setDeletedNote({
                id: opened.id,
                section: opened.section!,
                title: opened.title,
              });
              closeContent();
              refreshNotes();
            }}
          />
        </Suspense>
      ) : opened.kind === "official" ? (
        <>
          <OfficialLmsView
            url={opened.url!}
            active={viewerActive}
            available={schoolWebAvailable}
            openAccounts={openAccounts}
          />
        </>
      ) : (
        <div className="lmsDocument lmsFileDocument">
          <div className="lmsDocumentHeading">
            <small>{material?.sectionTitle || course.code}</small>
            <h2>{opened.title}</h2>
          </div>
          {materialBody && (
            <RichContent
              files={files}
              html={materialBody}
              openLink={openLink}
              folders={linkedFolders}
            />
          )}
          {(() => {
            // The item's own address, as Core's read models pick it;
            // Core decides what it opens as (`open_material`).
            const url = safeUrl(
              material?.url ||
                material?.resourceUrl ||
                (opened.kind === "link" ? opened.url : undefined),
            );
            if (!url)
              return material && !materialBody ? (
                <p className="lmsQuiet">
                  {t("lms.noContentInThisSection")}
                </p>
              ) : null;
            return (
              <Suspense fallback={<p>{t("lms.loadingPreview")}</p>}>
                <FilePreview
                  key={url}
                  url={url}
                  title={opened.title}
                  courseId={course.id}
                  limit={limit}
                  inline={Boolean(materialBody)}
                  openPage={(page) => openPage(page, opened.title)}
                  openWeb={(page) => openWebPage(page, opened.title)}
                  onArchive={() => archiveInOutline(opened)}
                  openFile={(fileUrl, name) =>
                    open({
                      id: `link:${fileUrl}`,
                      title: name,
                      kind: "link",
                      url: fileUrl,
                    })
                  }
                />
              </Suspense>
            );
          })()}
        </div>
      )}
    </ReaderPane>
  );
}
