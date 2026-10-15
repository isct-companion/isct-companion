import type { Platform } from "./capabilities";
export type AppAction =
  | "view"
  | "change-local-preference"
  | "export-data"
  | "edit-academic-data"
  | "edit-calendar-advanced"
  | "customize-school-site"
  | "configure-unattended-schedule"
  | "resolve-cloud-manually"
  | "import-school"
  | "push-cloud";
export type ActionDecision = { allowed: boolean; reason: string };
export function can(action: AppAction, platform: Platform): ActionDecision {
  if (
    platform === "browser-preview" &&
    (action === "edit-academic-data" ||
      action === "import-school" ||
      action === "push-cloud")
  )
    return {
      allowed: false,
      reason:
        "The browser preview is read-only for native data operations. Use the desktop app.",
    };
  if (
    platform !== "native-desktop" &&
    (action === "edit-calendar-advanced" ||
      action === "customize-school-site" ||
      action === "configure-unattended-schedule" ||
      action === "resolve-cloud-manually")
  )
    return {
      allowed: false,
      reason: "This advanced workflow is available in the desktop app.",
    };
  return { allowed: true, reason: "Available on this platform." };
}
export function requireAction(action: AppAction, platform: Platform): void {
  const decision = can(action, platform);
  if (!decision.allowed) throw new Error(decision.reason);
}
export async function performAction<T>(
  action: AppAction,
  platform: Platform,
  operation: () => T | Promise<T>,
): Promise<T> {
  requireAction(action, platform);
  return operation();
}
