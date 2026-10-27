import { useStore } from "zustand";
import { workspaceStore, type WorkspaceState } from "./workspace";

/** Select from the workspace store (revision and write state); records live
 * in the query cache, not here. */
export function useWorkspace<T>(select: (state: WorkspaceState) => T): T {
  return useStore(workspaceStore, select);
}
