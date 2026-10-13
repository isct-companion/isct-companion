import { clockForPeriods } from "../core/user-schedule";
import type { FinderCourse } from "../core/course-query";
import { validSyllabusContent } from "../core/syllabus-content";
import { pickLocalized, type Localized } from "../core/model";
import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import type { SyllabusCourse } from "../core/syllabus";

export type DesktopRuntime = "tauri";
export type NativeTarget = "native-desktop" | "native-mobile";
/** Rust's `std::env::consts::OS` for this build, injected at startup. */
export type NativeOs = "macos" | "ios" | "android" | "windows" | "linux";
export const nativeOs = (): NativeOs | undefined =>
  typeof window === "undefined"
    ? undefined
    : window.__ISCT_COMPANION_NATIVE_OS__;
export type SchoolSite =
  | "science-portal"
  | "science-lms"
  | "tokyo-portal"
  | "tokyo-mail"
  | "kyomu"
  | "library"
  | "tsubame"
  | "syllabus";
/** A file a school page saved to Downloads (download or PDF viewer ↓). */
export type SchoolDownloadEvent = {
  site: SchoolSite;
  state: "started" | "done" | "failed";
  name: string;
  /** Set on `done`: the saved file, accepted by `revealSchoolDownload`. */
  path?: string;
};
export type SchoolAuthProfile = "science" | "tokyo-tech";
export type SchoolCredentials = {
  username: string;
  password: string;
  otpSecret: string;
  matrix: string;
};
export type SchoolCredentialStatus = {
  science: {
    availability:
      | "not-configured"
      | "saved-locked"
      | "available"
      | "missing"
      | "access-denied";
    saved: boolean;
    username: string;
    hasOtp: boolean;
    hasMatrix: boolean;
  };
  tokyoTech: {
    availability:
      | "not-configured"
      | "saved-locked"
      | "available"
      | "missing"
      | "access-denied";
    saved: boolean;
    username: string;
    hasOtp: boolean;
    hasMatrix: boolean;
  };
};
export type SchoolLoginState =
  /** A sign-in step started; `message` names the step. */
  | "progress"
  /** Automation paused on a step the person must finish in the login page. */
  | "needs-user"
  | "connected"
  | "error"
  | "invalid-credentials"
  | "invalid-otp"
  | "invalid-matrix"
  | "locked"
  | "maintenance";
export type SchoolLoginEvent = {
  profile: SchoolAuthProfile;
  state: SchoolLoginState;
  message: string;
};
export type SchoolSessionState =
  "pending" | "active" | "failed" | "not-configured";
/** A school session state changed (sign-in, launch, credential change, or renewal). */
export type SchoolSessionEvent = {
  profile: SchoolAuthProfile;
  state: SchoolSessionState;
  revision: number;
};
export type SchoolSessionSnapshot = {
  revision: number;
  states: {
    science: SchoolSessionState;
    tokyoTech: SchoolSessionState;
  };
};
export type SchoolSessionLifecycle = {
  kind: "launch" | "focus";
  visible: boolean;
};
const isSchoolSessionState = (value: unknown): value is SchoolSessionState =>
  value === "pending" ||
  value === "active" ||
  value === "failed" ||
  value === "not-configured";
/** One extractor result from LMS or one 教務Web page. */
export type SchoolIngestOutcome = {
  site: SchoolSite;
  kind: string;
  state: "ok" | "reconnect-required" | "error";
  changed: number;
  message: string;
};
export type SchoolPullProgress = {
  site: "science-lms" | "kyomu";
  phase: "catalog" | "course" | "material" | "assignment";
  completed: number;
  total: number;
  courseId: number;
  courseTitle: string;
  term: string;
  itemTitle: string;
};
export type SchoolPullOptions = {
  /** Limit the expensive LMS content pass to one Moodle course. */
  lmsCourseId?: number;
};
export type LmsSection = {
  id: number;
  number: number;
  title: string;
  visible: boolean;
  bodyHtml?: string;
};

