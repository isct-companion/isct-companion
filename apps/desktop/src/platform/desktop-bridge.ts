import type {
  CoreWorkspacePatch,
  CoreWorkspaceSnapshot,
  WorkspaceMutation,
} from "../lib/workspace-intents";
import type { FinderRequest, FinderPage } from "./finder-query";
import type { Localized } from "../lib/model";
import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import type {
  SchoolPageAvailability,
  SchoolOtpSetup,
  WorkspaceChange,
} from "../generated/persistence";
import { isSyncSnapshot, type SyncHint, type SyncSnapshot } from "./sync";

export type DesktopRuntime = "tauri";
export type NativeTarget = "native-desktop";
/** Rust's `std::env::consts::OS` for this build, injected at startup. */
export type NativeOs = "macos" | "windows" | "linux";
export type WorkspaceChangedEvent = {
  source:
    | "user-intent"
    | "reference-pull"
    | "reference-poll"
    | "school-pull"
    | "scheduler"
    | "mcp"
    | "cloud-sync"
    | "cloud-restore"
    | "reset";
  /** The committed user-intent revision, when the source can provide it. */
  revision?: string;
  /** What changed; `workspaceRecords` reads the records these feed. */
  changes: WorkspaceChange[];
};
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
  | "syllabus"
  | "study-guide";
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
  suggestedAdmissionYear?: number;
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
  "pending" | "active" | "maintenance" | "failed" | "not-configured";
