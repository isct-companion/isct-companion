import type { MessageKey } from "../i18n";
import type {
  CoreWorkspaceCommit,
  WorkspaceMutation,
} from "../lib/workspace-intents";
import type { FinderRequest, FinderPage } from "./finder-query";
import type { Localized } from "../lib/model";
import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import { nativeError } from "./native-error";
import type {
  SchoolPageAvailability,
  WorkspaceChange,
} from "../generated/persistence";
import { isSyncSnapshot, type SyncHint, type SyncSnapshot } from "./sync";

export type DesktopRuntime = "tauri";
export type NativeTarget = "native-desktop";
/** Rust's `std::env::consts::OS` for this build, injected at startup. */
export type NativeOs = "macos" | "windows";
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
  /** What changed; answers that read one of these are fetched again. */
  changes: WorkspaceChange[];
  /** Core's commit sequence once these commits landed; a query answer
   * with an older `seq` is stale. */
  seq: number;
};
export const nativeOs = (): NativeOs | undefined =>
  typeof window === "undefined"
    ? undefined
    : window.__ISCT_COMPANION_NATIVE_OS__;
/** The desktop tools this binary was built with (`src-tauri/build.rs`). */
export type DesktopTool = "campus-wifi" | "campus-vpn" | "campus-printer";
export const nativeTools = (): readonly DesktopTool[] =>
  (typeof window === "undefined"
    ? undefined
    : window.__ISCT_COMPANION_TOOLS__) ?? [];
/** The system material behind the window (Windows 11 Mica), when there is one. */
export const windowMaterial = (): "mica" | undefined =>
  typeof window === "undefined"
    ? undefined
    : window.__ISCT_COMPANION_WINDOW_MATERIAL__ || undefined;
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
/** One account's school session, as Core's session read model names it. */
export type SchoolSessionState =
  /** No saved credentials. */
  | "not-configured"
  /** Nothing stands in the way: Core signs in when it needs to. */
  | "normal"
  | "signing-in"
  /** The school refused the saved credentials; Core does not sign in with
   * them on its own until they change or the person retries. */
  | "refused"
  /** The person signs in on the school page (email code, password change,
   * a page Core does not know). */
  | "needs-web"
  /** Network, timeout or server failure; Core tries again after `retryAt`. */
  | "unavailable";
const SCHOOL_SESSION_STATES: readonly unknown[] = [
  "not-configured",
  "normal",
  "signing-in",
  "refused",
  "needs-web",
  "unavailable",
] satisfies SchoolSessionState[];
export type SchoolSignInProblemKind =
  /** With `refused`. */
  | "invalid-credentials"
  | "invalid-otp"
  /** A Connect without an OTP setup reached the App OTP step. */
  | "otp-needed"
  | "invalid-matrix"
  /** With `needs-web`. */
  | "email-otp"
  | "password-change"
  | "unknown-page"
  /** With `unavailable`. */
  | "network"
  | "unanswered"
  | "anomaly"
  | "credential-store";
/** Why the last sign-in did not finish. `message` is Core's English
 * diagnostic, for logs and tooltips; the shell shows text chosen by `kind`. */
export type SchoolSignInProblem = {
  kind: SchoolSignInProblemKind;
  message: string;
};
/** LMS (science) or 教務Web (tokyoTech) paused for maintenance. */
export type SchoolMaintenance = {
  /** The school's own notice, when it posted one. */
  notice: string | null;
  /** RFC 3339. */
  until: string;
};
export type SchoolProfileSession = {
  signOutPending: boolean;
  refusalUnsaved: boolean;
  state: SchoolSessionState;
  problem: SchoolSignInProblem | null;
  /** RFC 3339; only with `unavailable`. */
  retryAt: string | null;
  maintenance: SchoolMaintenance | null;
};
/** Both accounts' sessions; every event carries the whole snapshot, and the
 * higher `revision` wins. */
