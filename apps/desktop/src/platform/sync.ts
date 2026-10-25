/**
 * Core's sync planner as the shell sees it (`crates/isct-core/src/sync_engine`).
 * The shell sends hints and renders the snapshot; it never decides what is
 * due or how far along a read is. The types mirror the Rust serde contract
 * (INTERFACES-CP3); these helpers only choose which row of the snapshot a
 * surface shows.
 */
import type { SchoolAuthProfile } from "./desktop-bridge";

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
  | { kind: "catalog"; year: number };

export type SyncHint =
  | { kind: "wake" }
  | { kind: "foreground" }
  | { kind: "background" }
  | { kind: "suspend" }
  | { kind: "resume" }
  | { kind: "network"; network: SyncNetwork }
  | { kind: "power-saver"; on: boolean }
  | { kind: "signed-in"; profile: SchoolAuthProfile }
  | { kind: "open"; view: SyncView }
  | { kind: "refresh"; scope: SyncScope };

/** An i18n key plus arguments; Core never builds sentences. */
export type SyncLabel = { key: string; args: Record<string, unknown> | null };

export type SyncHeld =
  | { kind: "offline" }
  | { kind: "cellular" }
  | { kind: "power-saver" }
  | { kind: "background" }
  | { kind: "backoff"; until: string }
  | { kind: "host-paused"; host: string; until: string }
  | { kind: "sign-in-required"; profile: SchoolAuthProfile }
  | { kind: "resetting" }
  | { kind: "prerequisite" };

export type SyncFailure = {
  /** `sync_runs.error_kind`: `timeout`, `http-503`, … */
  kind: string;
  keysFailed: number;
  retryAt: string | null;
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
};

export type SourceView = {
  /** `lms`, `kyomu`, `references`, `catalog`. */
  source: string;
  lastUpdated: string | null;
  running: boolean;
  held: SyncHeld | null;
  failure: SyncFailure | null;
  /** School sign-in state (`active`, `signed-out`, `maintenance`, …). */
  session: string | null;
  maintenance: string | null;
};

export type SyncSnapshot = {
  generation: number;
  resetting: boolean;
  runs: RunView[];
  sources: SourceView[];
};

export const EMPTY_SYNC: SyncSnapshot = {
  generation: 0,
  resetting: false,
  runs: [],
  sources: [],
};

export const isSyncSnapshot = (value: unknown): value is SyncSnapshot =>
  Boolean(value) &&
  typeof value === "object" &&
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
const RANK: Record<SyncPriority, number> = {
  interactive: 2,
  visible: 1,
  background: 0,
};

/** What the one-line progress in the workstation footer shows. */
export type ProgressLine =
  | { kind: "idle" }
  /** A visible run with a known size: a determinate bar. */
  | { kind: "bar"; run: RunView; done: number; total: number }
  /** A visible run of unknown size: its label, no bar. */
  | { kind: "busy"; run: RunView }
  /** Only background work: a small spinner and nothing else. */
  | { kind: "background"; run: RunView }
  /** Due work is waiting; the reason is the message. */
  | { kind: "held"; run: RunView; held: SyncHeld };

export function progressLine(snapshot: SyncSnapshot): ProgressLine {
  const ordered = [...snapshot.runs].sort(
    (a, b) => RANK[b.priority] - RANK[a.priority],
  );
  const foreground = ordered.find(
    (run) => run.priority !== "background" && working(run),
  );
  if (foreground)
    return foreground.total !== null
      ? {
          kind: "bar",
          run: foreground,
          done: foreground.done,
          total: foreground.total,
        }
      : { kind: "busy", run: foreground };
  const held = ordered.find((run) => run.held && run.priority !== "background");
  if (held) return { kind: "held", run: held, held: held.held! };
  const background = ordered.find(working);
  if (background) return { kind: "background", run: background };
  const waiting = ordered.find((run) => run.held);
  if (waiting) return { kind: "held", run: waiting, held: waiting.held! };
  return { kind: "idle" };
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

/** The refresh a failed run's Retry button sends. */
export function retryScope(run: RunView): SyncScope {
  if (run.dataset.startsWith("lms."))
    return run.scope && /^\d+$/.test(run.scope)
      ? { kind: "course", lmsCourseId: Number(run.scope) }
      : { kind: "site", site: "science-lms" };
  if (run.dataset.startsWith("kyomu."))
    return { kind: "site", site: "kyomu" };
  if (run.dataset.startsWith("reference")) return { kind: "references" };
  if (run.dataset === "catalog.official" && /^\d{4}$/.test(run.scope ?? ""))
    return { kind: "catalog", year: Number(run.scope) };
  return { kind: "everything" };
}

export const failedRuns = (snapshot: SyncSnapshot) =>
  snapshot.runs.filter((run) => run.failure);
