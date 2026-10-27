import type {
  CourseBatchChanges as GeneratedCourseBatchChanges,
  WorkspaceCommit as GeneratedWorkspaceCommit,
  WorkspaceIntent as GeneratedWorkspaceIntent,
  WorkspaceMutation as GeneratedWorkspaceMutation,
} from "../generated/persistence";

export type CourseBatchChanges = GeneratedCourseBatchChanges;
export type WorkspaceIntent = GeneratedWorkspaceIntent;
export type WorkspaceMutation = GeneratedWorkspaceMutation;
/** A committed intent: what it changed, the revision the next intent is
 * made against, and Core's commit sequence. */
export type CoreWorkspaceCommit = GeneratedWorkspaceCommit;