export type SchoolSessionSnapshot = {
  revision: number;
  profiles: {
    science: SchoolProfileSession;
    tokyoTech: SchoolProfileSession;
  };
};
/** How Connect ended. Only `connected` saved anything: the entered
 * credentials are the verified ones now and sync to the other devices. */
export type SchoolConnectOutcome =
  | { kind: "connected" }
  /** The school refused the ID or password, the one-time password or the
   * Matrix (`reason`). */
  | { kind: "refused"; reason: SchoolSignInProblemKind; refusalUnsaved: boolean }
  /** The school wants the person on its own page (`reason`). */
  | { kind: "needs-web"; reason: SchoolSignInProblemKind; refusalUnsaved: boolean }
  /** No verdict: offline, the network, out of time. */
  | { kind: "unreachable" }
  /** The school accepted the credentials and the system vault did not save
   * them: the saved ones are as they were. */
  | { kind: "not-saved" }
  /** A newer Connect, a sign-out or a reset stopped it. */
  | { kind: "stopped" };
const isSchoolSessionSnapshot = (
  value: unknown,
): value is SchoolSessionSnapshot => {
  if (!value || typeof value !== "object") return false;
  const { revision, profiles } = value as Partial<SchoolSessionSnapshot>;
  return (
    Number.isSafeInteger(revision) &&
    (revision as number) >= 0 &&
    Boolean(profiles) &&
    SCHOOL_SESSION_STATES.includes(profiles?.science?.state) &&
    SCHOOL_SESSION_STATES.includes(profiles?.tokyoTech?.state)
  );
};
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
/** Why a school page did not load, as Core reports it. */
export type SchoolLoadFailure =
  | "offline"
  | "host-not-found"
  | "unreachable"
  | "insecure"
  | "cleartext"
  | "other";

/** A one-time note about the page on screen: a reload was sent as GET. */
export type SchoolWebNotice = {
  site: SchoolSite;
  kind: "reopened";
  url: string;
};

/** Why Core's sign-in of a school view stopped. */
export type SchoolWebProblem =
  /** The school could not be reached; no password was sent. */
  | "unreachable"
  /** The school refused the saved password; Settings updates it. */
  | "rejected"
  /** The sign-in could not be written into the web view. */
  | "storage"
  /** The page still asks for a sign-in. */
  | "still-sign-in";

/** Core is signing the view in (the view is covered; the person may cancel),
 * or what its last sign-in left to say, until the view shows another page. */
export type SchoolWebAccess =
  { kind: "signing-in" } | { kind: "problem"; problem: SchoolWebProblem };

/** Whether the app's Back or Forward does something for the page in front;
 * "unknown" when the host does not report history. */
export type SchoolWebStep = "yes" | "no" | "unknown";

/** A load of the page in front that failed; Core clears it. */
export type SchoolWebFailure = {
  url: string | null;
  reason: SchoolLoadFailure;
};

/** The page the person sees of a site, as every surface renders it. */
export type SchoolWebPage = {
  site: SchoolSite;
  /** Grows with every change; keep the highest seen. */
  revision: number;
  url: string | null;
  /** Popups open over the view. */
  popups: number;
  back: SchoolWebStep;
  forward: SchoolWebStep;
  notice: SchoolWebFailure | null;
  access: SchoolWebAccess | null;
  /** The page in front is a sign-in page Core is not signing in now, and the
   * saved account could: the manual way out, through Reload. */
  signIn: boolean;
};

/** How a surface brings a site's view on screen. Core resumes, goes to the
 * URL, or (for "return") resumes only when this surface showed it last. */
export type SchoolWebEntry =
  | { kind: "resume" }
  | { kind: "go"; url: string }
  | { kind: "return"; url: string }
  /** The site's start page as it is: Core writes no identity and signs in
   * nowhere, so the person signs in on the page by hand. */
  | { kind: "unsigned" };

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
  notificationUnavailableReason?: MessageKey | null;
  backgroundSchedule: boolean;
  cloudSync: boolean;
};
export type CampusWifiOutcome =
  | "signed-in"
  | "already-online"
  | "needs-account"
  | "rejected"
  | "science-tokyo-x";
