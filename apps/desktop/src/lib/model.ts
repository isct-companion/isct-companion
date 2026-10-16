import { EMPTY_WORKSPACE } from "../generated/persistence";
import { termForDate } from "./academic";
import type {
  Localized,
  Quarter,
  Settings,
  Workspace,
} from "../generated/persistence";

export type * from "../generated/persistence";

export type Lang = "ja" | "en";
export type TermId = `${number}-${Quarter}`;

/** Language for school text; independent of the UI language unless left on auto. */
export function contentLanguage(
  settings: Pick<Settings, "locale" | "contentLanguage">,
): Lang {
  const chosen = settings.contentLanguage;
  if (chosen === "ja" || chosen === "en") return chosen;
  return settings.locale === "en" ? "en" : "ja";
}

/** Pick stored school text without synthesizing a translation. */
export function pickLocalized(
  value: Localized | undefined,
  locale: Settings["locale"] | string,
  fallback = "",
): string {
  const order: Lang[] =
    locale === "ja" || locale === "zh" ? ["ja", "en"] : ["en", "ja"];
  for (const lang of order) {
    const candidate = value?.[lang]?.trim();
    if (candidate) return candidate;
  }
  return fallback;
}

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

/** Core supplies every default; the shell only fills launch-local identity and time. */
export function seedWorkspace(): Workspace {
  const workspace = structuredClone(EMPTY_WORKSPACE);
  const term = termForDate(new Date());
  const deviceId = newDeviceId();
  workspace.deviceId = deviceId;
  workspace.selectedTerm = term;
  workspace.currentTerm = term;
  workspace.terms = [term];
  const updatedAt = new Date().toISOString();
  for (const task of workspace.scheduledTasks) {
    task.updatedAt = updatedAt;
    task.updatedBy = deviceId;
  }
  return workspace;
}
