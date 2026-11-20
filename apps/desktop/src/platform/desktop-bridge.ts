import type { MessageKey } from "../i18n";
import type {
  CoreWorkspaceCommit,
  WorkspaceMutation,
} from "../lib/workspace-intents";
import type { FinderRequest } from "./finder-contract";
import type { FinderPage } from "../generated/persistence";
import type { Localized } from "../lib/model";
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import type {
  SchoolPageAvailability,
  SyncHint,
  SyncSnapshot,
  WorkspaceChange,
} from "../generated/persistence";
import { isSyncSnapshot } from "./sync";

import {
  createBridgeCommands,
  quietly,
  requireString,
  subscribe,
  type InvokeNative,
  type ListenNative,
} from "./desktop-bridge/helpers";
import {
  campusToolsCommands,
  type CampusToolsBridge,
  type DesktopTool,
} from "./desktop-bridge/campus-tools";
import {
  cloudSyncCommands,
  type CloudSyncBridge,
} from "./desktop-bridge/cloud-sync";
import {
  pastExamsCommands,
  type PastExamsBridge,
} from "./desktop-bridge/past-exams";
import {
  schoolSessionCommands,
  type SchoolSessionBridge,
} from "./desktop-bridge/school-session";
import {
  schoolWebCommands,
  type SchoolWebBridge,
} from "./desktop-bridge/school-web";

export type {
  InvokeNative,
  ListenNative,
  SavedDownload,
} from "./desktop-bridge/helpers";
export {
  CAMPUS_WIFI_STEPS,
  type CampusPrinterStatus,
  type CampusWifiOutcome,
  type CampusWifiStatus,
  type CampusWifiStep,
  type DesktopTool,
  type PrinterAccount,
  type VpnNotice,
  type VpnStatus,
} from "./desktop-bridge/campus-tools";
export type {
  CloudSyncProvider,
  CloudSyncState,
  CloudSyncVersion,
} from "./desktop-bridge/cloud-sync";
export type {
  DownloadEvent,
  SchoolDownloadEvent,
  SchoolLoadFailure,
  SchoolOtpEnrollment,
  SchoolOtpIntent,
  SchoolWebBounds,
  SchoolWebEntry,
  SchoolWebNotice,
  SchoolWebPage,
  SchoolWebProblem,
} from "./desktop-bridge/school-web";

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
export const nativeTools = (): readonly DesktopTool[] =>
  (typeof window === "undefined"
    ? undefined
    : window.__ISCT_COMPANION_TOOLS__) ?? [];
/** The system material behind the window (Windows 11 Mica), when there is one. */
export const windowMaterial = (): "mica" | undefined =>
  typeof window === "undefined"
    ? undefined
    : window.__ISCT_COMPANION_WINDOW_MATERIAL__ || undefined;
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
export type NativeCapabilitySnapshot = {
  schoolImport: boolean;
  schoolWeb: boolean;
  notifications: boolean;
  notificationUnavailableReason?: MessageKey | null;
  backgroundSchedule: boolean;
  cloudSync: boolean;
};
/** The local MCP server: no listener or token while agent access is off. */
export type AgentConnection =
  | { state: "off" }
  | { state: "listening"; url: string; token: string }
  | { state: "failed"; error: string };

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
  /** Whether Core offers this version in the update prompt. */
  prompt?: boolean;
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

export interface DesktopBridge
  extends
    PastExamsBridge,
    SchoolSessionBridge,
    SchoolWebBridge,
    CampusToolsBridge,
    CloudSyncBridge
{
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
  openExternal(url: string): Promise<void>;
  /** Opens the feedback form in the system browser, this build filled in. */
  openFeedbackForm(): Promise<void>;
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
  /** A shell fact or intent for Core's sync planner; never waits for a read. */
  syncHint?(hint: SyncHint): Promise<void>;
  /** Core's sync snapshot now. */
  syncSnapshot?(): Promise<SyncSnapshot>;
  /** The current snapshot, then every `sync-snapshot` event. */
  onSyncSnapshot?(callback: (snapshot: SyncSnapshot) => void): () => void;
  /** Saves a validated workspace export under app-owned local storage. */
  /** Asks where to save an exported file; false when the person cancels. */
  saveFile(name: string, contents: string): Promise<boolean>;
  /** Local MCP server state; reconciles the listener with the setting first. */
  agentConnection?(): Promise<AgentConnection>;
}

export function createTauriDesktopBridge(
  invoke: InvokeNative,
  listen: ListenNative,
): DesktopBridge {
  const commands = createBridgeCommands(invoke);
  const { invokeCommand, result, booleanCommand } = commands;
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
    ...pastExamsCommands(commands, listen),
    scheduleQuery: (queries) => result("schedule_query", { queries }),
    getRuntimeCapabilities: () => result("runtime_capabilities"),
    openExternal: async (url) => {
      await invokeCommand("open_external", { url });
    },
    openFeedbackForm: async () => {
      await invokeCommand("open_feedback_form");
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
    onCatalogChanged: (callback) =>
      subscribe<{ year: number; version: number }>(
        listen,
        "catalog-changed",
        (payload) => {
          if (!payload || typeof payload !== "object") return;
          const { year, version } = payload;
          if (Number.isSafeInteger(year) && Number.isSafeInteger(version))
            return { year, version };
        },
        callback,
      ),
    ...schoolSessionCommands(commands, listen),
    ...campusToolsCommands(commands, listen),
    ...schoolWebCommands(commands, listen),
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
      void listen<unknown>("sync-snapshot", (payload) => {
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

    saveFile: (name, contents) =>
      booleanCommand("save_file", { name, contents }),
    ...cloudSyncCommands(commands, listen),
    agentConnection: () => result<AgentConnection>("agent_connection"),
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