/** An LMS course row as imported (`lms_courses`). */
export type LmsCourseRecord = {
  id: string;
  lmsId: number;
  offeringId?: string | null;
  shortname: string;
  title: Localized;
  year?: number | null;
  quarters: string[];
  url: string;
  hidden?: boolean;
  sections?: LmsSection[];
  /** Weekly meetings per term, read from the LMS dashboard timetable grid. */
  timetables?: Record<string, { day: number; periods: [number, number] }[]>;
};
/** School-owned rows the workspace does not carry. */
export type SchoolRecords = {
  /** Last complete import per `provider:scope` (ISO time). */
  lastRuns: Record<string, string>;
  lmsCourses: LmsCourseRecord[];
  registrations: {
    id: string;
    offeringId: string;
    code: string;
    title: string;
    courseState?: string;
    declaredOnly?: boolean;
    /** Rows from 科目から (`kind: "declared"`). */
    kind?: string;
    quarters?: number[];
    effectiveCredits?: number;
  }[];
  registrationStates: {
    id: string;
    state: string;
    raw: string;
    savedAt?: string | null;
  }[];
  grades: Record<string, unknown>[];
  gpaByTerm: { id: string; cells: string[] }[];
  requirements: {
    id: string;
    table: number;
    category: string;
    label: string;
    value: string;
  }[];
};
export type KyomuGuideItem = {
  offeringId: string;
  code: string;
  title: string;
  day?: number;
  slot: string;
};
export type SchoolWebBounds = {
  x: number;
  y: number;
  width: number;
  height: number;
};
export type NativeCapabilitySnapshot = {
  schoolImport: boolean;
  schoolWeb: boolean;
  notifications: boolean;
  backgroundSchedule: boolean;
  cloudSync: boolean;
  syllabusSearch: boolean;
  matrixOcr: boolean;
};
export type CampusWifiOutcome =
  | "signed-in"
  | "already-online"
  | "needs-account"
  | "rejected"
  | "science-tokyo-x";
export type CampusWifiStatus =
  "signed-in" | "sign-in-required" | "science-tokyo-x" | "other";
/** Sign-in stages in order; "joining" is skipped when already on ScienceTokyo. */
export const CAMPUS_WIFI_STEPS = [
  "joining",
  "opening-portal",
  "signing-in",
  "verifying",
] as const;
export type CampusWifiStep = (typeof CAMPUS_WIFI_STEPS)[number];
export type CloudProvider = "google-drive" | "onedrive" | "icloud";
/** Automatic sync, per device; contract in docs/cloud-sync.md. */
export type CloudSyncProvider = "icloud" | "google-drive";
export type CloudSyncVersion = {
  id: string;
  provider: CloudSyncProvider;
  savedAt: string;
  device: string;
  version?: string;
};
export type CloudSyncTask = {
  id: string;
  direction: "upload" | "pull";
  version: string;
  createdAt: string;
  deliveries: {
    provider: CloudSyncProvider;
    status: "pending" | "running" | "completed";
    attempts: number;
    retryAt: number;
    error?: string;
  }[];
};
export type CloudSyncState = {
  syncing: boolean;
  automatic?: boolean;
  localVersion?: string;
  tasks?: CloudSyncTask[];
  recoveryPoints?: { id: string; version: string; savedAt: string }[];
  providers: {
    provider: CloudSyncProvider;
    enabled: boolean;
    lastSyncedAt?: string;
    syncedVersion?: string;
    error?: string;
  }[];
  /** Earlier cloud copies, newest first. */
  versions: CloudSyncVersion[];
};

/** Native business pull state; login/navigation remain executor details. */
export type PullSnapshot = {
  id: string;
  target:
    | { type: "school"; site: "science-lms" | "kyomu"; courseId: number | null }
    | { type: "reference"; kind: string; indexUrl: string; year: number };
  priority: "automatic" | "manual";
  state:
    | "queued"
    | "checking"
    | "blocked"
    | "running"
    | "applying"
    | "retryWaiting"
    | "succeeded"
    | "failed"
    | "cancelling"
    | "cancelled"
    | "yielding";
  phase: string | null;
  blockedBy: string | null;
  attempt: number;
  waiterCount: number;
  lastError: string | null;
};

/** Prefer executing work when an older automatic request is blocked. */
export function schoolPullForDisplay(
  states: PullSnapshot[],
  site: "science-lms" | "kyomu",
) {
  const active = states.filter(
    (pull) =>
      pull.target.type === "school" &&
      pull.target.site === site &&
      !["succeeded", "failed", "cancelled"].includes(pull.state),
  );
  return (
    active.find((pull) =>
      ["running", "applying", "cancelling"].includes(pull.state),
    ) ??
    active.find((pull) => pull.state !== "blocked") ??
    active[0]
  );
}

export type InvokeNative = (
  command: string,
  payload?: Record<string, unknown>,
) => Promise<unknown>;
export type ListenNative = <T>(
  event: string,
  callback: (payload: T) => void,
) => Promise<() => void>;