export type CampusWifiStatus =
  "signed-in" | "sign-in-required" | "science-tokyo-x" | "other";
/** A campus VPN reminder Core decided is due. */
export type VpnNotice =
  | { kind: "ending"; minutes: number }
  | { kind: "ended" }
  | { kind: "keepEnded" }
  | { kind: "keepStopped"; code: string };
/** Campus VPN as the helper reports it; `error` is a code the UI keys on. */
export type VpnStatus = {
  /** `unavailable`: this build has no VPN payload (Intel Macs). */
  state: "disconnected" | "connecting" | "connected" | "error" | "unavailable";
  address?: string | null;
  /** When the campus session ends (RFC 3339), for the countdown. */
  expiresAt?: string | null;
  /** When auto-continue ends and disconnects (RFC 3339), while it is on. */
  keepUntil?: string | null;
  error?: string | null;
  detail?: string | null;
};
/** Sign-in stages in order; "joining" is skipped when already on ScienceTokyo. */
export const CAMPUS_WIFI_STEPS = [
  "joining",
  "opening-portal",
  "signing-in",
  "verifying",
] as const;
export type CampusWifiStep = (typeof CAMPUS_WIFI_STEPS)[number];
/** Whose jobs the campus printer would bill (Core's `PrinterAccount`). */
export type PrinterAccount =
  | { kind: "ready"; studentId: string }
  | { kind: "not-connected" }
  | { kind: "not-student-id" }
  | { kind: "needs-attention" }
  | { kind: "unverified" };
export type CampusPrinterStatus = {
  account: PrinterAccount;
  reach: "reachable" | "unreachable";
};
/** How adding the printer to the system ended. */
export type CampusPrinterSetup =
  | { kind: "added" }
  | { kind: "cancelled" }
  | { kind: "account"; account: PrinterAccount }
  | { kind: "failed" };
