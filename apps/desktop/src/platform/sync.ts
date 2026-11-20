import type {
  RunView,
  SyncHeld,
  SyncNetwork,
  SyncSnapshot,
} from "../generated/persistence";

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
