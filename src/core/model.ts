import type { AcademicProfile } from "./academic-calendar";

export type Quarter = "1Q" | "2Q" | "3Q" | "4Q";
export type TermId = `${number}-${Quarter}`;
export type SyncMeta = {
  id: string;
  rev: number;
  updatedAt: string;
  updatedBy: string;
  deletedAt?: string;
};
export type SourceRef = {
  provider: "sample" | "school" | "syllabus" | "manual";
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
  validFrom?: string;
  validTo?: string;
};
export type Course = SyncMeta & {
  kind: "course";
  termId: TermId;
  code: string;
  title: string;
  instructor: string;
  credits: number;
  language: string;
  color: string;
  meetings: Meeting[];
  source: SourceRef;
  overrides?: Partial<Pick<Course, "title" | "instructor" | "color">>;
  syllabusSource?: Pick<Course, "title" | "instructor" | "language">;
  url?: string;
};
export type Membership = SyncMeta & {
  kind: "membership";
  termId: TermId;
  courseId: string;
  status: "enrolled" | "lms-member" | "historical";
  evidence: SourceRef;
};
export type Assignment = SyncMeta & {
  kind: "assignment";
  courseId: string;
  title: string;
  details: string;
  dueAt: string;
  sourceUrl?: string;
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
export type DraftSelection = SyncMeta & {
  kind: "draft";
  termId: TermId;
  courseId: string;
};
export type CalendarException = SyncMeta & {
  kind: "exception";
  date: string;
  type: "no-class" | "replacement" | "break";
  replacementDay?: number;
  label: string;
};
export type Settings = {
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
    appCss: string;
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
  schemaVersion: 2;
  deviceId: string;
  selectedTerm: TermId;
  terms?: TermId[];
  courses: Course[];
  memberships: Membership[];
  assignments: Assignment[];
  materials: Material[];
  events: PersonalEvent[];
  drafts: DraftSelection[];
  exceptions: CalendarException[];
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
  };
  syncStatus: {
    state: "idle" | "needs-configuration" | "syncing" | "error";
    lastSyncedAt?: string;
    message?: string;
  };
};

const now = "2026-06-16T03:00:00.000Z",
  deviceId = "sample-device";
