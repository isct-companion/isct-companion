import { useCallback, useEffect, useRef, useState } from "react";
import { useEventListener } from "usehooks-ts";
import { t } from "../../i18n";
import { confirmAction } from "../../components/confirm";
import { useSubmissionGuard } from "../../components/use-submission-guard";
import { UiInput } from "../../components/ui";
import { FileTile, LmsIcon } from "./LmsIcon";
import type { CourseFolder } from "./files";
import {
  createWorkspace,
  deleteWorkspace,
  findWorkspace,
  openWorkspace,
  prepareSubmission,
  setWorkspaceSubmission,
  workspaceMissing,
  type AssignmentWorkspace,
  type HandInView,
} from "./assignment-workspace";

/**
 * Desktop assignment footer: the assignment workspace and a handoff to the official draft upload area.
 */
export function AssignmentWorkspaceFooter({
  context,
  assignment,
  closed,
  submit,
}: {
  context: CourseFolder;
  assignment: { id: string; title: string; sourceUrl?: string };
  /** In `TaskView.closedIds`: submitted, or past a deadline LMS no longer extends. */
  closed: boolean;
  /** Opens the assignment in the LMS; undefined when it has no LMS page. */
  submit?: (preparedUrl?: string) => void;
}) {
  const [view, setView] = useState<HandInView>();
  const workspace = view?.workspace;
  // The workspace on screen, to tell a folder deleted in Finder from one never made.
  const shown = useRef<AssignmentWorkspace | null>(null);
  const [lost, setLost] = useState<string>();
  /** Attachments Core could not download when it created the workspace. */
  const [missing, setMissing] = useState<string[]>([]);
  const { busy, error, run } = useSubmissionGuard();
  const [zipName, setZipName] = useState("");
  const load = useCallback(
    () =>
      findWorkspace(context, assignment.id, closed).then((next) => {
        if (next.workspace) setLost(undefined);
        else if (shown.current) setLost(shown.current.name);
        shown.current = next.workspace;
        setView(next);
      }),
    [context, assignment.id, closed],
  );
  useEffect(() => {
    void load().catch(() => setView({ workspace: null, handIn: "openLms" }));
  }, [load]);
  // Folders are also edited in Finder and VS Code; look again on return.
  useEventListener("focus", () => void load().catch(() => undefined));
  useEffect(() => setZipName(workspace?.zipName ?? ""), [workspace?.zipName]);

  const update = (next: AssignmentWorkspace) => {
    shown.current = next;
    setView((current) => current && { ...current, workspace: next });
  };
  /** A folder that vanished becomes the notice below, not an OS error. */
  const act = (work: () => Promise<unknown>) =>
    run(async () => {
      try {
        await work();
      } catch (cause) {
        if (!workspaceMissing(cause)) throw cause;
        await load();
      }
    });
  const prepare = () =>
    act(async () => {
      if (!workspace || !assignment.sourceUrl) return submit?.();
      const prepared = await prepareSubmission(
        workspace.path,
        assignment.id,
        assignment.sourceUrl,
      );
      update(prepared.workspace);
      submit?.(prepared.url);
    });
  const saveSubmission = (zip: boolean, name = zipName) => {
    if (!workspace) return;
    void act(async () =>
      update(await setWorkspaceSubmission(workspace.path, zip, name)),
    );
  };
  const draft = view?.handIn === "prepareDraft";

  return (
    <>
      {workspace && (
        <section className="lmsWorkspaceCard" aria-label={t("lms.workspace")}>
          <header>
            <FileTile family="folder" />
            <div>
              <b>{workspace.name}</b>
              <small>
                {workspace.submitFiles.length
                  ? t("lms.submitCountFiles", {
                      count: workspace.submitFiles.length,
                    })
                  : t("lms.submitIsEmpty")}
              </small>
            </div>
            <button
              type="button"
              className="lmsIconButton"
              aria-label={t("lms.deleteWorkspace")}
              title={t("lms.deleteWorkspace")}
              disabled={busy}
              onClick={async () => {
                if (
                  !(await confirmAction(
                    t("lms.deleteTheWorkspaceNameAnd", {
                      name: workspace.name,
                    }),
                    t("common.delete"),
                  ))
                )
                  return;
                void run(async () => {
                  await deleteWorkspace(workspace.path);
                  shown.current = null;
                  await load();
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
                void act(() => openWorkspace(workspace.path, "vscode"))
              }
            >
              <LmsIcon name="code" size={14} />
              {t("lms.openInVSCode")}
            </button>
            <button
              type="button"
              onClick={() =>
                void act(() => openWorkspace(workspace.path, "finder"))
              }
            >
              <LmsIcon name="folder" size={14} />
              {t("lms.openFolder")}
            </button>
          </div>
          <div className="lmsWorkspaceSubmit">
            <span
              className="lmsSegmented"
              role="radiogroup"
              aria-label={t("lms.handInAs")}
            >
              <button
                type="button"
                role="radio"
                aria-checked={!workspace.zip}
                onClick={() => workspace.zip && saveSubmission(false)}
              >
                {t("lms.files")}
              </button>
              <button
                type="button"
                role="radio"
                aria-checked={workspace.zip}
                onClick={() => !workspace.zip && saveSubmission(true)}
              >
                {t("lms.zip")}
              </button>
            </span>
            {workspace.zip && (
              <UiInput
                aria-label={t("lms.zipFileName")}
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
      {draft && (
        <p className="lmsWorkspaceHint">{t("lms.filesUploadToTheLMS")}</p>
      )}
      {lost && !closed && !workspace && (
        <p className="lmsWorkspaceHint" role="status">
          {t("lms.theWorkspaceFolderNameIs", { name: lost })}
        </p>
      )}
      <footer>
        {submit && (
          // Opening the page reads nothing local, so only the draft upload waits.
          <button
            type="button"
            className={closed ? "lmsTextLink" : "lmsSubmit"}
            disabled={busy && draft}
            onClick={() => (draft ? void prepare() : submit())}
          >
            {t(draft ? "lms.prepareInLMS" : "lms.viewInLMS")}{" "}
            <LmsIcon name="external" size={13} />
          </button>
        )}
        {submit && draft && (
          // Opens the page only, so it works whatever state the folder is in.
          <button
            type="button"
            className="lmsTextLink"
            onClick={() => submit()}
          >
            {t("lms.viewInLMS")} <LmsIcon name="external" size={13} />
          </button>
        )}
        {view && !workspace && (
          <button
            type="button"
            className="lmsTextLink"
            disabled={busy}
            onClick={() =>
              void run(async () => {
                const created = await createWorkspace(assignment.id);
                setMissing(created.missing.map((file) => file.name));
                await load();
              })
            }
          >
            {t(busy ? "lms.creatingWorkspace" : "lms.createWorkspace")}
          </button>
        )}
      </footer>
      {missing.length > 0 && (
        <p className="lmsWorkspaceError" role="status">
          {t("lms.attachmentsNotDownloaded", { names: missing.join(", ") })}
        </p>
      )}
      {error && (
        <p className="lmsWorkspaceError" role="alert">
          {error}
        </p>
      )}
    </>
  );
}
