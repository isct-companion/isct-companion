import { getDesktopBridge } from "../platform/desktop-bridge";
import type {
  SchoolRecords,
  WorkspaceChangedEvent,
} from "../platform/desktop-bridge";
import type { Workspace } from "../lib/model";
import type { ClassGroup } from "../generated/persistence";
import { repository } from "../storage/repository";
import type {
  CoreWorkspacePatch,
  CoreWorkspaceSnapshot,
  WorkspaceIntent,
  WorkspaceMutation,
} from "../lib/workspace-intents";
import type { WorkspaceChange } from "../generated/persistence";
import { requireAction } from "../platform/actions";
import { detectPlatform } from "../platform/capabilities";
import { createStore } from "zustand/vanilla";

export type WorkspaceSnapshot = {
  readonly workspace?: Workspace;
  readonly schoolRecords?: SchoolRecords;
  readonly status: "idle" | "loading" | "ready" | "error";
  /** Baseline for user intents; school and reference ingests keep it. */
  readonly revision: string;
  /** Changes with any snapshot content, whole or patched. */
  readonly contentRevision: string;
  /** Counts patches per change name; a whole snapshot counts as `all`.
   * Read-model caches key on the names their views read. */
  readonly viewRevisions: ViewRevisions;
  readonly error?: string;
};
export type ViewRevisions = Readonly<
  Partial<Record<WorkspaceChange | "all", number>>
>;
export type WorkspaceEdit = (draft: Workspace) => Workspace | void;

type WorkspaceService = {
  snapshot(): Promise<CoreWorkspaceSnapshot>;
  records(changes: readonly WorkspaceChange[]): Promise<CoreWorkspacePatch>;
  mutate(mutation: WorkspaceMutation): Promise<CoreWorkspaceSnapshot>;
};