const meta = (id: string, rev = 1) => ({
  id,
  rev,
  updatedAt: now,
  updatedBy: deviceId,
});
export function seedWorkspace(): Workspace {
  const course = (
    id: string,
    termId: TermId,
    code: string,
    title: string,
    instructor: string,
    credits: number,
    color: string,
    days: number[],
    periods: number[],
    startTime: string,
    endTime: string,
    room: string,
  ): Course => ({
    ...meta(id),
    kind: "course",
    termId,
    code,
    title,
    instructor,
    credits,
    language: "Japanese",
    color,
    meetings: [{ days, periods, startTime, endTime, room }],
    source: { provider: "sample" },
  });
  const courses: Course[] = [
    course(
      "linear",
      "2026-2Q",
      "MTH.A202",
      "線形代数学 第二",
      "Haruka Mori",
      2,
      "#7687a5",
      [1, 4],
      [1, 2],
      "08:50",
      "10:30",
      "S423",
    ),
    course(
      "parallel",
      "2026-2Q",
      "CSC.T354",
      "並列プログラミング",
      "Suguru Saito / Youmi Ma",
      2,
      "#355ba7",
      [2, 5],
      [3, 4],
      "10:45",
      "12:25",
      "TBD",
    ),
    course(
      "quantum",
      "2026-2Q",
      "PHY.Q207",
      "量子力学 基礎",
      "Kei Nakajima",
      2,
      "#806a9d",
      [3],
      [3, 4],
      "10:45",
      "12:25",
      "TBD",
    ),
    course(
      "algorithms",
      "2026-2Q",
      "CSC.T262",
      "データ構造とアルゴリズム",
      "Akira Ito",
      2,
      "#5c7d93",
      [1],
      [5, 6],
      "13:30",
      "15:10",
      "W831",
    ),
    course(
      "english",
      "2026-2Q",
      "LAJ.A201",
      "アカデミック英語",
      "Emma Wilson",
      1,
      "#668879",
      [4],
      [7, 8],
      "15:25",
      "17:05",
      "S221",
    ),
    course(
      "architecture",
      "2026-3Q",
      "CSC.T363",
      "Computer Architecture",
      "Computer Science faculty",
      2,
      "#355ba7",
      [3],
      [5, 6],
      "13:30",
      "15:10",
      "TBD",
    ),
    course(
      "oop",
      "2026-3Q",
      "CSC.T273",
      "Object-Oriented Programming",
      "Computer Science faculty",
      2,
      "#668879",
      [2],
      [7, 8],
      "15:25",
      "17:05",
      "TBD",
    ),
    course(
      "systems",
      "2026-3Q",
      "CSC.T366",
      "Systems Workshop",
      "Computer Science faculty",
      2,
      "#806a9d",
      [3],
      [5, 6],
      "13:30",
      "15:10",
      "W934",
    ),
    course(
      "stats",
      "2026-1Q",
      "MTH.C203",
      "確率論と統計",
      "Mathematics faculty",
      2,
      "#7687a5",
      [1],
      [1, 2],
      "08:50",
      "10:30",
      "S421",
    ),
  ];
  return {
    schemaVersion: 2,
    deviceId,
    selectedTerm: "2026-2Q",
    courses,
    memberships: courses
      .filter((c) => c.termId !== "2026-3Q")
      .map((c) => ({
        ...meta(`membership-${c.id}`),
        kind: "membership",
        termId: c.termId,
        courseId: c.id,
        status: c.termId === "2026-1Q" ? "historical" : "enrolled",
        evidence: { provider: "sample" },
      })),
    assignments: [
      {
        ...meta("a1"),
        kind: "assignment",
        courseId: "linear",
        title: "Problem set 3",
        details: "Complete exercises 3.1–3.8 and upload one PDF.",
        dueAt: "2026-06-15T14:59:00.000Z",
        sourceUrl: "https://t2schola.titech.ac.jp/",
        source: { provider: "sample" },
      },
      {
        ...meta("a2"),
        kind: "assignment",
        courseId: "quantum",
        title: "Pre-class quiz 4",
        details: "Review the lecture notes before starting.",
        dueAt: "2026-06-17T14:59:00.000Z",
        sourceUrl: "https://t2schola.titech.ac.jp/",
        source: { provider: "sample" },
      },
      {
        ...meta("a3"),
        kind: "assignment",
        courseId: "parallel",
        title: "OpenMP exercise 2",
        details: "Submit source and a short performance note.",
        dueAt: "2026-06-18T14:59:00.000Z",
        sourceUrl: "https://t2schola.titech.ac.jp/",
        source: { provider: "sample" },
      },
    ],
    materials: [
      {
        ...meta("m1"),
        kind: "material",
        courseId: "parallel",
        session: 4,
        title: "OpenMP patterns",
        type: "pdf",
        source: { provider: "sample" },
      },
      {
        ...meta("m2"),
        kind: "material",
        courseId: "linear",
        session: 3,
        title: "Vector spaces worksheet",
        type: "pdf",
        source: { provider: "sample" },
      },
    ],
    events: [
      {
        ...meta("event-seminar"),
        kind: "event",
        termId: "2026-2Q",
        title: "研究室ゼミ",
        room: "Lab",
        days: [3],
        startTime: "13:30",
        endTime: "17:05",
        blocking: true,
      },
    ],
    drafts: [
      {
        ...meta("draft-architecture"),
        kind: "draft",
        termId: "2026-3Q",
        courseId: "architecture",
      },
      {
        ...meta("draft-oop"),
        kind: "draft",
        termId: "2026-3Q",
        courseId: "oop",
      },
    ],
    exceptions: [
      {
        ...meta("ex-1"),
        kind: "exception",
        date: "2026-06-16",
        type: "replacement",
        replacementDay: 1,
        label: "Monday timetable",
      },
    ],
    settings: {
      appearance: "system",
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
        appCss: "",
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
      gradePath: [
        { grade: "B1", academicYear: 2024 },
        { grade: "B2", academicYear: 2025 },
        { grade: "B3", academicYear: 2026 },
        { grade: "B4", academicYear: 2027 },
      ],
      years: [2025, 2026, 2027].map((academicYear) => ({
        academicYear,
        quarterStarts: {
          "1Q": `${academicYear}-04-01`,
          "2Q": `${academicYear}-06-01`,
          "3Q": `${academicYear}-10-01`,
          "4Q": `${academicYear}-12-01`,
        },
      })),
    },
    importStatus: {
      school: "not-configured",
      message:
        "Open the official school sites to sign in. Import API access still requires school approval.",
    },
    syncStatus: {
      state: "needs-configuration",
      message: "Choose a provider and add its OAuth client ID.",
    },
  };
}
