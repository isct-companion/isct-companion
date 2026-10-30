import { t } from "../i18n";
import { getDesktopBridge } from "../platform/desktop-bridge";
import type { WorkspaceChangedEvent } from "../platform/desktop-bridge";
import type { Workspace } from "../lib/model";
import type { ClassGroup } from "../generated/persistence";
import { repository } from "../storage/repository";
import type {
  CoreWorkspaceCommit,
  WorkspaceIntent,
  WorkspaceMutation,
} from "../lib/workspace-intents";
import type { WorkspaceChange } from "../generated/persistence";
import { requireAction } from "../platform/actions";
import { detectPlatform } from "../platform/capabilities";
import { createStore } from "zustand/vanilla";
import { changesCommitted, everythingChanged } from "../platform/query-client";
import { fetchRecords, type RecordField } from "../platform/records";

/**
 * The shell's side of writing: where the next intent starts from and how
 * the last one went. Records themselves live only in the query cache
 * (`platform/records`); nothing here copies them.
 */
export type WorkspaceState = {
  readonly status: "idle" | "loading" | "ready" | "error";
  /** Baseline for user intents; school and reference ingests keep it. */
  readonly revision: string;
  /** Counts changes heard per name; a whole re-read counts as `all`. The
   * finder refreshes its open page by the names its page read. */
  readonly viewRevisions: ViewRevisions;
  readonly error?: string;
};
export type ViewRevisions = Readonly<
  Partial<Record<WorkspaceChange | "all", number>>
>;
export type WorkspaceEdit = (draft: Workspace) => Workspace | void;

type WorkspaceService = {
  /** The revision the next intent is made against. */
  revision(): Promise<string>;
  mutate(mutation: WorkspaceMutation): Promise<CoreWorkspaceCommit>;
};

/** What an intent built from stored records starts from: `fields` read
 * fresh from Core inside the write queue, so two quick edits never copy
 * the same old records. */
type DraftIntent = {
  fields: readonly RecordField[];
  build: (draft: Workspace) => WorkspaceIntent;
};

