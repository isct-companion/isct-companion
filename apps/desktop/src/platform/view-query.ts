import { useEffect, useState } from "react";
import { useWorkspace } from "../store/useWorkspace";
import type { ViewRevisions } from "../store/workspace";
import type { WorkspaceChange } from "../generated/persistence";
import type { QueryState } from "./core-query";

/** The revision of the records a view family reads: a whole snapshot read,
 * or a patch of any change it names, makes a new one. */
export const viewRevision = (
  revisions: ViewRevisions,
  reads: readonly WorkspaceChange[],
) =>
  [revisions.all ?? 0, ...reads.map((change) => revisions[change] ?? 0)].join(
    ":",
  );

// React owns view retention; Core owns the data. This bounded transport cache
// is shared by finder and schedule hooks so late responses cannot mix snapshots.
const requests = new Map<string, Promise<unknown>>();
export function useViewQuery<T>(
  namespace: string,
  reads: readonly WorkspaceChange[],
  input: unknown,
  load: () => Promise<T>,
  enabled = true,
  refresh: unknown = 0,
): QueryState<T> {
  const revision = useWorkspace((state) =>
    viewRevision(state.viewRevisions, reads),
  );
  const key = JSON.stringify([namespace, revision, input, refresh]);
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<QueryState<T> & { key?: string }>({
    pending: enabled,
  });
  useEffect(() => {
    if (!enabled) return;
    let active = true;
    let request = requests.get(key) as Promise<T> | undefined;
    if (!request) {
      request = load();
      requests.set(key, request);
      while (requests.size > 128)
        requests.delete(requests.keys().next().value!);
      const owned = request;
      void request.catch(() => {
        if (requests.get(key) === owned) requests.delete(key);
      });
    }
    void request.then(
      (value) => {
        if (active) setState({ key, value, pending: false });
      },
      (reason) => {
        if (active)
          setState((previous) => ({
            key,
            value: previous.value,
            pending: false,
            error: reason instanceof Error ? reason.message : String(reason),
          }));
      },
    );
    return () => {
      active = false;
    };
  }, [key, enabled, attempt]);
  const retry = () => {
    requests.delete(key);
    setState((previous) => ({ ...previous, pending: true, error: undefined }));
    setAttempt((value) => value + 1);
  };
  if (!enabled) return { pending: false };
  return { ...state, pending: state.key !== key || state.pending, retry };
}
