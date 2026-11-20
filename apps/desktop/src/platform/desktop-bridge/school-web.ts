import {
  quietly,
  subscribe,
  type BridgeCommands,
  type ListenNative,
} from "./helpers";

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
/** School-webview download progress stays associated with its own surface. */
export type SchoolDownloadEvent = DownloadEvent & { site: SchoolSite };
export type DownloadEvent = {
  state: "started" | "done" | "failed";
  name: string;
  /** Set on `done`: the saved file, accepted by `revealDownload`. */
  path?: string;
};
/** One extractor result from LMS or one 教務Web page. */
export type SchoolIngestOutcome = {
  site: SchoolSite;
  kind: string;
  state: "ok" | "unavailable" | "maintenance" | "reconnect-required" | "error";
  changed: number;
  message: string;
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
  /** The page in front is outside the school sites: the view has none of
   * the person's other accounts, so Open in browser is offered plainly. */
  outside: boolean;
  otp: SchoolOtpEnrollment | null;
};

export type SchoolOtpEnrollment = {
  id: number;
  owner: string | null;
  stage:
    | "sign-in"
    | "checking"
    | "setting-up"
    | "saving"
    | "complete"
    | "account-mismatch"
    | "incomplete"
    | "unknown"
    | "save-failed";
};
export type SchoolOtpIntent = "stop" | "retry-save";

/** How a surface brings a site's view on screen. Core resumes, goes to the
 * URL, or (for "return") resumes only when this surface showed it last. */
export type SchoolWebEntry =
  | { kind: "resume" }
  | { kind: "go"; url: string }
  | { kind: "return"; url: string }
  /** The site's start page as it is: Core writes no identity and signs in
   * nowhere, so the person signs in on the page by hand. */
  | { kind: "unsigned" }
  | { kind: "otpSetup"; username: string };

export type SchoolWebBounds = {
  x: number;
  y: number;
  width: number;
  height: number;
};
const schoolWebSteps: readonly unknown[] = ["yes", "no", "unknown"];
const isSchoolWebPage = (value: unknown): value is SchoolWebPage => {
  if (!value || typeof value !== "object") return false;
  const page = value as Record<string, unknown>;
  const notice = page.notice as Record<string, unknown> | null | undefined;
  const otp = page.otp as Record<string, unknown> | null | undefined;
  const access = page.access as Record<string, unknown> | null | undefined;
  return (
    typeof page.site === "string" &&
    Number.isSafeInteger(page.revision) &&
    (page.url === null || typeof page.url === "string") &&
    Number.isSafeInteger(page.popups) &&
    schoolWebSteps.includes(page.back) &&
    schoolWebSteps.includes(page.forward) &&
    typeof page.signIn === "boolean" &&
    typeof page.outside === "boolean" &&
    (otp === null ||
      (typeof otp === "object" &&
        Number.isSafeInteger(otp.id) &&
        (otp.owner === null || typeof otp.owner === "string") &&
        [
          "sign-in",
          "checking",
          "setting-up",
          "saving",
          "complete",
          "account-mismatch",
          "incomplete",
          "unknown",
          "save-failed",
        ].includes(String(otp.stage)))) &&
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

export interface SchoolWebBridge {
  resolveSchoolUrl(
    url: string,
    site: SchoolSite,
  ): Promise<{ site: SchoolSite; url: string }>;
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
  schoolOtpIntent?(id: number, intent: SchoolOtpIntent): Promise<boolean>;
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
  onSchoolDownload?(callback: (event: SchoolDownloadEvent) => void): () => void;
  /** Locates a file saved to Downloads this session; false for unknown or moved files. */
  revealDownload?(path: string): Promise<boolean>;
  /** Shows (or clears, with `[]`) the registration queue on 教務Web pages. */
  setKyomuGuide?(items: KyomuGuideItem[]): Promise<boolean>;
  /** An extractor finished while the person browsed LMS / 教務Web. */
  onSchoolIngest?(callback: (outcome: SchoolIngestOutcome) => void): () => void;
}

export function schoolWebCommands(
  { result, booleanCommand }: BridgeCommands,
  listen: ListenNative,
): SchoolWebBridge {
  return {
    resolveSchoolUrl: (url, site) =>
      result("resolve_school_url", { url, site }),
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
    schoolOtpIntent: (id, intent) =>
      booleanCommand("school_otp_intent", { id, intent }),
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
    onSchoolWebNotice: (callback) =>
      subscribe<SchoolWebNotice>(
        listen,
        "school-web-notice",
        (payload) => payload && typeof payload.site === "string" &&
          payload.kind === "reopened" && typeof payload.url === "string"
            ? payload
            : undefined,
        callback,
      ),
    onSchoolDownload: (callback) =>
      subscribe<SchoolDownloadEvent>(
        listen,
        "school-download",
        (payload) => {
          if (
            payload &&
            typeof payload.site === "string" &&
            (payload.state === "started" ||
              payload.state === "done" ||
              payload.state === "failed") &&
            typeof payload.name === "string" &&
            (payload.path === undefined || typeof payload.path === "string") &&
            (payload.state !== "done" || typeof payload.path === "string")
          ) {
            const { site, state, name, path } = payload;
            return { site, state, name, path };
          }
        },
        callback,
      ),
    revealDownload: (path) => booleanCommand("reveal_download", { path }),
    setKyomuGuide: (items) => booleanCommand("set_kyomu_guide", { items }),
    onSchoolIngest: (callback) =>
      subscribe<SchoolIngestOutcome>(
        listen,
        "school-ingest",
        (payload) =>
          payload && typeof payload.state === "string" ? payload : undefined,
        callback,
      ),
    configureSchoolWebPolicy: (policy) =>
      booleanCommand("configure_school_web_policy", { policy }),
  };
}
