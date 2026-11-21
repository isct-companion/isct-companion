import { DueChip } from "../../components/DueChip";
import { Tag } from "../../components/ui";
import { formatJst } from "../../lib/civil-date";
import { type OpenedContent } from "./reading-state";
import {
  RichContent,
  type FileAccess,
  type LinkedFolders,
} from "./RichContent";
import type { Assignment } from "../../lib/model";
import { t } from "../../i18n";
import { AssignmentWorkspaceFooter } from "./AssignmentWorkspaceCard";
import { contentText } from "./content";
import { LmsIcon } from "./LmsIcon";
import { type AssignmentDetail } from "../../platform/core-query";

/** Assignment card: state and deadline first, the teacher's text on request. */
export function LmsAssignmentCard({
  assignment: a,
  detail,
  assignmentState,
  availability,
  deadline,
  original,
  showBody,
  query,
  toggleDetails,
  locale,
  open,
  files,
  openLink,
  linkedFolders,
  workspaces,
}: {
  assignment: Assignment;
  detail?: AssignmentDetail;
  assignmentState?: AssignmentDetail["state"];
  availability?: AssignmentDetail["handInAvailability"];
  deadline?: AssignmentDetail["deadline"];
  original: boolean;
  showBody: boolean;
  query: string;
  toggleDetails: () => void;
  locale: string;
  open: (content: OpenedContent) => void;
  files: FileAccess;
  openLink: (url: string, title: string, anchor?: Element) => void;
  linkedFolders: LinkedFolders;
  workspaces: boolean;
}) {
  const done = assignmentState === "submitted" || assignmentState === "graded";
  const due = deadline ? new Date(deadline.at) : undefined;
  const state =
    assignmentState &&
    {
      open: "open",
      overdue: "missed",
      closed: "missed",
      submitted: "done",
      graded: "graded",
    }[assignmentState];
  const body =
    (original
      ? (detail?.originalBodyHtml ?? detail?.bodyHtml)
      : detail?.bodyHtml) ?? "";
  const hasBody = Boolean(contentText(body).trim() || a.details?.trim());
  const submit = (preparedUrl?: string) =>
    open({
      id: `official:${a.id}`,
      title: a.title,
      kind: "official",
      url: preparedUrl ?? a.sourceUrl!,
    });
  return (
    <article
      className={`lmsAssignmentCard ${state ?? ""}`}
      data-assignment={a.id}
    >
      <header>
        <span className="lmsAssignmentMark" aria-hidden="true" />
        <div className="lmsAssignmentTitle">
          <h4>
            {hasBody && !query.trim() ? (
              <button
                type="button"
                aria-expanded={showBody}
                onClick={toggleDetails}
              >
                {a.title}
                <LmsIcon
                  name="chevron"
                  size={12}
                  className="lmsAssignmentChevron"
                />
              </button>
            ) : (
              a.title
            )}
          </h4>
          {due && (
            <time dateTime={deadline?.at}>
              {t("lms.due")}{" "}
              {formatJst(due, locale, {
                month: "short",
                day: "numeric",
                weekday: "short",
                hour: "2-digit",
                minute: "2-digit",
              })}
            </time>
          )}
        </div>
        {deadline && !done ? (
          <DueChip due={deadline} locale={locale} />
        ) : (
          state && (
            <Tag
              tone={
                state === "graded" || state === "done"
                  ? "ok"
                  : state === "missed"
                    ? "danger"
                    : "neutral"
              }
              className={`lmsAssignmentState ${state}`}
            >
              {t(
                state === "graded"
                  ? "common.graded"
                  : state === "done"
                    ? "common.submitted"
                    : state === "missed"
                      ? "lms.notSubmitted"
                      : "common.noDeadline",
              )}
              {state === "graded" && a.grade && (
                <span className="lmsAssignmentGrade">{a.grade}</span>
              )}
            </Tag>
          )
        )}
      </header>
      {showBody && hasBody && (
        <div className="lmsAssignmentBody">
          {body ? (
            <RichContent
              files={files}
              html={body}
              openLink={openLink}
              folders={linkedFolders}
            />
          ) : (
            <p>{a.details}</p>
          )}
        </div>
      )}
      {workspaces ? (
        <AssignmentWorkspaceFooter
          assignment={a}
          state={assignmentState}
          dueAt={deadline?.at}
          submit={
            a.sourceUrl && availability !== "unavailable" ? submit : undefined
          }
        />
      ) : (
        <footer>
          {a.sourceUrl && availability !== "unavailable" && (
            <button
              type="button"
              className={done ? "lmsTextLink" : "lmsSubmit"}
              onClick={() => submit()}
            >
              {t(done ? "lms.viewInLMS" : "common.submit")}{" "}
              <LmsIcon name="external" size={13} />
            </button>
          )}
        </footer>
      )}
    </article>
  );
}
