import { clockForPeriods } from "../core/user-schedule";
import type { FinderCourse } from "../core/course-query";
import type { SyllabusFilters } from "../core/syllabus-filters";
import { validSyllabusContent } from "../core/syllabus-content";
import { pickLocalized, type Localized } from "../core/model";
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
  | "tsubame"
  | "syllabus";
export type Schedule = "off" | "launch" | "hourly";
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
/** One extractor result from LMS or one 教務Web page. */
export type SchoolIngestOutcome = {
  site: SchoolSite;
  kind: string;
  state: "ok" | "reconnect-required" | "error";
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
  syllabusFilters?: boolean;
  matrixOcr: boolean;
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
  /** Load the view's previous page as a fresh GET; false when there is none. */
  schoolWebBack?(site: SchoolSite): Promise<boolean>;
  setSchoolWebZoom?(site: SchoolSite, scale: number): Promise<boolean>;
  onSchoolWebUrl?(callback: (url: string) => void): () => void;
  /** A school page asked for a new window showing another service's page. */
  onSchoolWebOpen?(
    callback: (request: { site: SchoolSite; url: string }) => void,
  ): () => void;
  onSchoolLoginStatus?(callback: (event: SchoolLoginEvent) => void): () => void;
  onSchoolDownload?(callback: (event: SchoolDownloadEvent) => void): () => void;
  /** Shows a file saved from a school page this session in Finder; false for any other path. */
  revealSchoolDownload?(path: string): Promise<boolean>;
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
  /**
   * Runs the LMS or 教務Web extractor in the hidden school view and waits for
   * the result (one outcome per 教務Web page). Expired sessions are renewed
   * with the saved credentials on the way.
   */
  pullSchool?(site: "science-lms" | "kyomu"): Promise<SchoolIngestOutcome[]>;
  schoolRecords?(): Promise<SchoolRecords>;
  /** Shows (or clears, with `[]`) the registration queue on 教務Web pages. */
  setKyomuGuide?(items: KyomuGuideItem[]): Promise<boolean>;
  /** An extractor finished while the person browsed LMS / 教務Web. */
  onSchoolIngest?(callback: (outcome: SchoolIngestOutcome) => void): () => void;
  configureSchoolSchedule(schedule: Schedule): Promise<boolean>;
  onScheduledSchoolResult(
    callback: (result: SchoolImportResult) => void,
  ): () => void;
  updateExportWorkspace(workspace: unknown): Promise<boolean>;
  configureExportSchedule(schedule: Schedule): Promise<boolean>;
  /** Saves a validated workspace export under app-owned local storage. */
  writeScheduledExport(): Promise<string>;
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
  syllabusCourse(url: string, language?: "en" | "ja"): Promise<SyllabusCourse>;
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
    copySchoolMatrix: () => booleanCommand("copy_school_matrix"),
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
        if (disposed) quietly(fn);
        else unlisten = fn;
      });
      return () => {
        disposed = true;
        if (unlisten) quietly(unlisten);
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
    setSchoolWebZoom: (site, scale) =>
      booleanCommand("set_school_web_zoom", { site, scale }),
    onSchoolWebUrl: (callback) => {
      let disposed = false;
      let unlisten: (() => void) | undefined;
      void listen<string>("school-web-url", (payload) => {
        if (!disposed && typeof payload === "string") callback(payload);
      }).then((fn) => {
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
    pullSchool: async (site) => {
      const value = await invoke("pull_school", { site });
      if (!Array.isArray(value))
        throw new Error("pull_school returned an invalid result.");
      return value as SchoolIngestOutcome[];
    },
    schoolRecords: async () => {
      const value = requireObject<Partial<SchoolRecords>>(
        "school_records",
        await invoke("school_records"),
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
        if (disposed) quietly(fn);
        else unlisten = fn;
      });
      return () => {
        disposed = true;
        if (unlisten) quietly(unlisten);
      };
    },
    updateExportWorkspace: (workspace) =>
      booleanCommand("update_export_workspace", { workspace }),
    configureExportSchedule: (schedule) =>
      booleanCommand("configure_export_schedule", { schedule }),
    writeScheduledExport: async () => {
      const value = await invoke("write_scheduled_export");
      if (typeof value !== "string" || !value)
        throw new Error("write_scheduled_export returned an invalid result.");
      return value;
    },
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
      const locale = language ?? "en";
      const refresh = Boolean(
        query.trim() ||
        (filters &&
          Object.values(filters).some(
            (value) => value !== undefined && value !== "",
          )),
      );
      const response = requireObject<{
        items: unknown[];
        total: number;
        truncated: boolean;
      }>(
        "catalog_search",
        await invoke("catalog_search", {
          query,
          year,
          limit,
          filters,
          refresh,
        }),
      );
      return {
        items: response.items.map((item) => catalogCourse(item, locale)),
        total: response.total,
        truncated: response.truncated,
      };
    },
    syllabusCourse: async (url, language = "en") => {
      const synced = requireObject<{
        offering: unknown;
        content: { lang: string; content: unknown }[];
      }>(
        "catalog_sync_offering",
        await invoke("catalog_sync_offering", { id: url }),
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
        await invoke("catalog_query", {
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
      const warnings = await invoke("catalog_sync_offerings", { ids });
      return Array.isArray(warnings) ? warnings.map(String) : [];
    },
    catalogOfferings: async (year) => {
      const rows = await invoke("catalog_offerings", { year });
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
