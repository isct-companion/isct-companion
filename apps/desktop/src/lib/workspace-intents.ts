import type { SchoolRecords } from "../platform/desktop-bridge";
import type { Workspace } from "./model";
import type {
  CourseBatchChanges as GeneratedCourseBatchChanges,
  WorkspaceIntent as GeneratedWorkspaceIntent,
  WorkspaceMutation as GeneratedWorkspaceMutation,
  WorkspacePatch as GeneratedWorkspacePatch,
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
/** Only the fields the requested changes feed, to merge over a snapshot. */
export type CoreWorkspacePatch = Omit<
  GeneratedWorkspacePatch,
  "workspace" | "schoolRecords"
> & {
  workspace: Partial<Workspace>;
  schoolRecords: Partial<SchoolRecords>;
};
