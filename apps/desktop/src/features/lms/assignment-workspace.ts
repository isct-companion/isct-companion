import { t } from "../../i18n";
import { errorCode, invoke } from "../../platform/native-error";
import type { CourseFolder } from "./files";

/** A per-assignment folder (desktop only); see `apps/desktop/src-tauri/src/workspaces.rs`. */
export type AssignmentWorkspace = {
  path: string;
  name: string;
  /** Names directly inside `submit/`. */
  submitFiles: string[];
  zip: boolean;
  zipName: string;
};

/** Core's main action for an assignment: open its LMS page, or upload `submit/` as the draft first. */
export type HandInView = {
  workspace: AssignmentWorkspace | null;
  handIn: "openLms" | "prepareDraft";
};

/** `closed`: the assignment is in `TaskView.closedIds`. */
export const findWorkspace = (
  context: CourseFolder,
  assignment: string,
  closed: boolean,
) =>
  invoke<HandInView>("assignment_workspace", { context, assignment, closed });

/** The workspace folder was deleted outside the app. */
export const workspaceMissing = (error: unknown) =>
  errorCode(error) === "workspace-folder-missing";

/** An attachment Core could not download into a new workspace. */
export type MissingAttachment = { name: string; reason: string };

/**
 * Core downloads the attachments, writes `ASSIGNMENT.md` and creates the
 * folder once; attachments it could not fetch come back as `missing`.
 */
export const createWorkspace = (assignment: string) =>
  invoke<{ workspace: AssignmentWorkspace; missing: MissingAttachment[] }>(
    "create_assignment_workspace",
    { assignment },
  );

export const openWorkspace = (path: string, target: "finder" | "vscode") =>
  invoke<void>("open_workspace", { path, target });
export const setWorkspaceSubmission = (
  path: string,
  zip: boolean,
  zipName: string,
) =>
  invoke<AssignmentWorkspace>("set_workspace_submission", {
    path,
    zip,
    zipName,
  });
/** Queue these files for the assignment's official draft upload area. */
export const prepareSubmission = (
  path: string,
  assignment: string,
  url: string,
) =>
  invoke<{ workspace: AssignmentWorkspace; url: string }>(
    "prepare_workspace_submission",
    {
      path,
      assignment,
      url,
      messages: {
        waiting: t("failure.preparingYourLMSUpload"),
        queued: t("failure.filesSentToTheUpload"),
        failed: t("failure.theUploadAreaIsNot"),
      },
    },
  );
export const deleteWorkspace = (path: string) =>
  invoke<void>("delete_workspace", { path });
/** Remove workspaces unchanged for a month; returns how many. */
export const cleanWorkspaces = () => invoke<number>("clean_workspaces");