/** A school session state changed (sign-in, launch, credential change, or renewal). */
export type SchoolSessionEvent = {
  profile: SchoolAuthProfile;
  state: SchoolSessionState;
  revision: number;
};
export type SchoolSessionSnapshot = {
  revision: number;
  kyomuMaintenance?: string | null;
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
  value === "maintenance" ||
  value === "failed" ||
  value === "not-configured";
/** One extractor result from LMS or one 教務Web page. */
export type SchoolIngestOutcome = {
  site: SchoolSite;
  kind: string;
  state: "ok" | "unavailable" | "maintenance" | "reconnect-required" | "error";
  changed: number;
  message: string;
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
/** Freshness of one synced key (`dataset_state`). Times are ISO strings. */
export type KeyFreshness = {
  /** Last complete read; null before the first one or after gc. */
  fetchedAt: string | null;
  outcome: "complete" | "unavailable" | null;
  message: string | null;
  /** Consecutive failed reads since the last complete one. */
  failures: number;
  /** No automatic read before this time. */
  nextEligible: string | null;
  /** Short error code of the last failed read. */
  lastError: string | null;
};
/** School-owned rows the workspace does not carry. */
export type SchoolRecords = {
  /** Freshness per `dataset/key`, e.g. `kyomu.page/gpa`. */
  freshness: Record<string, KeyFreshness>;
  /** Latest complete read of each 教務Web page, keyed `kyomu:<kind>`. */
  pageAvailability: Record<string, SchoolPageAvailability>;
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
  notificationUnavailableReason?: string | null;
  backgroundSchedule: boolean;
  cloudSync: boolean;
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

/** The local MCP server: no listener or token while agent access is off. */
export type AgentConnection =
  | { state: "off" }
  | { state: "listening"; url: string; token: string }
  | { state: "failed"; error: string };

export type InvokeNative = (
  command: string,
  payload?: Record<string, unknown>,
) => Promise<unknown>;
export type ListenNative = <T>(
  event: string,
  callback: (payload: T) => void,
) => Promise<() => void>;

export interface AppUpdateStatus {
  phase:
    | "idle"
    | "checking"
    | "unpublished"
    | "current"
    | "available"
    | "downloading"
    | "ready"
    | "error";
  version?: string | null;
  message?: string | null;
  releaseUrl: string;
  signedUpdates: boolean;
  progress?: { downloaded: number; total?: number | null } | null;
  /** How the install attempted before this launch turned out; Core judges it. */
  lastInstall?: AppLastInstall | null;
}
export type AppLastInstall =
  | { outcome: "installed"; version: string }
  | { outcome: "incomplete"; version: string }
  | { outcome: "failed"; version: string; message: string };
export type ApplicationLog = {
  id: number;
  at: string;
  level: string;
  target: string;
  message: string;
};
export type ApplicationLogSnapshot = {
  entries: ApplicationLog[];
  cursor: number;
  dropped: boolean;
};

export interface DesktopBridge {
  applicationLogs?(after?: number): Promise<ApplicationLogSnapshot>;
  /** The retained log as plain text for a bug report. */
  exportApplicationLogs?(): Promise<string>;
  /** Copies the retained log to the system clipboard from the shell. */
  copyApplicationLogs?(): Promise<boolean>;
  recordApplicationError?(message: string): Promise<void>;
  checkAppUpdate?(
    manual: boolean,
    download?: boolean,
  ): Promise<AppUpdateStatus>;
  appUpdateState?(): Promise<AppUpdateStatus>;
  installAppUpdate?(): Promise<void>;
  workspaceSnapshot(): Promise<CoreWorkspaceSnapshot>;
  /** Only the records the named changes feed, to patch the snapshot with. */
  workspaceRecords(
    changes: readonly WorkspaceChange[],
  ): Promise<CoreWorkspacePatch>;
  mutateWorkspace(mutation: WorkspaceMutation): Promise<CoreWorkspaceSnapshot>;
  finderQuery(request: FinderRequest): Promise<FinderPage>;
  scheduleQuery(queries: readonly unknown[]): Promise<{
    revision: string;
    values: { kind: string; value: unknown }[];
  }>;
  runtime: DesktopRuntime;
  /** The build's operating system; see `nativeOs`. */
  os?: NativeOs;
  getRuntimeCapabilities(): Promise<NativeCapabilitySnapshot>;
  resolveSchoolUrl(
    url: string,
    site: SchoolSite,
  ): Promise<{ site: SchoolSite; url: string }>;
  openExternal(url: string): Promise<void>;
  bootstrapReminders(fired: unknown): Promise<boolean>;
  notify(title: string, body: string): Promise<boolean>;
  requestNotificationPermission(): Promise<NotificationPermission>;
  getNotificationPermission(): Promise<NotificationPermission>;
  openNotificationSettings(): Promise<void>;
  runScheduledTask(id: string): Promise<string>;
  onWorkspaceChanged(
    callback: (event: WorkspaceChangedEvent) => void,
  ): Promise<() => void>;
  /** A year's stored catalog changed; Core sends it at most every 250 ms
   * while a download lands department by department. */
  onCatalogChanged?(
    callback: (event: { year: number; version: number }) => void,
  ): () => void;
  schoolCredentialStatus(): Promise<SchoolCredentialStatus>;
  beginSchoolOtpSetup(): Promise<number>;
  cancelSchoolOtpSetup(operationId: number): Promise<void>;
  schoolOtpSetup(operationId: number): Promise<SchoolOtpSetup>;
  completeSchoolOtpSetup(operationId: number, reset: boolean): Promise<void>;
  saveSchoolCredentials(
    profile: SchoolAuthProfile,
    credentials: SchoolCredentials,
  ): Promise<boolean>;
  deleteSchoolCredentials(profile: SchoolAuthProfile): Promise<boolean>;
  /** Copies the saved Matrix to the system clipboard without exposing it to the WebView. */
  copySchoolMatrix(): Promise<boolean>;
  copySchoolOtp(setup?: string): Promise<boolean>;
  /** SVG QR code of the OTP setup, for another authenticator to scan. */
  schoolOtpQr(setup?: string): Promise<string>;
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
  /** A shell fact or intent for Core's sync planner; never waits for a read. */
  syncHint?(hint: SyncHint): Promise<void>;
  /** Core's sync snapshot now. */
  syncSnapshot?(): Promise<SyncSnapshot>;
  /** The current snapshot, then every `sync-snapshot` event. */
  onSyncSnapshot?(callback: (snapshot: SyncSnapshot) => void): () => void;
  /** Shows (or clears, with `[]`) the registration queue on 教務Web pages. */
  setKyomuGuide?(items: KyomuGuideItem[]): Promise<boolean>;
  /** An extractor finished while the person browsed LMS / 教務Web. */
  onSchoolIngest?(callback: (outcome: SchoolIngestOutcome) => void): () => void;
  /** Saves a validated workspace export under app-owned local storage. */
  /** Asks where to save an exported file; false when the person cancels. */
  saveFile(name: string, contents: string): Promise<boolean>;
  connectCloud(
    provider: CloudProvider,
    clientId: string,
    redirectUri: string,
  ): Promise<boolean>;
  cloudAuthorizationStatus(): Promise<CloudProvider[]>;
  cloudLifecycle?(kind: "foreground" | "background"): Promise<void>;
  cloudSyncState?(): Promise<CloudSyncState>;
  /** Local MCP server state; reconciles the listener with the setting first. */
  agentConnection?(): Promise<AgentConnection>;
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
const requireString = (command: string, value: unknown) => {
  if (typeof value !== "string")
    throw new Error(`${command} returned an invalid result.`);
  return value;
};
const requireObject = <T>(command: string, value: unknown): T => {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error(`${command} returned an invalid result.`);
  return value as T;
};
export function createTauriDesktopBridge(
  invoke: InvokeNative,
  listen: ListenNative,
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
    os: nativeOs(),
    checkAppUpdate: (manual: boolean, download = false) =>
      result<AppUpdateStatus>("check_app_update", { manual, download }),
    applicationLogs: (after) => result("application_logs", { after }),
    exportApplicationLogs: async () =>
      requireString(
        "export_application_logs",
        await invokeCommand("export_application_logs"),
      ),
    copyApplicationLogs: () => booleanCommand("copy_application_logs"),
    recordApplicationError: (message) =>
      result("application_error", { message }),
    appUpdateState: () => result<AppUpdateStatus>("app_update_state"),
    installAppUpdate: () => invokeCommand<void>("install_app_update"),
    workspaceSnapshot: () => result("workspace_snapshot"),
    workspaceRecords: (changes) => result("workspace_records", { changes }),
    mutateWorkspace: (mutation) => result("mutate_workspace", { mutation }),
    finderQuery: (request) => result("finder_query", { request }),
    scheduleQuery: (queries) => result("schedule_query", { queries }),
    getRuntimeCapabilities: () => result("runtime_capabilities"),
    resolveSchoolUrl: (url, site) =>
      result("resolve_school_url", { url, site }),
    openExternal: async (url) => {
      await invokeCommand("open_external", { url });
    },
    bootstrapReminders: (fired) =>
      booleanCommand("bootstrap_reminders", { fired }),
    notify: (title, body) => booleanCommand("notify", { title, body }),
    requestNotificationPermission: async () => {
      const value = await invokeCommand("request_notification_permission");
      if (!matchesNotificationPermission(value))
        throw new Error(
          "request_notification_permission returned an invalid result.",
        );
      return value;
    },
    getNotificationPermission: async () => {
      const value = await invokeCommand("notification_permission");
      if (!matchesNotificationPermission(value))
        throw new Error("notification_permission returned an invalid result.");
      return value;
    },
    openNotificationSettings: () =>
      invokeCommand<void>("open_notification_settings"),
    runScheduledTask: (id) => invokeCommand("run_scheduled_task", { id }),
    onWorkspaceChanged: async (callback) => {
      const unlisten = await listen<unknown>("workspace-changed", (event) => {
        if (!event || typeof event !== "object") return;
        const { source, revision, changes } = event as Record<string, unknown>;
        // Unknown future sources must still trigger snapshot recovery.
        if (
          typeof source !== "string" ||
          (revision !== undefined && typeof revision !== "string") ||
          !Array.isArray(changes) ||
          changes.some((change) => typeof change !== "string")
        )
          return;
        callback({
          source: source as WorkspaceChangedEvent["source"],
          ...(revision === undefined ? {} : { revision }),
          changes: changes as WorkspaceChange[],
        });
      });
      return () => quietly(unlisten);
    },
    onCatalogChanged: (callback) => {
      let disposed = false;
      let unlisten: (() => void) | undefined;
      void listen<unknown>("catalog-changed", (payload) => {
        if (disposed || !payload || typeof payload !== "object") return;
        const { year, version } = payload as Record<string, unknown>;
        if (Number.isSafeInteger(year) && Number.isSafeInteger(version))
          callback({ year: year as number, version: version as number });
      })
        .then((fn) => {
          if (disposed) quietly(fn);
          else unlisten = fn;
        })
        .catch(() => undefined);
      return () => {
        disposed = true;
        if (unlisten) quietly(unlisten);
      };
    },
    schoolCredentialStatus: () => result("school_credential_status"),
    beginSchoolOtpSetup: async () => {
      const id = await invoke("begin_school_otp_setup");
      if (typeof id !== "number" || !Number.isSafeInteger(id) || id < 0)
        throw new Error("begin_school_otp_setup returned an invalid result.");
      return id;
    },
    cancelSchoolOtpSetup: (operationId) =>
      invoke("cancel_school_otp_setup", { operationId }).then(() => {}),
    schoolOtpSetup: (operationId) =>
      invoke("school_otp_setup", { operationId }).then((value) =>
        requireObject<SchoolOtpSetup>("school_otp_setup", value),
      ),
    completeSchoolOtpSetup: (operationId, reset) =>
      invoke("complete_school_otp_setup", { operationId, reset }).then(
        () => {},
      ),
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
    copySchoolOtp: (setup) => booleanCommand("copy_school_otp", { setup }),
    schoolOtpQr: async (setup) =>
      requireString(
        "school_otp_qr",
        await invokeCommand("school_otp_qr", { setup }),
      ),
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
    syncHint: async (hint) => {
      await invokeCommand("sync_hint", { hint });
    },
    syncSnapshot: async () => {
      const value = await invokeCommand("sync_snapshot");
      if (!isSyncSnapshot(value))
        throw new Error("sync_snapshot returned an invalid result.");
      return value;
    },
    onSyncSnapshot: (callback) => {
      let disposed = false;
      let unlisten: (() => void) | undefined;
      let version = 0;
      void listen<SyncSnapshot>("sync-snapshot", (payload) => {
        if (!disposed && isSyncSnapshot(payload)) {
          version++;
          callback(payload);
        }
      })
        .then(async (fn) => {
          if (disposed) {
            quietly(fn);
            return;
          }
          unlisten = fn;
          // Listen first, then read: a run may finish before the shell mounts.
          // A late read never replaces an event that arrived meanwhile.
          const currentVersion = version;
          const value = await invokeCommand("sync_snapshot");
          if (!disposed && currentVersion === version && isSyncSnapshot(value))
            callback(value);
        })
        .catch(() => undefined);
      return () => {
        disposed = true;
        if (unlisten) quietly(unlisten);
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
    configureSchoolWebPolicy: (policy) =>
      booleanCommand("configure_school_web_policy", { policy }),

    saveFile: (name, contents) =>
      booleanCommand("save_file", { name, contents }),
    connectCloud: (provider, clientId, redirectUri) =>
      booleanCommand("connect_cloud", { provider, clientId, redirectUri }),
    cloudAuthorizationStatus: () =>
      result<CloudProvider[]>("cloud_authorization_status"),
    cloudLifecycle: (kind) => result<void>("cloud_lifecycle", { kind }),
    cloudSyncState: () => result<CloudSyncState>("cloud_sync_state"),
    agentConnection: () => result<AgentConnection>("agent_connection"),
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
