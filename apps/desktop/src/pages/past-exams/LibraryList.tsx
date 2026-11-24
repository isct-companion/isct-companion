import type { ReactNode } from "react";
import { Library, Trash2 } from "lucide-react";
import {
  Button,
  EmptyState,
  ErrorBanner,
  Skeleton,
  Tag,
} from "../../components/ui";
import { confirmAction } from "../../components/confirm";
import { t } from "../../i18n";
import type { ExamDocument, ExamLibrary } from "../../generated/persistence";
import {
  Row,
  Workspace,
  documentOccasion,
  roleName,
  subjectLabel,
  teacherLine,
  type ViewProps,
} from "./ExamList";

export function LibraryList({
  call,
  open,
  opening,
  reading,
  pane,
  library,
  failed,
  refresh,
  replace,
  closeReader,
}: Omit<ViewProps, "generation"> & {
  library?: ExamLibrary;
  failed: boolean;
  refresh: () => void;
  replace: (value: ExamLibrary) => void;
  closeReader: (id: string) => void;
}) {
  const remove = (kind: "removeDownload" | "deleteSubmission", id: string) => {
    const label = t(
      kind === "deleteSubmission"
        ? "examShare.deleteSubmission"
        : "examShare.removeDownload",
    );
    return (
      <Button
        variant="subtle"
        color="gray"
        size="compact-sm"
        className="examRowTool"
        aria-label={label}
        title={label}
        onClick={async () => {
          if (!(await confirmAction(label, t("common.delete")))) return;
          const answer = await call({ kind, id });
          if (answer?.kind === "library") {
            closeReader(id);
            replace(answer.value);
          }
        }}
      >
        <Trash2 size={15} strokeWidth={1.75} aria-hidden />
      </Button>
    );
  };
  const section = (
    key: "saved" | "downloaded" | "submitted",
    items: ExamDocument[],
    removal?: "removeDownload" | "deleteSubmission",
  ) =>
    items.length > 0 && (
      <section key={key}>
        <h2 className="examListHeading">{t(`examShare.${key}`)}</h2>
        <ul className="examList">
          {items.map((document) => (
            <DocumentRow
              key={document.id}
              document={document}
              open={open}
              opening={opening}
              current={reading === document.id}
              trailing={removal && remove(removal, document.id)}
            />
          ))}
        </ul>
      </section>
    );

  const empty =
    library &&
    !library.saved.length &&
    !library.downloaded.length &&
    !library.submitted.length;
  return (
    <Workspace>
      <div className="examPane">
        <div className="scroll">
          {failed && !library ? (
            <ErrorBanner onRetry={refresh}>
              {t("examShare.loadFailed")}
            </ErrorBanner>
          ) : !library ? (
            <Skeleton count={6} />
          ) : (
            <>
              {library.offline && (
                <ErrorBanner onRetry={refresh}>
                  {t("examShare.libraryUnavailable")}
                </ErrorBanner>
              )}
              {empty ? (
                <EmptyState compact icon={Library}>
                  {t("examShare.emptyLibrary")}
                </EmptyState>
              ) : (
                <>
                  {section("saved", library.saved)}
                  {section("downloaded", library.downloaded, "removeDownload")}
                  {section("submitted", library.submitted, "deleteSubmission")}
                </>
              )}
            </>
          )}
        </div>
      </div>
      <div className="examPane examDetail">
        {pane || (
          <EmptyState icon={Library}>{t("examShare.pickDocument")}</EmptyState>
        )}
      </div>
    </Workspace>
  );
}

function DocumentRow({
  document,
  open,
  opening,
  current,
  trailing,
}: {
  document: ExamDocument;
  open: (id: string) => void;
  opening?: string;
  current?: boolean;
  trailing?: ReactNode;
}) {
  return (
    <Row
      title={
        <>
          {document.subjects.map(subjectLabel).join(" / ")}
          {document.role && (
            <Tag tone={document.role === "answer" ? "ok" : "neutral"}>
              {roleName(document.role, document.roleText)}
            </Tag>
          )}
        </>
      }
      meta={[
        [document.academicYear, documentOccasion(document)]
          .filter(Boolean)
          .join(" "),
        teacherLine(document),
        document.note,
      ]
        .filter(Boolean)
        .join(" · ")}
      current={current}
      pending={opening === document.id}
      onClick={() => open(document.id)}
      trailing={trailing}
    />
  );
}
