import { t } from "../../i18n";
import { errorCode, invoke } from "../../platform/native-error";
import { openMaterial, type CourseFolder } from "./files";
import type { AssignmentAttachment } from "../../platform/core-query";

/** A per-assignment folder (desktop only); see `apps/desktop/src-tauri/src/workspaces.rs`. */
export type AssignmentWorkspace = {
  path: string;
  name: string;
  /** Names directly inside `submit/`. */
  submitFiles: string[];
  zip: boolean;
  zipName: string;
};

/** Description HTML as plain text that keeps its paragraphs and list items. */
function plainText(html: string): string {
  const body = new DOMParser().parseFromString(html, "text/html").body;
  body
    .querySelectorAll("script,style,iframe,object,embed,form")
    .forEach((node) => node.remove());
  body.querySelectorAll("br").forEach((node) => node.replaceWith("\n"));
  body.querySelectorAll("li").forEach((node) => node.prepend("- "));
  body
    .querySelectorAll("p,div,li,h1,h2,h3,h4,h5,h6,tr,pre")
    .forEach((node) => node.append("\n"));
  return (body.textContent ?? "")
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

/** `ASSIGNMENT.md`: what an editor or AI agent opened on the folder needs to know. */
export function assignmentBrief(a: {
  title: string;
  course: string;
  due?: string;
  url?: string;
  html: string;
  attachments: string[];
}): string {
  const facts = [
    `- Course: ${a.course}`,
    a.due && `- Due: ${a.due}`,
    a.url && `- LMS: ${a.url}`,
    "- Hand in: put the files to submit in `submit/`",
  ].filter(Boolean);
  const text = plainText(a.html);
  return (
    [
      `# ${a.title}`,
      facts.join("\n"),
      text,
      a.attachments.length
        ? `## Attachments\n\n${a.attachments.map((name) => `- ${name}`).join("\n")}`
        : "",
    ]
      .filter(Boolean)
      .join("\n\n") + "\n"
  );
}

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

/** Download the attachments into the course folder, then build the workspace from them. */
export async function createWorkspace(
  context: CourseFolder,
  assignment: { id: string; title: string },
  files: readonly AssignmentAttachment[],
  brief: (attachments: string[]) => string,
): Promise<AssignmentWorkspace> {
  const attachments = [];
  for (const file of files) {
    const material = await openMaterial(context, file.url, { download: true });
    if (material.state !== "ready")
      throw new Error(
        `${file.name}: ${material.state === "failed" ? material.reason : material.state}`,
      );
    attachments.push({ path: material.file.path, name: file.name });
  }
  return invoke("create_assignment_workspace", {
    context,
    assignment: assignment.id,
    title: assignment.title,
    brief: brief(attachments.map((a) => a.name)),
    attachments,
  });
}

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
