import type { AcademicProfile } from "./academic-calendar";

export type Lang = "ja" | "en";
export type Localized = Partial<Record<Lang, string>>;
export type Quarter = "1Q" | "2Q" | "3Q" | "4Q";
export type TermId = `${number}-${Quarter}`;
export type CreditSplit = {
  lecture: number;
  exercise: number;
  lab: number;
};
export type InstructionLanguage = "ja" | "en" | "ja-en" | "other";

export type SyncMeta = {
  id: string;
  rev: number;
  updatedAt: string;
  updatedBy: string;
  deletedAt?: string;
};

export type SourceRef = {
  provider:
    | "sample"
    | "school"
    | "syllabus"
    | "lms"
    | "kyomu"
    | "manual"
    | "import"
    | "mcp";
  externalId?: string;
  importRunId?: string;
  fetchedAt?: string;
};

export type Meeting = {
  days: number[];
  periods: number[];
  startTime: string;
  endTime: string;
  room: string;
  roomNames?: Localized;
  validFrom?: string;
  validTo?: string;
};

/** A person's timetable row. `title` and `instructor` are manual fallbacks. */
export type Course = SyncMeta & {
  kind: "course";
  termId: TermId;
  offeringId?: string;
  names?: { title: Localized; instructor: Localized };
  code: string;
  title: string;
  instructor: string;
  credits: number;
  creditSplit: CreditSplit;
  instructionLanguage: InstructionLanguage;
  color: string;
  meetings: Meeting[];
  source: SourceRef;
  overrides?: Partial<Pick<Course, "title" | "instructor" | "color">>;
  url?: string;
};

export type Membership = SyncMeta & {
  kind: "membership";
  termId: TermId;
  courseId: string;
  status: "declared" | "registered" | "lms-member" | "completed" | "withdrawn";
  evidence: SourceRef;
};

export type Assignment = SyncMeta & {
  kind: "assignment";
  courseId: string;
  title: string;
  details: string;
  dueAt: string;
  sourceUrl?: string;
  lmsCmid?: number;
  lmsCourseId?: number;
  submission?: "unknown" | "open" | "submitted" | "graded";
  /** Transitional UI field; Task 3 moves all assignment rendering to `submission`. */
  submissionStatus?:
    "unknown" | "draft" | "submitted" | "late" | "not-submitted";
  hidden?: boolean;
  completedAt?: string;
  source: SourceRef;
};

export type Material = SyncMeta & {
  kind: "material";
  courseId: string;
  session: number;
  title: string;
  type: "pdf" | "link" | "video";
  cmid?: number;
  lmsCourseId?: number;
  sectionId?: number;
  sectionNumber?: number;
  sectionTitle?: string;
  module?: string;
  visible?: boolean;
  bodyHtml?: string;
  resourceUrl?: string;
  url?: string;
  source: SourceRef;
};

export type PersonalEvent = SyncMeta & {
  kind: "event";
  termId: TermId;
  title: string;
  room: string;
  date?: string;
  days?: number[];
  startTime: string;
  endTime: string;
  blocking: boolean;
};

export type PlanItem = SyncMeta & {
  kind: "plan-item";
  termId: TermId;
  offeringId?: string;
  courseId?: string;
  state: "candidate" | "planned";
};

export type CalendarException = SyncMeta & {
  kind: "exception";
  date: string;
  type: "no-class" | "replacement" | "break";
  replacementDay?: number;
  label: string;
};

export type ScheduleEntry =
  | {
      kind: "weekly";
      day: 1 | 2 | 3 | 4 | 5 | 6 | 7;
      periods: [number, number];
      room?: Localized;
    }
  | {
      kind:
        | "intensive"
        | "seminar"
        | "internship"
        | "on-demand"
        | "tba"
        | "registration-only";
      room?: Localized;
    };

export type Offering = SyncMeta & {
  year: number;
  quarters: Quarter[];
  code: string;
  classCode?: string;
  unitCode?: string;
  title: Localized;
  instructors: Localized[];
  instructionLanguage: InstructionLanguage;
  format: {
    kinds: ("lecture" | "exercise" | "experiment")[];
    delivery?: "face-to-face" | "livestream" | "blended" | "other";
  };
  media: Localized;
  classLabel?: string;
  credits: CreditSplit;
  schedule: ScheduleEntry[];
  academicUnit: Localized;
  intensive: boolean;
  syllabusUrl: string;
  sourceUpdated?: string;
  fetchedAt: Localized;
};

export type IntensiveSchedule = SyncMeta & {
  courseId: string;
  from: string;
  to: string;
  periods: [number, number];
  days?: { date: string; periods: [number, number] | null }[];
  room?: string;
};

