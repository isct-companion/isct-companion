import { EMPTY_WORKSPACE } from "../generated/persistence";
import type { Quarter, Workspace } from "../generated/persistence";

export type * from "../generated/persistence";

export type TermId = `${number}-${Quarter}`;

function newDeviceId() {
  return (
    globalThis.crypto?.randomUUID?.() ??
    `device-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`
  );
}

export const REFERENCE_REFRESH_TASK_ID = EMPTY_WORKSPACE.scheduledTasks.find(
  (task) => task.action === "reference-refresh",
)!.id;
export const LMS_BACKGROUND_REFRESH_TASK_ID =
  EMPTY_WORKSPACE.scheduledTasks.find(
    (task) => task.action === "lms-refresh",
  )!.id;

/** Core supplies every default, the term included; the shell only fills
 * launch-local identity and time. */
export function seedWorkspace(): Workspace {
  const workspace = structuredClone(EMPTY_WORKSPACE);
  const deviceId = newDeviceId();
  workspace.deviceId = deviceId;
  const updatedAt = new Date().toISOString();
  for (const task of workspace.scheduledTasks) {
    task.updatedAt = updatedAt;
    task.updatedBy = deviceId;
  }
  return workspace;
}
