import type { SchoolRecords } from "../platform/desktop-bridge";
import type { Workspace } from "./model";
import type {
  CourseBatchChanges as GeneratedCourseBatchChanges,
  WorkspaceIntent as GeneratedWorkspaceIntent,
  WorkspaceMutation as GeneratedWorkspaceMutation,
  WorkspaceSnapshot as GeneratedWorkspaceSnapshot,
} from "../generated/persistence";

export type CourseBatchChanges = GeneratedCourseBatchChanges;
export type WorkspaceIntent = GeneratedWorkspaceIntent;
export type WorkspaceMutation = GeneratedWorkspaceMutation;
export type CoreWorkspaceSnapshot = Omit<
  GeneratedWorkspaceSnapshot,
  "workspace" | "schoolRecords"
> & {
  workspace: Workspace;
  schoolRecords: SchoolRecords;
};
