import { t } from "../../i18n";
import { invoke } from "@tauri-apps/api/core";
import { openMaterial, type CourseFolder } from "./files";

/** A per-assignment folder (desktop only); see `apps/desktop/src-tauri/src/workspaces.rs`. */
export type AssignmentWorkspace = {
  path: string;
  name: string;
  /** Names directly inside `submit/`. */
  submitFiles: string[];
  zip: boolean;
  zipName: string;
};

/** LMS files linked from an assignment's description. */
export function assignmentAttachments(
  html: string,
): { url: string; name: string }[] {
  const page = new DOMParser().parseFromString(html, "text/html");
  const seen = new Set<string>();
  return [
    ...page.querySelectorAll<HTMLAnchorElement>('a[href*="/pluginfile.php/"]'),
  ].flatMap((link) => {
    const url = link.getAttribute("href") ?? "";
    if (seen.has(url)) return [];
    try {
      const parsed = new URL(url);
      if (
        parsed.protocol !== "https:" ||
        parsed.hostname !== "lms.s.isct.ac.jp"
      )
        return [];
      seen.add(url);
      const name =
        decodeURIComponent(parsed.pathname.split("/").pop() || "") ||
        link.textContent?.trim() ||
        "attachment";
      return [{ url, name }];
    } catch {
      return [];
    }
  });
}

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

export const findWorkspace = (context: CourseFolder, assignment: string) =>
  invoke<AssignmentWorkspace | null>("assignment_workspace", {
    context,
    assignment,
  });

/** Download the attachments into the course folder, then build the workspace from them. */
export async function createWorkspace(
  context: CourseFolder,
  assignment: { id: string; title: string },
  html: string,
  brief: (attachments: string[]) => string,
): Promise<AssignmentWorkspace> {
  const attachments = [];
  for (const file of assignmentAttachments(html)) {
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
        waiting: t("Preparing your LMS upload…"),
        queued: t(
          "Files sent to the upload area. Wait for LMS to finish, review the files, then submit yourself.",
        ),
        failed: t("The upload area is not available. Choose the files in LMS."),
      },
    },
  );
export const deleteWorkspace = (path: string) =>
  invoke<void>("delete_workspace", { path });
/** Remove workspaces unchanged for a month; returns how many. */
export const cleanWorkspaces = () => invoke<number>("clean_workspaces");
