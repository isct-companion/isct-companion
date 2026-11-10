import { QueryClient, type Query } from "@tanstack/react-query";
import type { WorkspaceChange } from "../generated/persistence";

class SupersededAnswer extends Error {
  constructor() {
    super("Core snapshot was superseded by a newer commit.");
  }
}

/**
 * The one cache of Core's answers (`approach/frontend-query-cache`). Core
 * decides when an answer is stale, so nothing here goes stale by time:
 *
 * 1. each answer names the changes it read; a commit or `workspace-changed`
 *    naming one of them invalidates it;
 * 2. each answer carries Core's commit sequence read before its data; one
 *    older than the newest change heard is fetched again;
 * 3. a reload (startup, a lost race, an import) or the window coming back
 *    refetches every query;
 * 4. no answer is copied elsewhere, and nothing is updated optimistically.
 */
export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: Infinity,
      gcTime: 5 * 60_000,
      // TanStack owns the wait and cancellation; a burst must neither spin
      // the UI thread nor turn a known-stale answer into an infinite cache hit.
      retry: (_count, error) => error instanceof SupersededAnswer,
      retryDelay: (attempt) => Math.min(50 * 2 ** attempt, 250),
      refetchOnWindowFocus: false,
      refetchOnReconnect: false,
    },
  },
});

/** What a cached answer holds besides its value. */
export type CoreAnswer<T> = {
  value: T;
  reads: readonly WorkspaceChange[];
  seq: number;
};

let latestSeq = 0;

const readsAny = (query: Query, changes: ReadonlySet<WorkspaceChange>) => {
  const answer = query.state.data as Partial<CoreAnswer<unknown>> | undefined;
  // An answer not here yet cannot say what it read; it is fetched again.
  if (!answer?.reads) return true;
  return answer.reads.some((change) => changes.has(change));
};

/** A commit landed: answers that read one of `changes` are fetched again. */
export function changesCommitted(
  changes: readonly WorkspaceChange[],
  seq: number | undefined,
) {
  if (typeof seq === "number" && seq > latestSeq) latestSeq = seq;
  if (!changes.length) return;
  const named = new Set(changes);
  void queryClient.invalidateQueries({
    predicate: (query) => readsAny(query, named),
  });
}

/** Everything may have changed (the store reloaded, the window came back):
 * every answer is fetched again. */
export function everythingChanged() {
  void queryClient.invalidateQueries();
}

/** Accept only an answer no older than the newest change heard. The query
 * client's retry policy waits and reads again while the query is needed. */
export async function freshAnswer<T>(
  load: () => Promise<CoreAnswer<T>>,
  signal?: AbortSignal,
): Promise<CoreAnswer<T>> {
  signal?.throwIfAborted();
  const answer = await load();
  signal?.throwIfAborted();
  if (answer.seq < latestSeq) throw new SupersededAnswer();
  return answer;
}

/** Forget every answer and the sequence heard; a test starts afresh. */
export function resetQueries() {
  queryClient.clear();
  latestSeq = 0;
}

// The window coming back may follow missed events (sleep, a hidden window).
if (typeof document !== "undefined")
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") everythingChanged();
  });
