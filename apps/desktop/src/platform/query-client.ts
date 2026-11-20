import { QueryClient } from "@tanstack/react-query";
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
 *    older than a change it reads is fetched again;
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

const latest = new Map<WorkspaceChange, number>();

/** Only a newer notice for a dependency can invalidate an answer. TanStack
 * keeps the read already in flight; freshAnswer checks what it covered. */
export function changesCommitted(
  changes: readonly WorkspaceChange[],
  seq: number,
) {
  const advanced = changes.filter((name) => seq > (latest.get(name) ?? -1));
  if (advanced.length) {
    for (const name of advanced) latest.set(name, seq);
    void queryClient.invalidateQueries(
      {
        predicate: (query) => {
          const answer = query.state.data as CoreAnswer<unknown> | undefined;
          return (
            !answer ||
            (answer.seq < seq &&
              answer.reads.some((name) => advanced.includes(name)))
          );
        },
      },
      { cancelRefetch: false },
    );
  }
}

/** A missed event or a resumed clock starts a fresh read, including for a
 * cold query whose first answer hasn't arrived yet. */
export function everythingChanged() {
  void queryClient.cancelQueries();
  void queryClient.invalidateQueries();
}

/** Accept only an answer no older than a change it reads. The query
 * client's retry policy waits and reads again while the query is needed. */
export async function freshAnswer<T>(
  load: () => Promise<CoreAnswer<T>>,
  signal?: AbortSignal,
): Promise<CoreAnswer<T>> {
  signal?.throwIfAborted();
  const answer = await load();
  signal?.throwIfAborted();
  if (answer.reads.some((name) => answer.seq < (latest.get(name) ?? 0)))
    throw new SupersededAnswer();
  return answer;
}

/** Forget every answer and the sequence heard; a test starts afresh. */
export function resetQueries() {
  queryClient.clear();
  latest.clear();
}