export function createWorkspaceStore(service: WorkspaceService) {
  const state = createStore<WorkspaceSnapshot>(() =>
    Object.freeze({
      status: "idle",
      revision: "",
      contentRevision: "",
      viewRevisions: {},
    }),
  );
  let loading: Promise<void> | undefined;
  let writes: Promise<unknown> = Promise.resolve();
  let refreshing: Promise<SchoolRecords | undefined> | undefined;
  let refreshRequest = 0;
  let patching: Promise<void> | undefined;
  const unpatched = new Set<WorkspaceChange>();
  const localMutationRevisions = new Set<string>();
  const publish = (next: WorkspaceSnapshot) => {
    state.setState(Object.freeze(next), true);
  };
  const adopt = (snapshot: CoreWorkspaceSnapshot) => {
    const { viewRevisions } = state.getState();
    publish({
      workspace: snapshot.workspace,
      schoolRecords: snapshot.schoolRecords,
      status: "ready",
      revision: snapshot.revision,
      contentRevision: snapshot.contentRevision,
      viewRevisions: { ...viewRevisions, all: (viewRevisions.all ?? 0) + 1 },
      error: undefined,
    });
  };
  const mutate = (
    intent: WorkspaceIntent | ((workspace: Workspace) => WorkspaceIntent),
  ) => {
    const operation = writes.then(async () => {
      const current = state.getState();
      if (!current.workspace || !current.revision)
        throw new Error("Workspace is not ready.");
      try {
        const snapshot = await service.mutate({
          expectedRevision: current.revision,
          intent:
            typeof intent === "function"
              ? intent(structuredClone(current.workspace))
              : intent,
        });
        adopt(snapshot);
        localMutationRevisions.add(snapshot.revision);
        if (localMutationRevisions.size > 32)
          localMutationRevisions.delete(
            localMutationRevisions.values().next().value!,
          );
        return snapshot;
      } catch (error) {
        // The intent may have lost a race with school, cloud, or another
        // native writer. Recover the authoritative state for user review;
        // never replay the failed intent automatically.
        try {
          adopt(await service.snapshot());
        } catch {
          // Preserve the original mutation error and the last valid cache.
        }
        throw error;
      }
    });
    writes = operation.catch(() => undefined);
    return operation;
  };
  const reload = () => {
    refreshRequest++;
    if (refreshing) return refreshing;
    refreshing = (async () => {
      for (;;) {
        await writes;
        const before = state.getState();
        if (before.error && before.workspace) throw new Error(before.error);
        const request = refreshRequest;
        const snapshot = await service.snapshot();
        if (
          state.getState().contentRevision !== before.contentRevision ||
          request !== refreshRequest
        )
          continue;
        adopt(snapshot);
        return snapshot.schoolRecords;
      }
    })().finally(() => {
      refreshing = undefined;
    });
    return refreshing;
  };
  /** Re-reads only the records `changes` feed and merges them over the
   * snapshot; views that read none of the names keep their answers. Changes
   * heard while a read is in flight join the next read. */
  const patch = (changes: readonly WorkspaceChange[]) => {
    for (const change of changes) unpatched.add(change);
    if (patching || !unpatched.size) return patching ?? Promise.resolve();
    const run = async () => {
      try {
        while (unpatched.size) {
          await writes;
          const before = state.getState();
          // Opening reads everything; nothing to patch yet.
          if (!before.workspace || !before.schoolRecords) {
            unpatched.clear();
            return;
          }
          const requested = [...unpatched];
          unpatched.clear();
          let result: CoreWorkspacePatch;
          try {
            result = await service.records(requested);
          } catch (error) {
            // Keep the names for the next change to retry with.
            for (const change of requested) unpatched.add(change);
            throw error;
          }
          const current = state.getState();
          if (current.contentRevision !== before.contentRevision) {
            // A mutation or a whole read landed meanwhile; this patch may be
            // older than it, so read the same records again.
            for (const change of requested) unpatched.add(change);
            continue;
          }
          const viewRevisions: Partial<
            Record<WorkspaceChange | "all", number>
          > = { ...current.viewRevisions };
          for (const change of requested)
            viewRevisions[change] = (viewRevisions[change] ?? 0) + 1;
          publish({
            ...current,
            workspace: { ...current.workspace!, ...result.workspace },
            schoolRecords: {
              ...current.schoolRecords!,
              ...result.schoolRecords,
            },
            revision: result.revision,
            contentRevision: result.contentRevision,
            viewRevisions,
          });
        }
      } finally {
        // Cleared in the same turn the loop ends, so a change heard right
        // after starts a new read instead of joining a finished one.
        patching = undefined;
      }
    };
    patching = run();
    return patching;
  };
  const handleWorkspaceChanged = async (event: WorkspaceChangedEvent) => {
    if (event.source !== "user-intent") return patch(event.changes);
    if (!event.revision) {
      await reload();
      return;
    }
    // Core emits before the native command resolves. Wait for the command's
    // authoritative snapshot, then read only if this came from another client.
    await writes;
    if (
      state.getState().revision === event.revision ||
      localMutationRevisions.has(event.revision)
    )
      return;
    await reload();
  };
  return {
    getState: state.getState,
    getInitialState: state.getInitialState,
    subscribe: state.subscribe,
    initialize() {
      if (state.getState().workspace) return Promise.resolve();
      if (loading) return loading;
      publish({ ...state.getState(), status: "loading", error: undefined });
      loading = service
        .snapshot()
        .then(adopt)
        .catch(() => {
          publish({
            ...state.getState(),
            status: "error",
            error:
              "Workspace could not be opened. Stored data was preserved; retry opening.",
          });
        })
        .finally(() => {
          loading = undefined;
        });
      return loading;
    },
    reload,
    handleWorkspaceChanged,
    mutate,
    export() {
      const workspace = state.getState().workspace;
      if (!workspace) throw new Error("Workspace is not ready.");
      return JSON.stringify(workspace, null, 2);
    },
    async flush() {
      await writes;
      const error = state.getState().error;
      if (error) throw new Error(error);
    },
  };
}

const desktopBridge =
  typeof window === "undefined" ? undefined : getDesktopBridge();
const emptySchoolRecords = (): SchoolRecords => ({
  lmsCourses: [],
  registrations: [],
  registrationStates: [],
  grades: [],
  gpaByTerm: [],
  requirements: [],
  freshness: {},
  pageAvailability: {},
});

