import type { SyllabusFilters } from "../core/syllabus-filters";
import { validSyllabusContent } from "../core/syllabus-content";
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import type { SchoolImportResult } from "./capabilities";
import type { SyllabusCourse } from "../core/syllabus";

export type DesktopRuntime = "tauri";
export type NativeTarget = "native-desktop" | "native-mobile";
export type SchoolSite =
  | "science-portal"
  | "science-lms"
  | "tokyo-portal"
  | "tokyo-mail"
  | "kyomu"
  | "library"
  | "syllabus";
export type Schedule = "off" | "launch" | "hourly";
export type SchoolAuthProfile = "science" | "tokyo-tech";
export type SchoolCredentials = {
  username: string;
  password: string;
  otpSecret: string;
  matrix: string;
};
export type SchoolCredentialStatus = {
  science: {
    saved: boolean;
    username: string;
    hasOtp: boolean;
    hasMatrix: boolean;
  };
  tokyoTech: {
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
/** A school session became usable or failed (sign-in, launch, or renewal). */
export type SchoolSessionEvent = {
  profile: SchoolAuthProfile;
  state: "active" | "failed";
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
  syllabusFilters?: boolean;
};
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
  platform: string;
  getRuntimeCapabilities(): Promise<NativeCapabilitySnapshot>;
  openExternal(url: string): Promise<void>;
  notify(title: string, body: string): Promise<boolean>;
  requestNotificationPermission(): Promise<NotificationPermission>;
  schoolCredentialStatus(): Promise<SchoolCredentialStatus>;
  saveSchoolCredentials(
    profile: SchoolAuthProfile,
    credentials: SchoolCredentials,
  ): Promise<boolean>;
  deleteSchoolCredentials(profile: SchoolAuthProfile): Promise<boolean>;
  testSchoolConnection(
    profile: SchoolAuthProfile,
    credentials: SchoolCredentials,
  ): Promise<{ site: SchoolSite; url: string; phase: string }>;
  loginSchool(profile: SchoolAuthProfile): Promise<{
    site: SchoolSite;
    url: string;
    phase: string;
  }>;
  importSchool(): Promise<SchoolImportResult>;
  cachedSchool(): Promise<SchoolImportResult>;
  openSchoolSite(
    site: SchoolSite,
    css: string,
    js: string,
    bounds?: SchoolWebBounds,
    targetUrl?: string,
  ): Promise<{
    site: SchoolSite;
    url: string;
    phase:
      | "sessionRestored"
      | "pageLoaded"
      | "interactiveLoginCompleted"
      | "reconnectRequired";
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
  setSchoolWebZoom?(site: SchoolSite, scale: number): Promise<boolean>;
  onSchoolWebUrl?(callback: (url: string) => void): () => void;
  onSchoolLoginStatus?(callback: (event: SchoolLoginEvent) => void): () => void;
  /**
   * Makes sure the profile is signed in: loads its portal home in the hidden
   * school view, and if that lands on sign-in, signs in with the saved
   * credentials. Used at launch; later expiries are renewed natively whenever
   * a school page is redirected to sign-in. Resolves false when skipped
   * (school view in use or a sign-in running); results arrive through
   * `onSchoolSession`.
   */
  ensureSchoolSession?(profile: SchoolAuthProfile): Promise<boolean>;
  onSchoolSession?(callback: (event: SchoolSessionEvent) => void): () => void;
  configureSchoolSchedule(schedule: Schedule): Promise<boolean>;
  onScheduledSchoolResult(
    callback: (result: SchoolImportResult) => void,
  ): () => void;
  updateExportWorkspace(workspace: unknown): Promise<boolean>;
  configureExportSchedule(schedule: Schedule): Promise<boolean>;
  connectCloud(
    provider: "google-drive" | "onedrive",
    clientId: string,
    redirectUri: string,
  ): Promise<boolean>;
  cloudRequest(
    provider: "google-drive" | "onedrive",
    url: string,
    init: {
      method?: string;
      headers?: Record<string, string>;
      bodyText?: string;
      bodyBase64?: string;
    },
  ): Promise<{
    status: number;
    statusText: string;
    headers: Record<string, string>;
    body: string;
  }>;
  searchSyllabus(
    query: string,
    year: number,
    limit?: number,
    filters?: SyllabusFilters,
    language?: "en" | "ja",
  ): Promise<{ items: SyllabusCourse[]; total: number; truncated: boolean }>;
  syllabusCourse(url: string): Promise<SyllabusCourse>;
}

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
const requireSyllabusCourse = (value: unknown): SyllabusCourse => {
  const row = requireObject<Record<string, unknown>>("syllabus course", value);
  if (
    typeof row.externalId !== "string" ||
    typeof row.year !== "number" ||
    !["1Q", "2Q", "3Q", "4Q"].includes(String(row.quarter)) ||
    typeof row.code !== "string" ||
    typeof row.title !== "string" ||
    typeof row.instructor !== "string" ||
    typeof row.url !== "string" ||
    !Array.isArray(row.meetings)
  )
    throw new Error("syllabus course returned an invalid result.");
  if (row.content != null && !validSyllabusContent(row.content))
    throw new Error("syllabus content returned an invalid result.");
  return row as unknown as SyllabusCourse;
};
const requireSyllabusSearch = (value: unknown) => {
  const response = requireObject<Record<string, unknown>>(
    "search_syllabus",
    value,
  );
  if (
    !Array.isArray(response.items) ||
    typeof response.total !== "number" ||
    typeof response.truncated !== "boolean"
  )
    throw new Error("search_syllabus returned an invalid result.");
  return {
    items: response.items.map(requireSyllabusCourse),
    total: response.total,
    truncated: response.truncated,
  };
};

export function createTauriDesktopBridge(
  invoke: InvokeNative,
  listen: ListenNative,
  target: NativeTarget = window.__STWS_NATIVE_TARGET__ ?? "native-desktop",
): DesktopBridge {
  const result = async <T>(
    command: string,
    payload?: Record<string, unknown>,
  ) => requireObject<T>(command, await invoke(command, payload));
  const booleanCommand = async (
    command: string,
    payload?: Record<string, unknown>,
  ) => requireBoolean(command, await invoke(command, payload));
  return {
    runtime: "tauri",
    target,
    platform: navigator.platform,
    getRuntimeCapabilities: () => result("runtime_capabilities"),
    openExternal: async (url) => {
      await invoke("open_external", { url });
    },
    notify: (title, body) => booleanCommand("notify", { title, body }),
    requestNotificationPermission: async () => {
      const value = await invoke("request_notification_permission");
      if (!matchesNotificationPermission(value))
        throw new Error(
          "request_notification_permission returned an invalid result.",
        );
      return value;
    },
    schoolCredentialStatus: () => result("school_credential_status"),
    saveSchoolCredentials: (profile, credentials) =>
      booleanCommand("save_school_credentials", { profile, credentials }),
    deleteSchoolCredentials: (profile) =>
      booleanCommand("delete_school_credentials", { profile }),
    testSchoolConnection: (profile, credentials) =>
      result("test_school_connection", { profile, credentials }),
    loginSchool: (profile) => result("login_school", { profile, bounds: null }),
    ensureSchoolSession: (profile) =>
      booleanCommand("ensure_school_session", { profile }),
    onSchoolSession: (callback) => {
      let disposed = false;
      let unlisten: (() => void) | undefined;
      void listen<SchoolSessionEvent>("school-session", (payload) => {
        if (
          !disposed &&
          payload &&
          (payload.profile === "science" || payload.profile === "tokyo-tech") &&
          (payload.state === "active" || payload.state === "failed")
        )
          callback(payload);
      }).then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      });
      return () => {
        disposed = true;
        unlisten?.();
      };
    },
    importSchool: () => result("import_school"),
    cachedSchool: () => result("cached_school"),
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
    setSchoolWebZoom: (site, scale) =>
      booleanCommand("set_school_web_zoom", { site, scale }),
    onSchoolWebUrl: (callback) => {
      let disposed = false;
      let unlisten: (() => void) | undefined;
      void listen<string>("school-web-url", (payload) => {
        if (!disposed && typeof payload === "string") callback(payload);
      }).then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      });
      return () => {
        disposed = true;
        unlisten?.();
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
        if (disposed) fn();
        else unlisten = fn;
      });
      return () => {
        disposed = true;
        unlisten?.();
      };
    },
    configureSchoolWebPolicy: (policy) =>
      booleanCommand("configure_school_web_policy", { policy }),
    configureSchoolSchedule: (schedule) =>
      booleanCommand("configure_school_schedule", { schedule }),
    onScheduledSchoolResult: (callback) => {
      let disposed = false;
      let unlisten: (() => void) | undefined;
      void listen<SchoolImportResult>("scheduled-school-result", (payload) => {
        if (!disposed) callback(payload);
      }).then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      });
      return () => {
        disposed = true;
        unlisten?.();
      };
    },
    updateExportWorkspace: (workspace) =>
      booleanCommand("update_export_workspace", { workspace }),
    configureExportSchedule: (schedule) =>
      booleanCommand("configure_export_schedule", { schedule }),
    connectCloud: (provider, clientId, redirectUri) =>
      booleanCommand("connect_cloud", { provider, clientId, redirectUri }),
    cloudRequest: (provider, url, init) =>
      result("cloud_request", { provider, url, init }),
    searchSyllabus: async (query, year, limit = 100, filters, language) => {
      if (language !== undefined && language !== "en" && language !== "ja")
        throw new Error("Unsupported syllabus language.");
      if (
        filters &&
        Object.values(filters).some(
          (value) => value !== undefined && value !== "",
        )
      ) {
        const caps = await result<NativeCapabilitySnapshot>(
          "runtime_capabilities",
        );
        if (caps.syllabusFilters !== true)
          throw new Error(
            "Restart the updated macOS app to use syllabus filters.",
          );
      }
      return requireSyllabusSearch(
        await invoke("search_syllabus", {
          query,
          year,
          limit,
          filters,
          ...(language ? { language } : {}),
        }),
      );
    },
    syllabusCourse: async (url) =>
      requireSyllabusCourse(await invoke("syllabus_course", { url })),
  };
}

declare global {
  interface Window {
    /** Set by the trusted native bootstrap from its compile/runtime target. */
    __STWS_NATIVE_TARGET__?: NativeTarget;
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
