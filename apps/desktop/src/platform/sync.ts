/**
 * Core's sync planner as the shell sees it (`crates/app-core/src/plan`).
 * The shell sends hints and renders the snapshot; it never decides what is
 * due or how far along a read is. The types mirror the Rust serde contract
 * (`approach/core-deadline-plan`); these helpers only choose which row of the snapshot a
 * surface shows.
 */
import type {
  SchoolAuthProfile,
  SchoolSessionState,
  SignInPresentation,
} from "./school-session-contract";

export type SyncPriority = "background" | "visible" | "interactive";
export type SyncNetwork = "offline" | "wifi" | "cellular" | "unknown";
export type SyncSite = "science-lms" | "kyomu";

export type SyncView =
  | { kind: "today" }
  | { kind: "assignments" }
  | { kind: "timetable" }
  | { kind: "course"; lmsCourseId: number }
  | { kind: "finder"; year: number }
  | { kind: "grades" };

export type SyncScope =
  | { kind: "everything" }
  | { kind: "site"; site: SyncSite }
  | { kind: "course"; lmsCourseId: number }
  | { kind: "timetable" }
  | { kind: "references" }
  | { kind: "catalog"; year: number }
  | { kind: "lms-history"; year: number; month: number };

export type RetryPlan = { scopes: SyncScope[] };

export type SyncHint =
  | { kind: "foreground" }
  | { kind: "background" }
  | { kind: "suspend" }
  | { kind: "resume" }
  | { kind: "network"; network: SyncNetwork }
  | { kind: "power-saver"; on: boolean }
  | { kind: "open"; view: SyncView }
  | { kind: "refresh"; scope: SyncScope };

/** An i18n key plus arguments; Core never builds sentences. */
export type SyncLabel = { key: string; args: Record<string, unknown> | null };

/** What lets an account's waiting reads go on: the person (Retry, Connect,
 * changed credentials) or the end of a pause. Waiting for the network is
 * `offline`. */
export type AccountWait = { kind: "person" } | { kind: "paused"; until: string };

export type SyncHeld =
  | { kind: "offline" }
  | { kind: "cellular" }
  | { kind: "power-saver" }
  | { kind: "background" }
  | { kind: "backoff"; until: string }
  | { kind: "host-paused"; host: string; until: string }
  /** No credential on the key's path can serve it and none can be got now;
   * keys with a usable token or session go on. Not a failure, no backoff. */
  | { kind: "account"; profile: SchoolAuthProfile; wait: AccountWait }
  | { kind: "resetting" }
  | { kind: "prerequisite" };

export type SyncFailure = {
  /** `sync_runs.error_kind`: `timeout`, `http-503`, … */
  kind: string;
  reason: SyncLabel;
  keysFailed: number;
  retryAt: string | null;
  /** Core's diagnostic text: one `page: reason` line per failed page. */
  detail: string | null;
};

export type RunView = {
  run: string;
  /** `sync_state::Dataset::name()`, e.g. `lms.course`. */
  dataset: string;
  label: SyncLabel;
  priority: SyncPriority;
  done: number;
  /** Known chunk count; a determinate bar only when this is set. */
  total: number | null;
  current: SyncLabel | null;
  /** Narrows a run to a page: an LMS course id, offering id or year. */
  scope: string | null;
  held: SyncHeld | null;
  failure: SyncFailure | null;
  retry: RetryPlan | null;
};

export type SourceView = {
  /** `lms`, `kyomu`, `references`, `catalog`. */
  source: string;
  lastUpdated: string | null;
  running: boolean;
  held: SyncHeld | null;
  failure: SyncFailure | null;
  retry: RetryPlan | null;
  /** The account's session state for school sources. */
  session: SchoolSessionState | null;
  maintenance: string | null;
};

export type SyncWork =
  { kind: "resetting" | "problem" } | { kind: "active" | "held"; run: string };

