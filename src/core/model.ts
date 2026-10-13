import { COURSE_FILE_POLICY } from "./course-file-policy";
import { termForDate } from "./academic";
import type {
  Localized,
  Quarter,
  ScheduledTask,
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

export const REFERENCE_REFRESH_TASK_ID = "builtin:reference-subscriptions";
export const LMS_BACKGROUND_REFRESH_TASK_ID = "builtin:lms-background-refresh";

export function lmsBackgroundRefreshTask(
  deviceId: string,
  updatedAt = new Date().toISOString(),
): ScheduledTask {
  return {
    id: LMS_BACKGROUND_REFRESH_TASK_ID,
    rev: 1,
    updatedAt,
    updatedBy: deviceId,
    name: "LMS background refresh",
    action: "lms-refresh",
    schedule: "RRULE:FREQ=HOURLY;INTERVAL=4;BYMINUTE=0",
    // Full LMS reads are deliberately opt-in. The task also checks Wi-Fi and
    // foreground activity immediately before starting.
    enabled: false,
  };
}

export function referenceRefreshTask(
  deviceId: string,
  updatedAt = new Date().toISOString(),
): ScheduledTask {
  return {
    id: REFERENCE_REFRESH_TASK_ID,
    rev: 1,
    updatedAt,
    updatedBy: deviceId,
    name: "Reference subscriptions",
    action: "reference-refresh",
    // The action itself enforces the daily interval. Hourly scheduling makes
    // an offline/metered pause retry without requiring a background daemon.
    schedule: "RRULE:FREQ=HOURLY;INTERVAL=1;BYMINUTE=0",
    enabled: true,
  };
}

/** A real first launch is empty; preview/demo data must be opted into elsewhere. */
export function seedWorkspace(): Workspace {
  const selectedTerm = termForDate(new Date());
  const deviceId = newDeviceId();
  return {
    schemaVersion: 3,
    deviceId,
    selectedTerm,
    currentTerm: selectedTerm,
    terms: [selectedTerm],
    courses: [],
    memberships: [],
    assignments: [],
    materials: [],
    events: [],
    planItems: [],
    exceptions: [],
    offerings: [],
    intensiveSchedules: [],
    plans: [],
    academicCalendar: [],
    gpaSummary: [],
    departments: [],
    recommendedCourses: [],
    importantDates: [],
    exams: [],
    scheduleOverrides: [],
    subscriptions: [],
    scheduledTasks: [
      referenceRefreshTask(deviceId),
      lmsBackgroundRefreshTask(deviceId),
    ],
    settings: {
      autoDownloadLimitMb: COURSE_FILE_POLICY.autoDownloadLimitMb.default,
      autoCleanDays: COURSE_FILE_POLICY.retentionDays.default,
      appearance: "system",
      agentAccess: { enabled: false, lastWrites: [] },
      density: "comfortable",
      locale: "en",
      restoreSchoolPage: false,
      schoolServiceWidth: 168,
      schoolWebZoom: 1,
      showGrades: false,
      siteConsents: { portal: false, lms: false, syllabus: false },
      notifications: {
        classes: true,
        assignments: true,
        localPermission: "default",
      },
      cloud: {
        provider: "none",
        clientId: "",
        redirectUri: "http://127.0.0.1:42813/oauth/callback",
        conflict: "resolve-manually",
      },
      subscriptionRefresh: {
        frequency: "daily",
        allowCellular: false,
      },
      customization: {
        background: "",
        sites: {
          portal: { css: "", js: "", enabled: false },
          lms: { css: "", js: "", enabled: false },
          syllabus: { css: "", js: "", enabled: false },
        },
        hooks: [],
      },
    },
    academicProfile: {
      name: "Personal program",
      gradePath: [],
      years: [],
    },
    importStatus: { school: "not-configured" },
    syncStatus: {
      state: "needs-configuration",
      message: "Cloud sync is off.",
    },
  };
}
