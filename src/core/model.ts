import { COURSE_FILE_POLICY } from "./course-file-policy";
import type {
  Localized,
  Quarter,
  Settings,
  Workspace,
} from "../generated/persistence";

export type * from "../generated/persistence";

export type Lang = "ja" | "en";
export type TermId = `${number}-${Quarter}`;

/** Pick stored school text without synthesizing a translation. */
export function pickLocalized(
  value: Localized | undefined,
  locale: Settings["locale"] | string,
  fallback = "",
): string {
  const order: Lang[] = locale === "ja" ? ["ja", "en"] : ["en", "ja"];
  for (const lang of order) {
    const candidate = value?.[lang]?.trim();
    if (candidate) return candidate;
  }
  return fallback;
}

function defaultTerm(now = new Date()): TermId {
  const month = now.getMonth() + 1;
  const quarter: Quarter =
    month >= 12 || month <= 1
      ? "4Q"
      : month >= 10
        ? "3Q"
        : month >= 6
          ? "2Q"
          : "1Q";
  const academicYear = month <= 1 ? now.getFullYear() - 1 : now.getFullYear();
  return `${academicYear}-${quarter}`;
}

function newDeviceId() {
  return (
    globalThis.crypto?.randomUUID?.() ??
    `device-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`
  );
}

/** A real first launch is empty; preview/demo data must be opted into elsewhere. */
export function seedWorkspace(): Workspace {
  const selectedTerm = defaultTerm();
  return {
    schemaVersion: 3,
    deviceId: newDeviceId(),
    selectedTerm,
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
    subscriptions: [],
    scheduledTasks: [],
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
        frequency: "launch",
        conflict: "resolve-manually",
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
      message: "Choose a provider and add its OAuth client ID.",
    },
  };
}