export type Plan = SyncMeta & {
  title: string;
  courseId?: string;
  room?: string;
  start: string;
  duration: number;
  rrule?: string;
  exdates: string[];
  reminder?: number;
};

export type AcademicCalendarDay = SyncMeta & {
  academicYear?: number;
  quarter?: Quarter;
  teachingRanges?: { start: string; end: string }[];
  examRanges?: { start: string; end: string }[];
  date: string;
  type:
    "holiday" | "no-class" | "replacement" | "quarter-start" | "quarter-end";
  replacementDay?: number;
  label: Localized;
};

export type GpaSummary = SyncMeta & {
  scope: "all" | "liberal-arts" | "major";
  gpa?: number;
  gpt?: number;
  earnedCredits: number;
  fetchedAt: string;
};

export type Department = SyncMeta & {
  admissionYear: number;
  name: Localized;
  syllabusScope: string;
  unitCode: string;
  guidePdf: string;
};

export type RecommendedCourse = SyncMeta & {
  /** The admission cohort whose official study guide defines this row. */
  admissionYear: number;
  departmentId: string;
  category: Localized;
  level: number;
  code: string;
  group: string;
  mark?: "required" | "recommended";
  title: Localized;
  credits?: CreditSplit;
  sourcePages?: number[];
  sourceKind?: "curriculum-map" | "standard-plan" | "course-table";
  note?: string;
};

export type Subscription = SyncMeta & {
  dataUrl?: string;
  url: string;
  kind: "calendar" | "departments";
  enabled: boolean;
  fetchedAt?: string;
  lastError?: string;
};

export type ScheduledTask = SyncMeta & {
  name: string;
  action: "school-refresh" | "export" | "hook";
  hookId?: string;
  schedule: "launch" | string;
  enabled: boolean;
  /** Stable scheduled occurrence claimed before its side effect begins. */
  lastOccurrenceAt?: string;
  lastRunAt?: string;
  lastResult?: string;
};

export type Settings = {
  departmentId?: string;
  lmsBilingualFilter?: boolean;
  autoSaveCourseFiles?: boolean;
  agentAccess?: {
    enabled: boolean;
    lastWrites?: { at: string; tool: string }[];
  };
  appearance: "system" | "light" | "dark";
  density: "comfortable" | "compact";
  textSize?: "small" | "medium" | "large";
  locale: "en" | "ja" | "zh";
  notifications: {
    classes: boolean;
    assignments: boolean;
    localPermission: "default" | "granted" | "denied";
  };
  cloud: {
    provider: "none" | "google-drive" | "onedrive";
    clientId: string;
    redirectUri?: string;
    frequency: "manual" | "launch" | "hourly" | "daily";
    conflict: "keep-current" | "use-latest" | "resolve-manually";
  };
  customization?: {
    background: string;
    sites: Record<
      "portal" | "lms" | "syllabus",
      { css: string; js: string; enabled: boolean }
    >;
    hooks: {
      id: string;
      name: string;
      action: "school-sync" | "cloud-sync" | "export";
      schedule: "manual" | "launch" | "hourly";
      script: string;
      enabled: boolean;
    }[];
  };
};

export type Workspace = {
  schemaVersion: 3;
  deviceId: string;
  selectedTerm: TermId;
  terms?: TermId[];
  courses: Course[];
  memberships: Membership[];
  assignments: Assignment[];
  materials: Material[];
  events: PersonalEvent[];
  planItems: PlanItem[];
  exceptions: CalendarException[];
  offerings: Offering[];
  intensiveSchedules: IntensiveSchedule[];
  plans: Plan[];
  academicCalendar: AcademicCalendarDay[];
  gpaSummary: GpaSummary[];
  departments: Department[];
  recommendedCourses: RecommendedCourse[];
  subscriptions: Subscription[];
  scheduledTasks: ScheduledTask[];
  settings: Settings;
  academicProfile: AcademicProfile;
  logs?: {
    at: string;
    source: "school" | "cloud";
    phase: string;
    state: string;
    message: string;
    retryable: boolean;
  }[];
  importStatus: {
    school: "not-configured" | "connected" | "reconnect-required" | "error";
    lastImportedAt?: string;
    message?: string;
    lastResult?: "fresh" | "cached" | "reconnect-required" | "error";
    /** Last time the timetable was compared with LMS (TTL starts here). */
    timetableReviewedAt?: string;
  };
  syncStatus: {
    state: "idle" | "needs-configuration" | "syncing" | "error";
    lastSyncedAt?: string;
    message?: string;
  };
};

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
      autoSaveCourseFiles: false,
      appearance: "system",
      agentAccess: { enabled: false, lastWrites: [] },
      density: "comfortable",
      locale: "en",
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