export interface DesktopBridge {
  runtime: DesktopRuntime;
  target: NativeTarget;
  /** The build's operating system; see `nativeOs`. */
  os?: NativeOs;
  getRuntimeCapabilities(): Promise<NativeCapabilitySnapshot>;
  resolveSchoolUrl(url: string): Promise<{ site: SchoolSite; url: string }>;
  openExternal(url: string): Promise<void>;
  notify(title: string, body: string): Promise<boolean>;
  requestNotificationPermission(): Promise<NotificationPermission>;
  runScheduledTask(id: string): Promise<string>;
  onWorkspaceChanged(callback: () => void): () => void;
  schoolCredentialStatus(): Promise<SchoolCredentialStatus>;
  saveSchoolCredentials(
    profile: SchoolAuthProfile,
    credentials: SchoolCredentials,
  ): Promise<boolean>;
  deleteSchoolCredentials(profile: SchoolAuthProfile): Promise<boolean>;
  /** Copies the saved Matrix to the system clipboard without exposing it to the WebView. */
  copySchoolMatrix(): Promise<boolean>;
  testSchoolConnection(
    profile: SchoolAuthProfile,
    credentials: SchoolCredentials,
  ): Promise<{ site: SchoolSite; url: string; phase: string }>;
  loginSchool(profile: SchoolAuthProfile): Promise<{
    site: SchoolSite;
    url: string;
    phase: string;
  }>;
  openSchoolSite(
    site: SchoolSite,
    css: string,
    js: string,
    bounds?: SchoolWebBounds,
    targetUrl?: string,
  ): Promise<{
    site: SchoolSite;
    url: string;
    phase: "sessionRestored" | "pageLoaded" | "navigationDispatched";
  }>;
  configureSchoolWebPolicy(policy: {
    restoreAcrossRestarts: boolean;
    idleMinutes: number;
  }): Promise<boolean>;
  /** Position the embedded school webview over a region of the main window (logical px). */
  setSchoolWebBounds?(bounds: SchoolWebBounds): Promise<boolean>;
  /** Hide every embedded school webview (when leaving the Web page). */
  hideSchoolWeb?(): Promise<boolean>;
  reloadSchoolWeb?(site: SchoolSite): Promise<boolean>;
  navigateSchoolWeb?(site: SchoolSite, url: string): Promise<boolean>;
  /** Load the view's previous page as a fresh GET; false when there is none. */
  schoolWebBack?(site: SchoolSite): Promise<boolean>;
  schoolWebForward?(site: SchoolSite): Promise<boolean>;
  setSchoolWebZoom?(site: SchoolSite, scale: number): Promise<boolean>;
  onSchoolWebUrl?(
    callback: (request: { site: SchoolSite; url: string }) => void,
  ): () => void;
  /** A school page asked for a new window showing another service's page. */
  onSchoolWebOpen?(
    callback: (request: { site: SchoolSite; url: string }) => void,
  ): () => void;
  onSchoolLoginStatus?(callback: (event: SchoolLoginEvent) => void): () => void;
  /** What the Wi-Fi interface is on now (macOS builds only). */
  campusWifiStatus?(): Promise<CampusWifiStatus>;
  /**
   * Joins ScienceTokyo if needed and signs in once with the Tokyo Tech account
   * (macOS builds only). Returns "science-tokyo-x" without changing anything
   * unless leaving ScienceTokyoX was confirmed.
   */
  campusWifiConnect?(
    leaveScienceTokyoX: boolean,
    onStep: (step: CampusWifiStep) => void,
  ): Promise<CampusWifiOutcome>;
  onSchoolDownload?(callback: (event: SchoolDownloadEvent) => void): () => void;
  /** Shows a file saved from a school page this session in Finder; false for any other path. */
  revealSchoolDownload?(path: string): Promise<boolean>;
  schoolSessionSnapshot(): Promise<SchoolSessionSnapshot>;
  schoolSessionLifecycle(event: SchoolSessionLifecycle): Promise<void>;
  onSchoolSession(
    callback: (event: SchoolSessionEvent) => void,
  ): Promise<() => void> | (() => void);
  /**
   * Runs the LMS or 教務Web extractor in the hidden school view and waits for
   * the result (one outcome per 教務Web page). Expired sessions are renewed
   * with the saved credentials on the way.
   */
  pullSchool?(
    site: "science-lms" | "kyomu",
    options?: SchoolPullOptions,
  ): Promise<SchoolIngestOutcome[]>;
  getPullState?(): Promise<PullSnapshot[]>;
  onPullState?(callback: (states: PullSnapshot[]) => void): () => void;
  cancelPull?(id: string): Promise<boolean>;
  refreshReferenceSubscriptions?(force: boolean): Promise<string>;
  schoolRecords?(): Promise<SchoolRecords>;
  /** Shows (or clears, with `[]`) the registration queue on 教務Web pages. */
  setKyomuGuide?(items: KyomuGuideItem[]): Promise<boolean>;
  /** An extractor finished while the person browsed LMS / 教務Web. */
  onSchoolIngest?(callback: (outcome: SchoolIngestOutcome) => void): () => void;
  /** Fine-grained progress from a running hidden LMS extraction. */
  onSchoolPullProgress?(
    callback: (progress: SchoolPullProgress) => void,
  ): () => void;
  /** Saves a validated workspace export under app-owned local storage. */
  writeScheduledExport(): Promise<string>;
  /** Asks where to save an exported file; false when the person cancels. */
  saveFile(name: string, contents: string): Promise<boolean>;
  connectCloud(
    provider: CloudProvider,
    clientId: string,
    redirectUri: string,
  ): Promise<boolean>;
  cloudAuthorizationStatus(): Promise<CloudProvider[]>;
  schoolRefreshEvent?(event: {
    kind: "account-active" | "foreground" | "background" | "lecture-opened";
    profile?: SchoolAuthProfile;
    courseId?: number;
    network?: string;
  }): Promise<void>;
  cloudLifecycle?(kind: "foreground" | "background"): Promise<void>;
  cloudSyncState?(): Promise<CloudSyncState>;
  setCloudSyncProvider?(
    provider: CloudSyncProvider,
    enabled: boolean,
  ): Promise<CloudSyncState>;
  syncCloudNow?(direction: "upload" | "pull"): Promise<CloudSyncState>;
  setCloudSyncAutomatic?(automatic: boolean): Promise<CloudSyncState>;
  restoreCloudVersion?(id: string): Promise<CloudSyncState>;
  cloudRecoveryDocument?(id: string): Promise<string>;
  onCloudSyncState?(callback: (state: CloudSyncState) => void): () => void;
  disconnectCloud(provider: CloudProvider): Promise<boolean>;
  cloudCredentialSyncStatus(): Promise<boolean>;
  cloudCredentialBackupStatus(provider: CloudProvider): Promise<boolean>;
  setCloudCredentialSyncPassword(password: string): Promise<boolean>;
  syncCloudCredentials(
    provider: CloudProvider,
    restoreExisting: boolean,
  ): Promise<boolean>;
  clearCloudCredentialSyncPassword(): Promise<boolean>;
  syllabusCourse(url: string, language: "en" | "ja"): Promise<SyllabusCourse>;
  catalogSyncDepartment(
    year: number,
    scopePath: string,
    quarter?: "1Q" | "2Q" | "3Q" | "4Q",
  ): Promise<{
    listed: number;
    synced: number;
    truncated: boolean;
    warnings: string[];
  }>;
  /** Every stored offering of a year, for the course finder's local search. */
  catalogOfferings?(year: number): Promise<FinderCourse[]>;
  /** One official syllabus search (≤100 rows, no paging); rows are stored. */
  catalogQuery?(
    year: number,
    query: { q: string; scope?: string; code?: string; groups?: string[] },
  ): Promise<{ ids: string[]; total: number; truncated: boolean }>;
  /** Fetches detail pages (schedule, credits) for up to 20 stored offerings. */
  catalogSyncOfferings?(ids: string[]): Promise<string[]>;
}

