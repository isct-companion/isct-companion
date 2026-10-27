import {
  keepPreviousData,
  useQuery,
  type QueryKey,
} from "@tanstack/react-query";
import type { ViewRevisions } from "../store/workspace";
import type { WorkspaceChange } from "../generated/persistence";
import type { QueryState } from "./core-query";
import { freshAnswer, queryClient, type CoreAnswer } from "./query-client";

/** The revision of the records a view family reads: a reload, or a commit
 * naming any change it reads, makes a new one. */
export const viewRevision = (
  revisions: ViewRevisions,
  reads: readonly WorkspaceChange[],
) =>
  [revisions.all ?? 0, ...reads.map((change) => revisions[change] ?? 0)].join(
    ":",
  );

/** A view's answer and the changes Core computed it from. */
export type ViewAnswer<T> = CoreAnswer<T>;

/** Core's answer to `key`, cached until Core says what it read changed
 * (`query-client`). While a new key or a refresh loads, the last answer
 * stays on screen; a late answer for an old key never replaces it. */
export function useViewQuery<T>(
  key: QueryKey,
  load: () => Promise<ViewAnswer<T>>,
  enabled = true,
): QueryState<T> {
  const query = useQuery(
    {
      queryKey: key,
      queryFn: () => freshAnswer(load),
      enabled,
      placeholderData: keepPreviousData,
    },
    queryClient,
  );
  if (!enabled) return { pending: false };
  return {
    value: query.data?.value,
    pending: query.isFetching || query.isPlaceholderData || query.isPending,
    error: query.error
      ? query.error instanceof Error
        ? query.error.message
        : String(query.error)
      : undefined,
    retry: () => void query.refetch(),
  };
}
