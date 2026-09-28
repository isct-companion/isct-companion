import { useStore } from "zustand";
import { workspaceStore, type WorkspaceSnapshot } from "./workspace";

/** Select from the single workspace snapshot; no second React state copy. */
export function useWorkspace<T>(select: (state: WorkspaceSnapshot) => T): T {
  return useStore(workspaceStore, select);
}
