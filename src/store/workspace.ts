import { getDesktopBridge } from "../platform/desktop-bridge";
import type {
  SchoolIngestOutcome,
  SchoolPullOptions,
  SchoolRecords,
} from "../platform/desktop-bridge";
import type { Workspace } from "../core/model";
import { repository } from "../storage/repository";
import type {
  CoreWorkspaceSnapshot,
  WorkspaceIntent,
  WorkspaceMutation,
} from "../core/workspace-intents";
import { requireAction } from "../platform/actions";
import { detectPlatform } from "../platform/capabilities";
import { createStore } from "zustand/vanilla";

export type WorkspaceSnapshot = {
  readonly workspace?: Workspace;
  readonly schoolRecords?: SchoolRecords;
  readonly status: "idle" | "loading" | "ready" | "error";
  readonly revision: string;
  readonly error?: string;
};
export type WorkspaceEdit = (draft: Workspace) => Workspace | void;

type WorkspaceService = {
  snapshot(): Promise<CoreWorkspaceSnapshot>;
  mutate(mutation: WorkspaceMutation): Promise<CoreWorkspaceSnapshot>;
};

export function createWorkspaceStore(
  service: WorkspaceService,
  school?: {
    pull?(
      site: "science-lms" | "kyomu",
      options?: SchoolPullOptions,
    ): Promise<SchoolIngestOutcome[]>;
  },
) {
  const state = createStore<WorkspaceSnapshot>(() =>
    Object.freeze({
      status: "idle",
      revision: "",
    }),
  );
  let loading: Promise<void> | undefined;
  let writes: Promise<unknown> = Promise.resolve();
  let refreshing: Promise<SchoolRecords | undefined> | undefined;
  let refreshRequest = 0;
  const publish = (next: WorkspaceSnapshot) => {
    state.setState(Object.freeze(next), true);
  };
  const adopt = (snapshot: CoreWorkspaceSnapshot) => {
    publish({
      workspace: snapshot.workspace,
      schoolRecords: snapshot.schoolRecords,
      status: "ready",
      revision: snapshot.revision,
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
          state.getState().revision !== before.revision ||
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
    pullSchool(site: "science-lms" | "kyomu", options?: SchoolPullOptions) {
      if (!school?.pull)
        return Promise.reject(new Error("School pull needs the desktop app."));
      // Native PullCoordinator owns admission and request coalescing.
      return (async () => {
        await writes;
        if (state.getState().error) throw new Error(state.getState().error);
        const outcomes = options
          ? await school.pull!(site, options)
          : await school.pull!(site);
        await reload();
        return outcomes;
      })();
    },
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
  lastRuns: {},
});

/** Browser preview has no Core process and exposes a read-only sample. */
export function createBrowserPreviewService(
  previewRepository: typeof repository,
): WorkspaceService {
  let workspace: Workspace | undefined;
  const snapshot = (): CoreWorkspaceSnapshot => ({
    revision: `preview:${JSON.stringify(workspace).length}`,
    workspace: structuredClone(workspace!),
    schoolRecords: emptySchoolRecords(),
  });
  return {
    async snapshot() {
      workspace ??= await previewRepository.load();
      return snapshot();
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
        mutate: (mutation) => desktopBridge.mutateWorkspace(mutation),
      }
    : browserPreviewService,
  desktopBridge?.pullSchool
    ? { pull: (site, options) => desktopBridge.pullSchool!(site, options) }
    : undefined,
);

/** Read/lifecycle surface. Mutations are available only through workspaceActions. */
export const workspaceStore = {
  getState: internalWorkspaceStore.getState,
  getInitialState: internalWorkspaceStore.getInitialState,
  subscribe: internalWorkspaceStore.subscribe,
  initialize: internalWorkspaceStore.initialize,
  reload: internalWorkspaceStore.reload,
  flush: internalWorkspaceStore.flush,
};

/** Guarded domain actions shared by React and non-React callers. */
export function createWorkspaceHooks(
  store: ReturnType<typeof createWorkspaceStore>,
  platform = detectPlatform,
) {
  return {
    getState: store.getState,
    subscribe: store.subscribe,
    actions: {
      updatePreferences(
        preferences: Partial<
          Pick<
            Workspace["settings"],
            "appearance" | "locale" | "contentLanguage" | "textSize"
          >
        >,
      ) {
        requireAction("change-local-preference", platform());
        const choices = {
          appearance: ["system", "light", "dark"],
          locale: ["en", "ja", "zh"],
          contentLanguage: ["auto", "ja", "en"],
          textSize: ["small", "medium", "large"],
        };
        if (
          !preferences ||
          typeof preferences !== "object" ||
          Array.isArray(preferences)
        )
          throw new Error("Invalid preferences.");
        for (const [key, value] of Object.entries(preferences)) {
          if (
            !(key in choices) ||
            typeof value !== "string" ||
            !choices[key as keyof typeof choices].includes(value)
          )
            throw new Error("Invalid preference value.");
        }
        return store.mutate({ kind: "patchSettings", patch: preferences });
      },
    },
  };
}
export const workspaceHooks = createWorkspaceHooks(internalWorkspaceStore);

export const workspaceActions = {
  /** Refresh canonical workspace and parsed school records in one publication. */
  refreshSchool() {
    return internalWorkspaceStore.reload();
  },
  pullSchool(site: "science-lms" | "kyomu", options?: SchoolPullOptions) {
    requireAction("import-school", detectPlatform());
    return internalWorkspaceStore.pullSchool(site, options);
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
  /** The study-guide cohort is a basic choice on every platform; the rest of
   * the academic profile stays behind the advanced calendar editor. */
  setAdmissionYear(year: number | undefined) {
    requireAction("change-local-preference", detectPlatform());
    if (year !== undefined && (!Number.isInteger(year) || year < 2000))
      throw new Error("Invalid admission year.");
    return internalWorkspaceStore.mutate({
      kind: "setAdmissionYear",
      admissionYear: year ?? null,
    });
  },
  updateSettings(edit: WorkspaceEdit) {
    requireAction("change-local-preference", detectPlatform());
    // Resolve form edits inside the transport queue, after adopting the preceding
    // Core result. Two immediate preference edits must not copy the same old snapshot.
    return internalWorkspaceStore.mutate((draft) => ({
      kind: "patchSettings",
      patch: (edit(draft) ?? draft).settings,
    }));
  },
  /** The timetable was compared with LMS; its cache TTL restarts now. */
  markTimetableReviewed() {
    return internalWorkspaceStore.mutate({ kind: "markTimetableReviewed" });
  },
  async reset() {
    await repository.reset();
    await internalWorkspaceStore.reload();
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