/** Tauri's unlisten throws, or rejects, once the runtime is gone (window closing, tests). */
const quietly = (unlisten: () => unknown) => {
  try {
    void Promise.resolve(unlisten()).catch(() => undefined);
  } catch {
    // Already gone.
  }
};
const requireBoolean = (command: string, value: unknown) => {
  if (typeof value !== "boolean")
    throw new Error(`${command} returned an invalid result.`);
  return value;
};
const requireObject = <T>(command: string, value: unknown): T => {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error(`${command} returned an invalid result.`);
  return value as T;
};
type CatalogItem = {
  id: string;
  year: number;
  quarters: SyllabusCourse["quarter"][];
  code: string;
  title: Localized;
  instructors: Localized[];
  syllabusUrl: string;
};
type CatalogOffering = CatalogItem & {
  credits: { lecture: number; exercise: number; lab: number };
  instructionLanguage: string;
  schedule: {
    kind: string;
    day?: number;
    periods?: [number, number];
    room?: Localized;
  }[];
  sourceUpdated?: string;
};
// Any period range (e.g. 5-8 → 13:30–17:05), not only the standard two-period slots.
const periodTimes = (periods: [number, number]) => {
  const clock = clockForPeriods(periods);
  return [clock.start, clock.end] as const;
};
export const catalogCourse = (
  value: unknown,
  locale: "en" | "ja",
  content?: unknown,
): SyllabusCourse => {
  const row = requireObject<CatalogItem>("catalog offering", value);
  if (
    typeof row.id !== "string" ||
    typeof row.year !== "number" ||
    !Array.isArray(row.quarters) ||
    typeof row.code !== "string" ||
    typeof row.syllabusUrl !== "string"
  )
    throw new Error("catalog offering returned an invalid result.");
  const full = row as CatalogOffering;
  const meetings = Array.isArray(full.schedule)
    ? full.schedule.flatMap((entry) => {
        if (
          entry.kind !== "weekly" ||
          typeof entry.day !== "number" ||
          !Array.isArray(entry.periods)
        )
          return [];
        const [startTime, endTime] = periodTimes(entry.periods);
        return [
          {
            days: [entry.day],
            periods: Array.from(
              { length: entry.periods[1] - entry.periods[0] + 1 },
              (_, index) => entry.periods![0] + index,
            ),
            startTime,
            endTime,
            room: pickLocalized(entry.room, locale),
            roomNames: entry.room,
          },
        ];
      })
    : [];
  return {
    externalId: row.id,
    year: row.year,
    quarter: row.quarters[0] ?? "1Q",
    quarters: row.quarters,
    code: row.code,
    title: pickLocalized(row.title, locale),
    instructor: Array.isArray(row.instructors)
      ? row.instructors
          .map((name) => pickLocalized(name, locale))
          .filter(Boolean)
          .join(" / ")
      : "",
    credits: full.credits
      ? full.credits.lecture + full.credits.exercise + full.credits.lab
      : undefined,
    language: full.instructionLanguage,
    meetings,
    url: row.syllabusUrl,
    updated: full.sourceUpdated,
    content:
      content != null && validSyllabusContent(content) ? content : undefined,
  };
};