export type SyncAttentionProblem =
  | {
      kind: "account";
      profile: SchoolAuthProfile;
      presentation: SignInPresentation | null;
      retryAt: string | null;
    }
  | {
      kind: "run-failed";
      dataset: string;
      failure: SyncFailure;
      retry: RetryPlan | null;
    }
  | {
      kind: "source-failed";
      source: string;
      failure: SyncFailure;
      retry: RetryPlan | null;
    };

export type SyncAttention = {
  primary: SyncWork | null;
  problem: SyncAttentionProblem | null;
  busy: boolean;
};

export type SyncSnapshot = {
  generation: number;
  resetting: boolean;
  attention: SyncAttention;
  runs: RunView[];
  sources: SourceView[];
};

export const EMPTY_SYNC: SyncSnapshot = {
  generation: 0,
  resetting: false,
  attention: { primary: null, problem: null, busy: false },
  runs: [],
  sources: [],
};

export const isSyncSnapshot = (value: unknown): value is SyncSnapshot =>
  Boolean(value) &&
  typeof value === "object" &&
  typeof (value as SyncSnapshot).attention?.busy === "boolean" &&
  Array.isArray((value as SyncSnapshot).runs) &&
  Array.isArray((value as SyncSnapshot).sources);

/** The link the webview can see. Unknown links stay `unknown`; Core treats
 * them by its own policy. */
export function networkHint(): SyncNetwork {
  if (typeof navigator === "undefined") return "unknown";
  if (!navigator.onLine) return "offline";
  const connection = (
    navigator as Navigator & {
      connection?: { type?: string; saveData?: boolean; metered?: boolean };
    }
  ).connection;
  if (!connection) return "unknown";
  if (
    connection.type === "cellular" ||
    connection.saveData === true ||
    connection.metered === true
  )
    return "cellular";
  if (connection.type === "wifi" || connection.type === "ethernet")
    return "wifi";
  return "unknown";
}

const working = (run: RunView) => !run.held && !run.failure;
/** What the one-line progress in the workstation footer shows. */
export type ProgressLine =
  | { kind: "idle" | "resetting" | "problem" }
  /** A visible run with a known size: a determinate bar. */
  | { kind: "bar"; run: RunView; done: number; total: number }
  /** A visible run of unknown size: its label, no bar. */
  | { kind: "busy"; run: RunView }
  /** Only background work: a small spinner and nothing else. */
  | { kind: "background"; run: RunView }
  /** Due work is waiting; the reason is the message. */
  | { kind: "held"; run: RunView; held: SyncHeld };

export function progressLine(snapshot: SyncSnapshot): ProgressLine {
  const work = snapshot.attention.primary;
  if (!work) return { kind: "idle" };
  if (!("run" in work)) return { kind: work.kind };
  const run = snapshot.runs.find((candidate) => candidate.run === work.run);
  if (!run) return { kind: "idle" };
  if (work.kind === "held")
    return run.held ? { kind: "held", run, held: run.held } : { kind: "idle" };
  if (run.priority === "background") return { kind: "background", run };
  return run.total === null
    ? { kind: "busy", run }
    : { kind: "bar", run, done: run.done, total: run.total };
}

/** The runs of one page: datasets by prefix, optionally narrowed by scope. */
export function runsFor(
  snapshot: SyncSnapshot,
  datasets: readonly string[],
  scope?: string,
): RunView[] {
  return snapshot.runs.filter(
    (run) =>
      datasets.some(
        (name) => run.dataset === name || run.dataset.startsWith(`${name}.`),
      ) &&
      (scope === undefined || run.scope === scope),
  );
}

/** In-page progress: counts summed over a page's runs that are working. */
export type PageProgress = { done: number; total: number | null } | null;
export function pageProgress(runs: RunView[]): PageProgress {
  const active = runs.filter(working);
  if (!active.length) return null;
  const total = active.every((run) => run.total !== null)
    ? active.reduce((sum, run) => sum + (run.total ?? 0), 0)
    : null;
  return { done: active.reduce((sum, run) => sum + run.done, 0), total };
}

export const failedRuns = (snapshot: SyncSnapshot) =>
  snapshot.runs.filter((run) => run.failure);