export function createWorkspaceStore(service: WorkspaceService) {
  const state = createStore<WorkspaceState>(() =>
    Object.freeze({
      status: "idle",
      revision: "",
      viewRevisions: {},
    }),
  );
  let loading: Promise<void> | undefined;
  let writes: Promise<unknown> = Promise.resolve();
  const localMutationRevisions = new Set<string>();
  /** A background writer (cloud sync, the scheduler, an agent) may have
   * moved the revision; the next intent reads it first. */
  let revisionStale = false;
  const publish = (next: WorkspaceState) => {
    state.setState(Object.freeze(next), true);
  };
  const counted = (changes: readonly (WorkspaceChange | "all")[]) => {
    const viewRevisions: Partial<Record<WorkspaceChange | "all", number>> = {
      ...state.getState().viewRevisions,
    };
    for (const change of changes)
      viewRevisions[change] = (viewRevisions[change] ?? 0) + 1;
    return viewRevisions;
  };
  /** Reads the revision again and refetches every answer: anything may have
   * changed (startup, an intent that lost a race, an import). */
  const reload = () => {
    // A revision read participates in the same queue as writes: waiting for
    // preceding writes alone lets a following commit overtake the read.
    const operation = writes.then(async () => {
      const revision = await service.revision();
      publish({
        ...state.getState(),
        status: "ready",
        revision,
        viewRevisions: counted(["all"]),
        error: undefined,
      });
      everythingChanged();
    });
    writes = operation.catch(() => undefined);
    return operation;
  };
  const mutate = (intent: WorkspaceIntent | DraftIntent) => {
    const operation = writes.then(async () => {
      const current = state.getState();
      if (current.status !== "ready" || !current.revision)
        throw new Error(t("workspace.workspaceIsNotReady"));
      try {
        if (revisionStale) {
          revisionStale = false;
          publish({ ...state.getState(), revision: await service.revision() });
        }
        const resolved =
          "build" in intent
            ? intent.build(
                structuredClone(await fetchRecords(intent.fields)) as Workspace,
              )
            : intent;
        const commit = await service.mutate({
          expectedRevision: state.getState().revision,
          intent: resolved,
        });
        publish({
          ...state.getState(),
          revision: commit.revision,
          viewRevisions: counted(commit.changes),
        });
        changesCommitted(commit.changes, commit.seq);
        localMutationRevisions.add(commit.revision);
        if (localMutationRevisions.size > 32)
          localMutationRevisions.delete(
            localMutationRevisions.values().next().value!,
          );
        return commit;
      } catch (error) {
        // The intent may have lost a race with school, cloud, or another
        // native writer. Read where the next one starts and what is stored
        // now for the person to review; never replay the failed intent.
        try {
          publish({ ...state.getState(), revision: await service.revision() });
          everythingChanged();
        } catch {
          // Keep the original mutation error.
        }
        throw error;
      }
    });
    writes = operation.catch(() => undefined);
    return operation;
  };
  const handleWorkspaceChanged = async (event: WorkspaceChangedEvent) => {
    changesCommitted(event.changes, event.seq);
    publish({ ...state.getState(), viewRevisions: counted(event.changes) });
    if (event.source !== "user-intent") {
      revisionStale = true;
      return;
    }
    // Core emits before the native command resolves. Wait for the command's
    // own commit, then re-read the revision if another client wrote.
    await writes;
    if (!event.revision) {
      await reload();
      return;
    }
    if (
      state.getState().revision === event.revision ||
      localMutationRevisions.has(event.revision)
    )
      return;
    // Delivery order is not commit order. The event's revision may already
    // have been superseded, so only a queued read can seed the next edit.
    await reload();
  };
  return {
    getState: state.getState,
    getInitialState: state.getInitialState,
    subscribe: state.subscribe,
    initialize() {
      if (state.getState().status === "ready") return Promise.resolve();
      if (loading) return loading;
      publish({ ...state.getState(), status: "loading", error: undefined });
      loading = service
        .revision()
        .then((revision) => {
          publish({
            ...state.getState(),
            status: "ready",
            revision,
            error: undefined,
          });
        })
        .catch(() => {
          publish({
            ...state.getState(),
            status: "error",
            error: t("workspace.workspaceCouldNotBeOpened"),
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
    async flush() {
      await writes;
      const error = state.getState().error;
      if (error) throw new Error(error);
    },
  };
}

const desktopBridge =
  typeof window === "undefined" ? undefined : getDesktopBridge();

/** Browser preview has no Core process: nothing to write against. */
export function createBrowserPreviewService(): WorkspaceService {
  return {
    async revision() {
      return "preview";
    },
    async mutate() {
      throw new Error(t("workspace.workspaceEditsNeedTheNative"));
    },
  };
}
const internalWorkspaceStore = createWorkspaceStore(
  desktopBridge
    ? {
        revision: () => desktopBridge.workspaceRevision(),
        mutate: (mutation) => desktopBridge.mutateWorkspace(mutation),
      }
    : createBrowserPreviewService(),
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
  /** Read everything again: the revision and every answer in use. */
  refreshSchool() {
    return internalWorkspaceStore.reload();
  },
  mutate(intent: WorkspaceIntent) {
    return internalWorkspaceStore.mutate(intent);
  },
  editCalendar(edit: WorkspaceEdit) {
    requireAction("edit-calendar-advanced", detectPlatform());
    return internalWorkspaceStore.mutate({
      fields: ["academicProfile", "exceptions"],
      build: (draft) => {
        const next = edit(draft) ?? draft;
        return {
          kind: "editAcademicCalendar",
          academicProfile: next.academicProfile,
          exceptions: next.exceptions,
        };
      },
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
    return internalWorkspaceStore.mutate({
      fields: ["settings"],
      build: (draft) => ({
        kind: "replaceSettings",
        settings: (edit(draft) ?? draft).settings,
      }),
    });
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
      throw new Error(t("workspace.thisRepositoryCannotImportAn"));
    await repository.import(document);
    await internalWorkspaceStore.reload();
  },
};