export function createTauriDesktopBridge(
  invoke: InvokeNative,
  listen: ListenNative,
  target: NativeTarget = window.__ISCT_COMPANION_NATIVE_TARGET__ ??
    "native-desktop",
): DesktopBridge {
  const invokeCommand = async <T>(
    command: string,
    payload?: Record<string, unknown>,
  ): Promise<T> => {
    try {
      return (await invoke(command, payload)) as T;
    } catch (error) {
      throw error instanceof Error
        ? error
        : new Error(typeof error === "string" ? error : `${command} failed`);
    }
  };
  const result = async <T>(
    command: string,
    payload?: Record<string, unknown>,
  ) => requireObject<T>(command, await invokeCommand(command, payload));
  const booleanCommand = async (
    command: string,
    payload?: Record<string, unknown>,
  ) => requireBoolean(command, await invokeCommand(command, payload));
  return {
    runtime: "tauri",
    target,
    os: nativeOs(),
    getRuntimeCapabilities: () => result("runtime_capabilities"),
    resolveSchoolUrl: (url) => result("resolve_school_url", { url }),
    openExternal: async (url) => {
      await invokeCommand("open_external", { url });
    },
    notify: (title, body) => booleanCommand("notify", { title, body }),
    requestNotificationPermission: async () => {
      const value = await invokeCommand("request_notification_permission");
      if (!matchesNotificationPermission(value))
        throw new Error(
          "request_notification_permission returned an invalid result.",
        );
      return value;
    },
    runScheduledTask: (id) => invokeCommand("run_scheduled_task", { id }),
    onWorkspaceChanged: (callback) => {
      let disposed = false;
      let unlisten: (() => void) | undefined;
      void listen("workspace-changed", () => {
        if (!disposed) callback();
      }).then((fn) => {
        if (disposed) quietly(fn);
        else unlisten = fn;
      });
      return () => {
        disposed = true;
        if (unlisten) quietly(unlisten);
      };
    },
    schoolCredentialStatus: () => result("school_credential_status"),
    campusWifiStatus: () => invokeCommand("campus_wifi_status"),
    campusWifiConnect: (leaveScienceTokyoX, onStep) => {
      const steps = new Channel<CampusWifiStep>();
      steps.onmessage = onStep;
      return invokeCommand("campus_wifi_connect", {
        leaveScienceTokyoX,
        onStep: steps,
      });
    },
    saveSchoolCredentials: (profile, credentials) =>
      booleanCommand("save_school_credentials", { profile, credentials }),
    deleteSchoolCredentials: (profile) =>
      booleanCommand("delete_school_credentials", { profile }),
    copySchoolMatrix: () => booleanCommand("copy_school_matrix"),
    testSchoolConnection: (profile, credentials) =>
      result("test_school_connection", { profile, credentials }),
    loginSchool: (profile) => result("login_school", { profile, bounds: null }),
    schoolSessionSnapshot: async () => {
      const snapshot = await result<SchoolSessionSnapshot>(
        "school_session_snapshot",
      );
      if (
        !Number.isSafeInteger(snapshot.revision) ||
        snapshot.revision < 0 ||
        !snapshot.states ||
        !isSchoolSessionState(snapshot.states.science) ||
        !isSchoolSessionState(snapshot.states.tokyoTech)
      )
        throw new Error("school_session_snapshot returned an invalid result.");
      return snapshot;
    },
    schoolSessionLifecycle: async (event) => {
      await invokeCommand("school_session_lifecycle", event);
    },
    onSchoolSession: async (callback) => {
      let disposed = false;
      const unlisten = await listen<SchoolSessionEvent>(
        "school-session",
        (payload) => {
          if (
            !disposed &&
            payload &&
            (payload.profile === "science" ||
              payload.profile === "tokyo-tech") &&
            isSchoolSessionState(payload.state) &&
            Number.isSafeInteger(payload.revision) &&
            payload.revision >= 0
          )
            callback(payload);
        },
      );
      return () => {
        disposed = true;
        quietly(unlisten);
      };
    },
    openSchoolSite: (site, css, js, bounds, targetUrl) =>
      result("open_school_site", {
        site,
        css,
        js,
        bounds: bounds ?? null,
        targetUrl: targetUrl ?? null,
      }),
    setSchoolWebBounds: (bounds) =>
      booleanCommand("set_school_web_bounds", { bounds }),
    hideSchoolWeb: () => booleanCommand("hide_school_web"),
    reloadSchoolWeb: (site) => booleanCommand("reload_school_web", { site }),
    navigateSchoolWeb: (site, url) =>
      booleanCommand("navigate_school_web", { site, url }),
    schoolWebBack: (site) => booleanCommand("school_web_back", { site }),
    schoolWebForward: (site) => booleanCommand("school_web_forward", { site }),
    setSchoolWebZoom: (site, scale) =>
      booleanCommand("set_school_web_zoom", { site, scale }),
    onSchoolWebUrl: (callback) => {
      let disposed = false;
      let unlisten: (() => void) | undefined;
      void listen<{ site: SchoolSite; url: string }>(
        "school-web-url",
        (payload) => {
          if (
            !disposed &&
            payload &&
            typeof payload.site === "string" &&
            typeof payload.url === "string"
          )
            callback(payload);
        },
      ).then((fn) => {
        if (disposed) quietly(fn);
        else unlisten = fn;
      });
      return () => {
        disposed = true;
        if (unlisten) quietly(unlisten);
      };
    },
    onSchoolWebOpen: (callback) => {
      let disposed = false;
      let unlisten: (() => void) | undefined;
      void listen<{ site: SchoolSite; url: string }>(
        "school-web-open",
        (payload) => {
          if (
            !disposed &&
            payload &&
            typeof payload.site === "string" &&
            typeof payload.url === "string"
          )
            callback(payload);
        },
      ).then((fn) => {
        if (disposed) quietly(fn);
        else unlisten = fn;
      });
      return () => {
        disposed = true;
        if (unlisten) quietly(unlisten);
      };
    },
    onSchoolLoginStatus: (callback) => {
      let disposed = false;
      let unlisten: (() => void) | undefined;
      void listen<SchoolLoginEvent>("school-login-status", (payload) => {
        if (
          !disposed &&
          payload &&
          (payload.profile === "science" || payload.profile === "tokyo-tech") &&
          typeof payload.state === "string" &&
          typeof payload.message === "string"
        )
          callback(payload);
      }).then((fn) => {
        if (disposed) quietly(fn);
        else unlisten = fn;
      });
      return () => {
        disposed = true;
        if (unlisten) quietly(unlisten);
      };
    },
    onSchoolDownload: (callback) => {
      let disposed = false;
      let unlisten: (() => void) | undefined;
      void listen<SchoolDownloadEvent>("school-download", (payload) => {
        if (
          !disposed &&
          payload &&
          typeof payload.site === "string" &&
          (payload.state === "started" ||
            payload.state === "done" ||
            payload.state === "failed") &&
          typeof payload.name === "string" &&
          (payload.path === undefined || typeof payload.path === "string") &&
          (payload.state !== "done" || typeof payload.path === "string")
        )
          callback({
            site: payload.site,
            state: payload.state,
            name: payload.name,
            path: payload.path,
          });
      }).then((fn) => {
        if (disposed) quietly(fn);
        else unlisten = fn;
      });
      return () => {
        disposed = true;
        if (unlisten) quietly(unlisten);
      };
    },
    revealSchoolDownload: (path) =>
      booleanCommand("reveal_school_download", { path }),
    pullSchool: async (site, options) => {
      const value = await invokeCommand("pull_school", {
        site,
        lmsCourseId: options?.lmsCourseId,
      });
      if (!Array.isArray(value))
        throw new Error("pull_school returned an invalid result.");
      return value as SchoolIngestOutcome[];
    },
    getPullState: async () => {
      const value = await invokeCommand("pull_state");
      if (!Array.isArray(value))
        throw new Error("pull_state returned an invalid result.");
      return value as PullSnapshot[];
    },
    onPullState: (callback) => {
      let disposed = false;
      let unlisten: (() => void) | undefined;
      let version = 0;
      void listen<PullSnapshot[]>("pull-state-changed", (payload) => {
        if (!disposed && Array.isArray(payload)) {
          version++;
          callback(payload);
        }
      })
        .then(async (fn) => {
          if (disposed) {
            fn();
            return;
          }
          unlisten = fn;
          // Listen first, then read: fast startup jobs may finish before mount.
          const currentVersion = version;
          const value = await invokeCommand("pull_state");
          if (!disposed && currentVersion === version && Array.isArray(value))
            callback(value as PullSnapshot[]);
        })
        .catch(() => undefined);
      return () => {
        disposed = true;
        try {
          void Promise.resolve(unlisten?.()).catch(() => undefined);
        } catch {
          /* Closing runtime. */
        }
      };
    },
    cancelPull: (id) => booleanCommand("cancel_pull", { id }),
    refreshReferenceSubscriptions: async (force) => {
      const value = await invokeCommand("refresh_reference_subscriptions", {
        force,
      });
      if (typeof value !== "string")
        throw new Error("Reference refresh returned an invalid result.");
      return value;
    },
    schoolRecords: async () => {
      const value = requireObject<Partial<SchoolRecords>>(
        "school_records",
        await invokeCommand("school_records"),
      );
      const rows = <T>(list: unknown): T[] =>
        Array.isArray(list)
          ? (list.filter((row) => row && typeof row === "object") as T[])
          : [];
      const lastRuns =
        value.lastRuns && typeof value.lastRuns === "object"
          ? Object.fromEntries(
              Object.entries(value.lastRuns).filter(
                (entry): entry is [string, string] =>
                  typeof entry[1] === "string",
              ),
            )
          : {};
      return {
        lastRuns,
        lmsCourses: rows<LmsCourseRecord>(value.lmsCourses).filter(
          (row) =>
            typeof row.lmsId === "number" && typeof row.shortname === "string",
        ),
        registrations: rows(value.registrations),
        registrationStates: rows(value.registrationStates),
        grades: rows(value.grades),
        gpaByTerm: rows(value.gpaByTerm),
        requirements: rows(value.requirements),
      };
    },
    setKyomuGuide: (items) => booleanCommand("set_kyomu_guide", { items }),
    onSchoolIngest: (callback) => {
      let disposed = false;
      let unlisten: (() => void) | undefined;
      void listen<SchoolIngestOutcome>("school-ingest", (payload) => {
        if (!disposed && payload && typeof payload.state === "string")
          callback(payload);
      }).then((fn) => {
        if (disposed) {
          try {
            void Promise.resolve(fn() as unknown).catch(() => undefined);
          } catch {
            // The runtime is already gone (window closing, tests).
          }
        } else unlisten = fn;
      });
      return () => {
        disposed = true;
        try {
          // Tauri's unlisten may also reject asynchronously.
          void Promise.resolve(unlisten?.() as unknown).catch(() => undefined);
        } catch {
          // The runtime is already gone (window closing, tests).
        }
      };
    },
    onSchoolPullProgress: (callback) => {
      let disposed = false;
      let unlisten: (() => void) | undefined;
      void listen<SchoolPullProgress>("school-pull-progress", (payload) => {
        if (
          !disposed &&
          payload?.site === "science-lms" &&
          typeof payload.completed === "number" &&
          typeof payload.total === "number"
        )
          callback(payload);
      }).then((fn) => {
        if (disposed) {
          try {
            void Promise.resolve(fn() as unknown).catch(() => undefined);
          } catch {
            // The runtime is already gone (window closing, tests).
          }
        } else unlisten = fn;
      });
      return () => {
        disposed = true;
        try {
          void Promise.resolve(unlisten?.() as unknown).catch(() => undefined);
        } catch {
          // The runtime is already gone (window closing, tests).
        }
      };
    },
    configureSchoolWebPolicy: (policy) =>
      booleanCommand("configure_school_web_policy", { policy }),
    writeScheduledExport: async () => {
      const value = await invokeCommand("write_scheduled_export");
      if (typeof value !== "string" || !value)
        throw new Error("write_scheduled_export returned an invalid result.");
      return value;
    },
    saveFile: (name, contents) =>
      booleanCommand("save_file", { name, contents }),
    connectCloud: (provider, clientId, redirectUri) =>
      booleanCommand("connect_cloud", { provider, clientId, redirectUri }),
    cloudAuthorizationStatus: () =>
      result<CloudProvider[]>("cloud_authorization_status"),
    schoolRefreshEvent: (event) =>
      result<void>("school_refresh_event", { event }),
    cloudLifecycle: (kind) => result<void>("cloud_lifecycle", { kind }),
    cloudSyncState: () => result<CloudSyncState>("cloud_sync_state"),
    setCloudSyncProvider: (provider, enabled) =>
      result<CloudSyncState>("set_cloud_sync_provider", { provider, enabled }),
    syncCloudNow: (direction) =>
      result<CloudSyncState>("sync_cloud_now", { direction }),
    setCloudSyncAutomatic: (automatic) =>
      result<CloudSyncState>("set_cloud_sync_automatic", { automatic }),
    cloudRecoveryDocument: (id) =>
      result<string>("cloud_recovery_document", { id }),
    restoreCloudVersion: (id) =>
      result<CloudSyncState>("restore_cloud_version", { id }),
    onCloudSyncState: (callback) => {
      let disposed = false;
      let unlisten: (() => void) | undefined;
      void listen<CloudSyncState>("cloud-sync-state", (payload) => {
        if (!disposed && payload && Array.isArray(payload.providers))
          callback(payload);
      }).then((fn) => {
        if (disposed) quietly(fn);
        else unlisten = fn;
      });
      return () => {
        disposed = true;
        if (unlisten) quietly(unlisten);
      };
    },
    disconnectCloud: (provider) =>
      booleanCommand("disconnect_cloud", { provider }),
    cloudCredentialSyncStatus: () =>
      booleanCommand("cloud_credential_sync_status"),
    cloudCredentialBackupStatus: (provider) =>
      booleanCommand("cloud_credential_backup_status", { provider }),
    setCloudCredentialSyncPassword: (password) =>
      booleanCommand("set_cloud_credential_sync_password", { password }),
    syncCloudCredentials: (provider, restoreExisting) =>
      booleanCommand("sync_cloud_credentials", { provider, restoreExisting }),
    clearCloudCredentialSyncPassword: () =>
      booleanCommand("clear_cloud_credential_sync_password"),
    syllabusCourse: async (url, language) => {
      const synced = requireObject<{
        offering: unknown;
        content: { lang: string; content: unknown }[];
      }>(
        "catalog_sync_offering",
        await invokeCommand("catalog_sync_offering", { id: url }),
      );
      const preferred =
        synced.content.find((row) => row.lang === language) ??
        synced.content.find((row) => row.lang !== language);
      return catalogCourse(synced.offering, language, preferred?.content);
    },
    catalogSyncDepartment: (year, scopePath, quarter) =>
      result("catalog_sync_department", { year, scopePath, quarter }),
    catalogQuery: async (year, { q, scope, code, groups }) => {
      const response = requireObject<{
        items: { id: string }[];
        total: number;
        truncated: boolean;
      }>(
        "catalog_query",
        await invokeCommand("catalog_query", {
          query: q,
          year,
          filters: { department: scope, code, groups },
        }),
      );
      return {
        ids: response.items.map((item) => item.id),
        total: response.total,
        truncated: response.truncated,
      };
    },
    catalogSyncOfferings: async (ids) => {
      const warnings = await invokeCommand("catalog_sync_offerings", { ids });
      return Array.isArray(warnings) ? warnings.map(String) : [];
    },
    catalogOfferings: async (year) => {
      const rows = await invokeCommand("catalog_offerings", { year });
      if (!Array.isArray(rows))
        throw new Error("catalog_offerings returned an invalid result.");
      // Rows that do not have the finder's shape are skipped, not fatal. A row
      // stored from a search result has no schedule or credits yet.
      return rows
        .filter(
          (row): row is FinderCourse =>
            Boolean(row) &&
            typeof row.id === "string" &&
            typeof row.code === "string" &&
            Array.isArray(row.quarters) &&
            typeof row.title === "object",
        )
        .map((row) => ({
          ...row,
          detailed:
            Array.isArray(row.schedule) && typeof row.credits === "object",
          schedule: Array.isArray(row.schedule) ? row.schedule : [],
          credits:
            typeof row.credits === "object" && row.credits
              ? row.credits
              : { lecture: 0, exercise: 0, lab: 0 },
          instructors: Array.isArray(row.instructors) ? row.instructors : [],
          format:
            row.format && Array.isArray(row.format.kinds)
              ? row.format
              : { kinds: [] },
          instructionLanguage:
            typeof row.instructionLanguage === "string"
              ? row.instructionLanguage
              : "other",
        }));
    },
  };
}

declare global {
  interface Window {
    /** Set by the trusted native bootstrap from its compile/runtime target. */
    __ISCT_COMPANION_NATIVE_TARGET__?: NativeTarget;
    /** Set by the same bootstrap from the binary's compile-time OS. */
    __ISCT_COMPANION_NATIVE_OS__?: NativeOs;
  }
}

let bridge: DesktopBridge | undefined;
export function getDesktopBridge(): DesktopBridge | undefined {
  if (bridge) return bridge;
  if ("__TAURI_INTERNALS__" in window)
    return (bridge = createTauriDesktopBridge(
      (command, payload) => tauriInvoke<unknown>(command, payload),
      async <T>(event: string, callback: (payload: T) => void) => {
        const off = await tauriListen<T>(event, ({ payload }) =>
          callback(payload),
        );
        return off;
      },
    ));
  return undefined;
}

function matchesNotificationPermission(
  value: unknown,
): value is NotificationPermission {
  return value === "default" || value === "denied" || value === "granted";
}
