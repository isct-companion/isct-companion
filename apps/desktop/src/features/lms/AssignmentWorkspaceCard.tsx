import { useCallback, useEffect, useRef, useState } from "react";
import { useEventListener } from "usehooks-ts";
import { t } from "../../i18n";
import type { AssignmentState } from "../../generated/persistence";
import { confirmAction } from "../../components/confirm";
import { useSubmissionGuard } from "../../components/use-submission-guard";
import { UiInput } from "../../components/ui";
import { FileTile, LmsIcon } from "./LmsIcon";
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
  assignment,
  state,
  dueAt,
  submit,
}: {
  assignment: { id: string };
  state: AssignmentState | undefined;
  dueAt?: string;
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
  const request = useRef(0);
  const load = useCallback(async () => {
    const generation = ++request.current;
    const next = await findWorkspace(assignment.id).catch(() => undefined);
    if (generation !== request.current) return;
    if (!next) {
      setView(undefined);
      return;
    }
    if (next.workspace) setLost(undefined);
    else if (shown.current) setLost(shown.current.name);
    shown.current = next.workspace;
    setView(next);
  }, [assignment.id, state]);
  useEffect(() => {
    void load();
    return () => {
      request.current += 1;
    };
  }, [load]);
  useEffect(() => {
    if (!dueAt) return;
    const due = Date.parse(dueAt);
    if (!(due > Date.now())) return;
    let timer: ReturnType<typeof setTimeout>;
    const arm = () => {
      const delay = due - Date.now();
      if (delay <= 0) void load();
      else timer = setTimeout(arm, Math.min(delay, 2_147_483_647));
    };
    arm();
    return () => clearTimeout(timer);
  }, [dueAt, load]);
  // Folders are also edited in Finder and VS Code; look again on return.
  useEventListener("focus", () => void load());
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
      if (!workspace) return;
      const prepared = await prepareSubmission(workspace.path, assignment.id);
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
      {lost && !workspace && (
        <p className="lmsWorkspaceHint" role="status">
          {t("lms.theWorkspaceFolderNameIs", { name: lost })}
        </p>
      )}
      <footer>
        {submit && draft && (
          <button
            type="button"
            className="lmsSubmit"
            disabled={busy}
            onClick={() => void prepare()}
          >
            {t("lms.prepareInLMS")} <LmsIcon name="external" size={13} />
          </button>
        )}
        {submit && (
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
