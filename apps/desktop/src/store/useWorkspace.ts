import { useStore } from "zustand";
import { workspaceStore, type WorkspaceState } from "./workspace";

/** Select from the single workspace snapshot; no second React state copy. */
export function useWorkspace<T>(select: (state: WorkspaceState) => T): T {
  return useStore(workspaceStore, select);
}