/** Automatic sync, per device; contract in slotledger approach/cloud-upload-queue. */
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
  /** The revision the next intent is made against. */
  workspaceRevision(): Promise<string>;
  /** Commits one intent; answers what it changed and the next revision. */
  mutateWorkspace(mutation: WorkspaceMutation): Promise<CoreWorkspaceCommit>;
  finderQuery(request: FinderRequest): Promise<FinderPage>;
  scheduleQuery(queries: readonly unknown[]): Promise<{
    values: { kind: string; value: unknown }[];
    /** The changes the values read (`WorkspaceQuery::reads`). */
    reads: WorkspaceChange[];
    /** Core's commit sequence read before the values' data. */
    seq: number;
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
  /** Signs in with the entered credentials; saves them only when the school
   * accepts them. */
  connectSchool(
    profile: SchoolAuthProfile,
    credentials: SchoolCredentials,
  ): Promise<SchoolConnectOutcome>;
  deleteSchoolCredentials(profile: SchoolAuthProfile): Promise<boolean>;
  /** Copies the saved Matrix to the system clipboard without exposing it to the WebView. */
  copySchoolMatrix(): Promise<boolean>;
  copySchoolOtp(setup?: string): Promise<boolean>;
  /** SVG QR code of the OTP setup, for another authenticator to scan. */
  schoolOtpQr(setup?: string): Promise<string>;
  /** Shows `site`'s view for `surface` (an id of the app page showing it). */
  openSchoolSite(
    site: SchoolSite,
    css: string,
    js: string,
    bounds: SchoolWebBounds | undefined,
    surface: string,
    entry: SchoolWebEntry,
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
  /** "Cancel, I'll sign in myself" while Core signs the view in. */
  cancelSchoolWebSignIn?(site: SchoolSite): Promise<boolean>;
  navigateSchoolWeb?(site: SchoolSite, url: string): Promise<boolean>;
  /** Back through the engine's history of the view or the popup in front;
   * Core keeps it from sending a form again. */
  schoolWebBack?(site: SchoolSite): Promise<boolean>;
  schoolWebForward?(site: SchoolSite): Promise<boolean>;
  /** Closes the popup in front of the site's view; false when none is open. */
  closeSchoolWebPopup?(site: SchoolSite): Promise<boolean>;
  setSchoolWebZoom?(site: SchoolSite, scale: number): Promise<boolean>;
  /** The page the person sees of `site` now. */
  schoolWebPage?(site: SchoolSite): Promise<SchoolWebPage>;
  /** Every change of the page on screen. Resolves once the listener is
   * registered, so a snapshot read afterwards misses nothing. */
  onSchoolWebPage?(
    callback: (page: SchoolWebPage) => void,
  ): Promise<() => void>;
  /** A one-time note about the page the person is looking at. */
  onSchoolWebNotice?(callback: (notice: SchoolWebNotice) => void): () => void;
  /** What the Wi-Fi interface is on now (macOS builds only). */
  campusWifiStatus?(): Promise<CampusWifiStatus>;
  /** Campus VPN state from the privileged helper (macOS and Windows builds). */
  vpnStatus?(): Promise<VpnStatus>;
  /** One administrator approval installs or updates the helper and openconnect. */
  vpnInstall?(): Promise<VpnStatus>;
  /** Signs in to the campus VPN with the saved Tokyo Tech account and connects. */
  vpnConnect?(): Promise<VpnStatus>;
  vpnDisconnect?(): Promise<VpnStatus>;
  /** Auto-continue for `hours`, which it always needs, or off with `null`. */
  vpnKeep?(hours: number | null): Promise<VpnStatus>;
  /** The state after each check while a tunnel is up (every minute). */
  onVpnStatus?(callback: (status: VpnStatus) => void): () => void;
  /** The session ends in a few minutes, or ended on its own. */
  onVpnNotice?(callback: (notice: VpnNotice) => void): () => void;
  /**
   * Joins ScienceTokyo if needed and signs in once with the Tokyo Tech account
   * (macOS builds only). Returns "science-tokyo-x" without changing anything
   * unless leaving ScienceTokyoX was confirmed.
   */
  campusWifiConnect?(
    leaveScienceTokyoX: boolean,
    onStep: (step: CampusWifiStep) => void,
  ): Promise<CampusWifiOutcome>;
  /** The verified student ID and whether the printer answers, found once. */
  campusPrinterStatus?(): Promise<CampusPrinterStatus>;
  /** Adds or replaces the system printer behind the system's administrator
   * prompt, which shows `prompt` on macOS. */
  installCampusPrinter?(prompt: string): Promise<CampusPrinterSetup>;
  onSchoolDownload?(callback: (event: SchoolDownloadEvent) => void): () => void;
  /** Shows a file saved from a school page this session in Finder; false for any other path. */
  revealSchoolDownload?(path: string): Promise<boolean>;
  schoolSessionSnapshot(): Promise<SchoolSessionSnapshot>;
  /** One foreground sign-in with the saved credentials, read again from the
   * synced store; the credentials themselves do not change. */
  retrySchool(profile: SchoolAuthProfile): Promise<SchoolSessionSnapshot>;
  onSchoolSession(
    callback: (snapshot: SchoolSessionSnapshot) => void,
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
  onCloudSyncState?(callback: (state: CloudSyncState) => void): () => void;
  cloudCredentialSyncStatus(): Promise<boolean>;
  cloudCredentialBackupStatus(): Promise<boolean>;
  /** Restores the Drive backup with `password` on a device without one;
   * otherwise uploads this device's logins under `password`, a new one. */
  enableCloudCredentialSync(password: string): Promise<boolean>;
  clearCloudCredentialSyncPassword(): Promise<boolean>;
}

const schoolWebSteps: readonly unknown[] = ["yes", "no", "unknown"];
const isSchoolWebPage = (value: unknown): value is SchoolWebPage => {
  if (!value || typeof value !== "object") return false;
  const page = value as Record<string, unknown>;
  const notice = page.notice as Record<string, unknown> | null | undefined;
  const access = page.access as Record<string, unknown> | null | undefined;
  return (
    typeof page.site === "string" &&
    Number.isSafeInteger(page.revision) &&
    (page.url === null || typeof page.url === "string") &&
    Number.isSafeInteger(page.popups) &&
    schoolWebSteps.includes(page.back) &&
    schoolWebSteps.includes(page.forward) &&
    typeof page.signIn === "boolean" &&
    (notice === null ||
      (typeof notice === "object" &&
        (notice.url === null || typeof notice.url === "string") &&
        typeof notice.reason === "string")) &&
    (access === null ||
      (typeof access === "object" &&
        (access.kind === "signing-in" ||
          (access.kind === "problem" && typeof access.problem === "string"))))
  );
};

/** Listens to a native event, passing on only payloads `accepts` takes. */
function subscribe<T>(
  event: string,
  accepts: (payload: any) => boolean,
  callback: (payload: T) => void,
): () => void {
  let disposed = false;
  let unlisten: (() => void) | undefined;
  void tauriListen<T>(event, ({ payload }) => {
    if (!disposed && accepts(payload)) callback(payload);
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
      throw nativeError(error, `${command} failed`);
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
    workspaceRevision: async () => {
      const revision = await invokeCommand("workspace_revision");
      if (typeof revision !== "string")
        throw new Error("workspace_revision returned an invalid result.");
      return revision;
    },
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
        const { source, revision, changes, seq } = event as Record<
          string,
          unknown
        >;
        // Unknown future sources must still make the queries refetch.
        if (
          typeof source !== "string" ||
          typeof seq !== "number" ||
          (revision !== undefined && typeof revision !== "string") ||
          !Array.isArray(changes) ||
          changes.some((change) => typeof change !== "string")
        )
          return;
        callback({
          source: source as WorkspaceChangedEvent["source"],
          ...(revision === undefined ? {} : { revision }),
          changes: changes as WorkspaceChange[],
          seq,
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
    campusPrinterStatus: () => result("plugin:campus-printer|status"),
    installCampusPrinter: (prompt) =>
      result("plugin:campus-printer|install", { prompt }),
    campusWifiStatus: () => invokeCommand("plugin:campus-wifi|status"),
    vpnStatus: () => invokeCommand("plugin:campus-vpn|status"),
    vpnInstall: () => invokeCommand("plugin:campus-vpn|install"),
    vpnConnect: () => invokeCommand("plugin:campus-vpn|connect"),
    vpnDisconnect: () => invokeCommand("plugin:campus-vpn|disconnect"),
    vpnKeep: (hours) => invokeCommand("plugin:campus-vpn|keep", { hours }),
    onVpnStatus: (callback) =>
      subscribe<VpnStatus>(
        "vpn-status",
        (payload) => typeof payload?.state === "string",
        callback,
      ),
    onVpnNotice: (callback) =>
      subscribe<VpnNotice>(
        "vpn-notice",
        (payload) =>
          payload?.kind === "ended" ||
          payload?.kind === "keepEnded" ||
          (payload?.kind === "keepStopped" &&
            typeof payload.code === "string") ||
          (payload?.kind === "ending" && typeof payload.minutes === "number"),
        callback,
      ),
    campusWifiConnect: (leaveScienceTokyoX, onStep) => {
      const steps = new Channel<CampusWifiStep>();
      steps.onmessage = onStep;
      return invokeCommand("plugin:campus-wifi|connect", {
        leaveScienceTokyoX,
        onStep: steps,
      });
    },
    connectSchool: (profile, credentials) =>
      result("connect_school", { profile, credentials }),
    deleteSchoolCredentials: (profile) =>
      booleanCommand("delete_school_credentials", { profile }),
    copySchoolMatrix: () => booleanCommand("copy_school_matrix"),
    copySchoolOtp: (setup) => booleanCommand("copy_school_otp", { setup }),
    schoolOtpQr: async (setup) =>
      requireString(
        "school_otp_qr",
        await invokeCommand("school_otp_qr", { setup }),
      ),
    schoolSessionSnapshot: async () => {
      const snapshot = await invokeCommand("school_session_snapshot");
      if (!isSchoolSessionSnapshot(snapshot))
        throw new Error("school_session_snapshot returned an invalid result.");
      return snapshot;
    },
    retrySchool: async (profile) => {
      const snapshot = await invokeCommand("retry_school", { profile });
      if (!isSchoolSessionSnapshot(snapshot))
        throw new Error("retry_school returned an invalid result.");
      return snapshot;
    },
    onSchoolSession: async (callback) => {
      let disposed = false;
      const unlisten = await listen<unknown>("school-session", (payload) => {
        if (!disposed && isSchoolSessionSnapshot(payload)) callback(payload);
      });
      return () => {
        disposed = true;
        quietly(unlisten);
      };
    },
    openSchoolSite: (site, css, js, bounds, surface, entry) =>
      result("open_school_site", {
        site,
        css,
        js,
        bounds: bounds ?? null,
        surface,
        entry,
      }),
    schoolWebPage: async (site) => {
      const page = await result<SchoolWebPage>("school_web_page", { site });
      if (!isSchoolWebPage(page))
        throw new Error("school_web_page returned an invalid result.");
      return page;
    },
    onSchoolWebPage: async (callback) => {
      let disposed = false;
      const unlisten = await listen<unknown>("school-web-page", (payload) => {
        if (!disposed && isSchoolWebPage(payload)) callback(payload);
      });
      return () => {
        disposed = true;
        quietly(unlisten);
      };
    },
    setSchoolWebBounds: (bounds) =>
      booleanCommand("set_school_web_bounds", { bounds }),
    hideSchoolWeb: () => booleanCommand("hide_school_web"),
    reloadSchoolWeb: (site) => booleanCommand("reload_school_web", { site }),
    cancelSchoolWebSignIn: (site) =>
      booleanCommand("cancel_school_web_sign_in", { site }),
    navigateSchoolWeb: (site, url) =>
      booleanCommand("navigate_school_web", { site, url }),
    schoolWebBack: (site) => booleanCommand("school_web_back", { site }),
    schoolWebForward: (site) => booleanCommand("school_web_forward", { site }),
    closeSchoolWebPopup: (site) =>
      booleanCommand("close_school_web_popup", { site }),
    setSchoolWebZoom: (site, scale) =>
      booleanCommand("set_school_web_zoom", { site, scale }),
    onSchoolWebNotice: (callback) => {
      let disposed = false;
      let unlisten: (() => void) | undefined;
      void listen<SchoolWebNotice>("school-web-notice", (payload) => {
        if (
          !disposed &&
          payload &&
          typeof payload.site === "string" &&
          payload.kind === "reopened" &&
          typeof payload.url === "string"
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
    cloudLifecycle: (kind) => result<void>("cloud_lifecycle", { kind }),
    cloudSyncState: () => result<CloudSyncState>("cloud_sync_state"),
    agentConnection: () => result<AgentConnection>("agent_connection"),
    setCloudSyncProvider: (provider, enabled) =>
      result<CloudSyncState>("set_cloud_sync_provider", { provider, enabled }),
    syncCloudNow: (direction) =>
      result<CloudSyncState>("sync_cloud_now", { direction }),
    setCloudSyncAutomatic: (automatic) =>
      result<CloudSyncState>("set_cloud_sync_automatic", { automatic }),
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
    cloudCredentialSyncStatus: () =>
      booleanCommand("cloud_credential_sync_status"),
    cloudCredentialBackupStatus: () =>
      booleanCommand("cloud_credential_backup_status"),
    enableCloudCredentialSync: (password) =>
      booleanCommand("enable_cloud_credential_sync", { password }),
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
    /** Set by the same bootstrap: the tools compiled into the binary. */
    __ISCT_COMPANION_TOOLS__?: readonly DesktopTool[];
    /** Set by the same bootstrap: the material the native window draws. */
    __ISCT_COMPANION_WINDOW_MATERIAL__?: "mica" | "";
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
