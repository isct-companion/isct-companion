import { useEffect, useState } from "react";
import { t } from "../../i18n";
import { useSubmissionGuard } from "../../components/use-submission-guard";
import { UiInput } from "../../components/mantine";
import { FileTile, LmsIcon } from "./LmsIcon";
import type { CourseFolder } from "./files";
import type { AssignmentAttachment } from "../../platform/core-query";
import {
  createWorkspace,
  deleteWorkspace,
  findWorkspace,
  openWorkspace,
  prepareSubmission,
  setWorkspaceSubmission,
  type AssignmentWorkspace,
} from "./assignment-workspace";

/**
 * Desktop assignment footer: the assignment workspace and a handoff to the official draft upload area.
 */
export function AssignmentWorkspaceFooter({
  context,
  assignment,
  attachments,
  brief,
  done,
  submit,
}: {
  context: CourseFolder;
  assignment: { id: string; title: string; sourceUrl?: string };
  /** The LMS files the assignment links, from Core's `AssignmentDetail`. */
  attachments: readonly AssignmentAttachment[];
  brief: (attachments: string[]) => string;
  done: boolean;
  /** Opens the assignment in the LMS; undefined when it has no LMS page. */
  submit?: (preparedUrl?: string) => void;
}) {
  const [workspace, setWorkspace] = useState<AssignmentWorkspace | null>();
  const { busy, error, run } = useSubmissionGuard();
  const [zipName, setZipName] = useState("");
  useEffect(() => {
    let live = true;
    void findWorkspace(context, assignment.id)
      .then((found) => live && setWorkspace(found))
      .catch(() => live && setWorkspace(null));
    return () => {
      live = false;
    };
  }, [context, assignment.id]);
  useEffect(() => setZipName(workspace?.zipName ?? ""), [workspace?.zipName]);

  const handIn = () =>
    run(async () => {
      if (workspace && !done && assignment.sourceUrl) {
        const prepared = await prepareSubmission(
          workspace.path,
          assignment.id,
          assignment.sourceUrl,
        );
        setWorkspace(prepared.workspace);
        submit?.(prepared.url);
      } else submit?.();
    });
  const saveSubmission = (zip: boolean, name = zipName) => {
    if (!workspace) return;
    void run(async () =>
      setWorkspace(await setWorkspaceSubmission(workspace.path, zip, name)),
    );
  };

  return (
    <>
      {workspace && (
        <section className="lmsWorkspaceCard" aria-label={t("Workspace")}>
          <header>
            <FileTile family="folder" />
            <div>
              <b>{workspace.name}</b>
              <small>
                {workspace.submitFiles.length
                  ? t("submit/ · {{count}} files", {
                      count: workspace.submitFiles.length,
                    })
                  : t("submit/ is empty")}
              </small>
            </div>
            <button
              type="button"
              className="lmsIconButton"
              aria-label={t("Delete workspace")}
              title={t("Delete workspace")}
              disabled={busy}
              onClick={() => {
                if (
                  !confirm(
                    t("Delete the workspace “{{name}}” and everything in it?", {
                      name: workspace.name,
                    }),
                  )
                )
                  return;
                void run(async () => {
                  await deleteWorkspace(workspace.path);
                  setWorkspace(null);
                });
              }}
            >
              <LmsIcon name="trash" size={15} />
            </button>
          </header>
          <div className="lmsWorkspaceActions">
            <button
              type="button"
              onClick={() =>
                void run(() => openWorkspace(workspace.path, "vscode"))
              }
            >
              <LmsIcon name="code" size={14} />
              {t("Open in VS Code")}
            </button>
            <button
              type="button"
              onClick={() =>
                void run(() => openWorkspace(workspace.path, "finder"))
              }
            >
              <LmsIcon name="folder" size={14} />
              {t("Open folder")}
            </button>
          </div>
          <div className="lmsWorkspaceSubmit">
            <span
              className="lmsSegmented"
              role="radiogroup"
              aria-label={t("Hand in as")}
            >
              <button
                type="button"
                role="radio"
                aria-checked={!workspace.zip}
                onClick={() => workspace.zip && saveSubmission(false)}
              >
                {t("Files")}
              </button>
              <button
                type="button"
                role="radio"
                aria-checked={workspace.zip}
                onClick={() => !workspace.zip && saveSubmission(true)}
              >
                {t("ZIP")}
              </button>
            </span>
            {workspace.zip && (
              <UiInput
                aria-label={t("ZIP file name")}
                value={zipName}
                spellCheck={false}
                onChange={(event) => setZipName(event.target.value)}
                onBlur={() =>
                  zipName !== workspace.zipName && saveSubmission(true)
                }
                onKeyDown={(event) => {
                  if (event.key === "Enter") event.currentTarget.blur();
                }}
              />
            )}
          </div>
        </section>
      )}
      {workspace && !done && (
        <p className="lmsWorkspaceHint">
          {t("Files upload to the LMS draft. Review and submit in LMS.")}
        </p>
      )}
      <footer>
        {submit && (
          <button
            type="button"
            className={done ? "lmsTextLink" : "lmsSubmit"}
            disabled={busy}
            onClick={() => void handIn()}
          >
            {t(done ? "View in LMS" : "Prepare in LMS")}{" "}
            <LmsIcon name="external" size={13} />
          </button>
        )}
        {workspace === null && (
          <button
            type="button"
            className="lmsTextLink"
            disabled={busy}
            onClick={() =>
              void run(async () =>
                setWorkspace(
                  await createWorkspace(
                    context,
                    assignment,
                    attachments,
                    brief,
                  ),
                ),
              )
            }
          >
            {t(busy ? "Creating workspace…" : "Create workspace")}
          </button>
        )}
      </footer>
      {error && (
        <p className="lmsWorkspaceError" role="alert">
          {error}
        </p>
      )}
    </>
  );
}