/** Browser preview has no Core process and exposes a read-only sample. */
export function createBrowserPreviewService(
  previewRepository: typeof repository,
): WorkspaceService {
  let workspace: Workspace | undefined;
  const snapshot = (): CoreWorkspaceSnapshot => ({
    revision: `preview:${JSON.stringify(workspace).length}`,
    contentRevision: `preview:${JSON.stringify(workspace).length}`,
    workspace: structuredClone(workspace!),
    schoolRecords: emptySchoolRecords(),
  });
  return {
    async snapshot() {
      workspace ??= await previewRepository.load();
      return snapshot();
    },
    async records() {
      // Nothing changes underneath a preview.
      return { ...snapshot(), workspace: {}, schoolRecords: {} };
    },
    async mutate() {
      throw new Error("Workspace edits need the native app.");
    },
  };
}
const browserPreviewService = createBrowserPreviewService(repository);
const internalWorkspaceStore = createWorkspaceStore(
  desktopBridge
    ? {
        snapshot: () => desktopBridge.workspaceSnapshot(),
        records: (changes) => desktopBridge.workspaceRecords(changes),
        mutate: (mutation) => desktopBridge.mutateWorkspace(mutation),
      }
    : browserPreviewService,
);

/** Read/lifecycle surface. Mutations are available only through workspaceActions. */
export const workspaceStore = {
  getState: internalWorkspaceStore.getState,
  getInitialState: internalWorkspaceStore.getInitialState,
  subscribe: internalWorkspaceStore.subscribe,
  initialize: internalWorkspaceStore.initialize,
  reload: internalWorkspaceStore.reload,
  handleWorkspaceChanged: internalWorkspaceStore.handleWorkspaceChanged,
  flush: internalWorkspaceStore.flush,
};

export const workspaceActions = {
  /** Refresh canonical workspace and parsed school records in one publication. */
  refreshSchool() {
    return internalWorkspaceStore.reload();
  },
  mutate(intent: WorkspaceIntent) {
    return internalWorkspaceStore.mutate(intent);
  },
  editCalendar(edit: WorkspaceEdit) {
    requireAction("edit-calendar-advanced", detectPlatform());
    const current = internalWorkspaceStore.getState().workspace;
    if (!current) return Promise.reject(new Error("Workspace is not ready."));
    const draft = structuredClone(current);
    const next = edit(draft) ?? draft;
    return internalWorkspaceStore.mutate({
      kind: "editAcademicCalendar",
      academicProfile: next.academicProfile,
      exceptions: next.exceptions,
    });
  },
  setStudyProfile(
    admissionYear: number | undefined,
    studyEnrollments: Workspace["academicProfile"]["studyEnrollments"],
  ) {
    requireAction("change-local-preference", detectPlatform());
    return internalWorkspaceStore.mutate({
      kind: "setStudyProfile",
      admissionYear: admissionYear ?? null,
      studyEnrollments: studyEnrollments ?? [],
    });
  },
  updateSettings(edit: WorkspaceEdit) {
    requireAction("change-local-preference", detectPlatform());
    // Resolve form edits inside the transport queue, after adopting the preceding
    // Core result. Two immediate preference edits must not copy the same old snapshot.
    // The whole object replaces the stored one, so cleared keys stay cleared;
    // Core validates the values.
    return internalWorkspaceStore.mutate((draft) => ({
      kind: "replaceSettings",
      settings: (edit(draft) ?? draft).settings,
    }));
  },
  /** The red/white group of a semester's 物理学演習・実験 ("2026-3Q4Q"). */
  setClassGroup(semester: string, group: ClassGroup | null) {
    return internalWorkspaceStore.mutate({
      kind: "setClassGroup",
      semester,
      group,
    });
  },
  /** The timetable was compared with LMS; its cache TTL restarts now. */
  markTimetableReviewed() {
    return internalWorkspaceStore.mutate({ kind: "markTimetableReviewed" });
  },
  exportData() {
    return repository.export();
  },
  async importData(document: string) {
    if (!repository.import)
      throw new Error("This repository cannot import an export document.");
    await repository.import(document);
    await internalWorkspaceStore.reload();
  },
};
