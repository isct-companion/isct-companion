import { getDesktopBridge } from "../platform/desktop-bridge";
import type { SchoolRecords, SchoolIngestOutcome } from "../platform/desktop-bridge";
import type { Workspace } from "../core/model";
import { seedWorkspace } from "../core/model";
import { repository, type WorkspaceRepository } from "../storage/repository";
import { requireAction } from "../platform/actions";
import { detectPlatform } from "../platform/capabilities";
import { createStore } from "zustand/vanilla";
import { publishSchoolImport } from "../connectors/school";
import type { SchoolImportResult } from "../platform/capabilities";

export type WorkspaceSnapshot = {
  readonly workspace?: Workspace;
  readonly schoolRecords?: SchoolRecords;
  readonly status: "idle" | "loading" | "ready" | "error";
  readonly revision: number;
  readonly persistedRevision: number;
  readonly error?: string;
};
export type WorkspaceEdit = (draft: Workspace) => Workspace | void;

function freeze<T>(value: T): T {
  if (value && typeof value === "object" && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const child of Object.values(value)) freeze(child);
  }
  return value;
}
function validate(value: Workspace) {
  if (
    value.schemaVersion !== 3 ||
    !value.deviceId ||
    !value.settings ||
    !value.academicProfile ||
    !value.importStatus ||
    !value.syncStatus
  )
    throw new Error("Invalid workspace. The previous workspace was preserved.");
  for (const key of [
    "courses",
    "memberships",
    "assignments",
    "materials",
    "events",
    "planItems",
    "exceptions",
    "offerings",
    "intensiveSchedules",
    "plans",
    "academicCalendar",
    "gpaSummary",
    "departments",
    "recommendedCourses",
    "importantDates",
    "exams",
    "scheduleOverrides",
    "subscriptions",
    "scheduledTasks",
  ] as const) {
    if (!Array.isArray(value[key]))
      throw new Error(`Invalid workspace ${key}.`);
    const ids = new Set<string>();
    for (const item of value[key]) {
      if (!item || typeof item.id !== "string" || ids.has(item.id))
        throw new Error(`Invalid or duplicate ${key} record.`);
      ids.add(item.id);
    }
  }
  // Persistence is JSON; reject cycles, non-serializable payloads, and invalid numbers before publishing.
  JSON.stringify(value, (_key, entry) => {
    if (
      typeof entry === "function" ||
      typeof entry === "symbol" ||
      typeof entry === "bigint" ||
      (typeof entry === "number" && !Number.isFinite(entry))
    )
      throw new Error("Workspace contains non-serializable data.");
    return entry;
  });
}

export function createWorkspaceStore(
  storage: Pick<WorkspaceRepository, "load" | "save">,
  school?: { read(): Promise<SchoolRecords>; pull?(site: "science-lms" | "kyomu"): Promise<SchoolIngestOutcome[]> },
) {
  const state = createStore<WorkspaceSnapshot>(() =>
    Object.freeze({
      status: "idle",
      revision: 0,
      persistedRevision: 0,
    }),
  );
  let loading: Promise<void> | undefined;
  let writes = Promise.resolve();
  let refreshing: Promise<SchoolRecords | undefined> | undefined;
  let refreshRequest = 0;
  let pulls: Promise<unknown> = Promise.resolve();
  const publish = (next: WorkspaceSnapshot) => {
    state.setState(Object.freeze(next), true);
  };
  const persist = (workspace: Workspace, revision: number) => {
    writes = writes.then(async () => {
      try {
        await storage.save(structuredClone(workspace));
        publish({
          ...state.getState(),
          persistedRevision: revision,
          error: undefined,
        });
      } catch (reason) {
        const cause = reason instanceof Error ? reason.message : String(reason);
        publish({
          ...state.getState(),
          error:
            `Workspace could not be saved. Your changes remain in memory; retry saving. ${cause}`,
        });
      }
    });
  };
  const update = (edit: WorkspaceEdit) => {
    const current = state.getState();
    if (!current.workspace) throw new Error("Workspace is not ready.");
    const draft = structuredClone(current.workspace);
    const next = edit(draft) ?? draft;
    validate(next);
    if (JSON.stringify(next) === JSON.stringify(current.workspace)) return;
    // Copy again so a caller retaining its draft cannot mutate the published snapshot.
    const workspace = freeze(structuredClone(next));
    const revision = current.revision + 1;
    publish({ ...current, workspace, revision });
    persist(workspace, revision);
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
        const [data, records] = await Promise.all([storage.load(), school?.read()]);
        validate(data);
        // An edit or another import arrived while IPC was pending. Read the
        // persisted latest state rather than replacing a newer in-memory draft.
        if (state.getState().revision !== before.revision || request !== refreshRequest) continue;
        const revision = before.revision + 1;
        publish({ workspace: freeze(structuredClone(data)), schoolRecords: records ? freeze(structuredClone(records)) : before.schoolRecords,
          status: "ready", revision, persistedRevision: revision, error: undefined });
        return records;
      }
    })().finally(() => { refreshing = undefined; });
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
      loading = Promise.all([storage.load(), school?.read()])
        .then(([data, records]) => {
          validate(data);
          publish({
            workspace: freeze(structuredClone(data)),
            schoolRecords: records ? freeze(structuredClone(records)) : undefined,
            status: "ready",
            revision: 0,
            persistedRevision: 0,
          });
        })
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
    pullSchool(site: "science-lms" | "kyomu") {
      if (!school?.pull) return Promise.reject(new Error("School pull needs the desktop app."));
      const operation = pulls.catch(() => undefined).then(async () => {
        await writes;
        if (state.getState().error) throw new Error(state.getState().error);
        const outcomes = await school.pull!(site);
        await reload();
        return outcomes;
      });
      pulls = operation;
      return operation;
    },
    // Application services use this transaction after their own command capability checks.
    update,
    reset() {
      update(() => seedWorkspace());
    },
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
    retrySave() {
      const current = state.getState();
      if (current.workspace) persist(current.workspace, current.revision);
      return writes;
    },
  };
}

const desktopBridge = typeof window === "undefined" ? undefined : getDesktopBridge();
const internalWorkspaceStore = createWorkspaceStore(repository, desktopBridge?.schoolRecords ? {
  read: () => desktopBridge!.schoolRecords!(),
  pull: desktopBridge.pullSchool ? site => desktopBridge!.pullSchool!(site) : undefined,
} : undefined);

/** Read/lifecycle surface. Mutations are available only through workspaceActions. */
export const workspaceStore = {
  getState: internalWorkspaceStore.getState,
  getInitialState: internalWorkspaceStore.getInitialState,
  subscribe: internalWorkspaceStore.subscribe,
  initialize: internalWorkspaceStore.initialize,
  reload: internalWorkspaceStore.reload,
  flush: internalWorkspaceStore.flush,
  retrySave: internalWorkspaceStore.retrySave,
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
      editAcademic(edit: WorkspaceEdit) {
        requireAction("edit-academic-data", platform());
        store.update((draft) => {
          const envelope = (value: Workspace) => {
            const {
              courses,
              memberships,
              assignments,
              materials,
              events,
              planItems,
              selectedTerm,
              terms,
              academicProfile,
              exceptions,
              offerings,
              intensiveSchedules,
              plans,
              academicCalendar,
              gpaSummary,
              departments,
              recommendedCourses,
              importantDates,
              exams,
              scheduleOverrides,
              subscriptions,
              scheduledTasks,
              settings: _settings,
              ...rest
            } = value;
            return JSON.stringify(rest);
          };
          const originalEnvelope = envelope(draft);
          const settings = JSON.stringify(draft.settings);
          const profile = JSON.stringify(draft.academicProfile);
          const exceptions = JSON.stringify(draft.exceptions);
          const next = edit(draft) ?? draft;
          if (envelope(next) !== originalEnvelope)
            throw new Error(
              "Academic actions cannot modify workspace identity, settings or connection status.",
            );
          if (JSON.stringify(next.settings) !== settings)
            throw new Error(
              "Use a settings action to change preferences or native configuration.",
            );
          if (
            JSON.stringify(next.academicProfile) !== profile ||
            JSON.stringify(next.exceptions) !== exceptions
          )
            requireAction("edit-calendar-advanced", platform());
          return next;
        });
      },
      updatePreferences(
        preferences: Partial<
          Pick<Workspace["settings"], "appearance" | "locale" | "textSize">
        >,
      ) {
        requireAction("change-local-preference", platform());
        const choices = {
          appearance: ["system", "light", "dark"],
          locale: ["en", "ja", "zh"],
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
        store.update((draft) => {
          for (const key of ["appearance", "locale", "textSize"] as const) {
            if (preferences[key] !== undefined)
              Object.assign(draft.settings, { [key]: preferences[key] });
          }
        });
      },
    },
  };
}
export const workspaceHooks = createWorkspaceHooks(internalWorkspaceStore);

const protectedWorkspaceEnvelope = (workspace: Workspace) => {
  const {
    settings: _settings,
    syncStatus: _syncStatus,
    scheduledTasks: _scheduledTasks,
    importantDates: _importantDates,
    exams: _exams,
    ...protectedData
  } = workspace;
  return JSON.stringify(protectedData);
};

export const workspaceActions = {
  /** Refresh canonical workspace and parsed school records in one publication. */
  refreshSchool() { return internalWorkspaceStore.reload(); },
  pullSchool(site: "science-lms" | "kyomu") {
    requireAction("import-school", detectPlatform());
    return internalWorkspaceStore.pullSchool(site);
  },
  editAcademic: workspaceHooks.actions.editAcademic,
  editCalendar(edit: WorkspaceEdit) {
    requireAction("edit-calendar-advanced", detectPlatform());
    internalWorkspaceStore.update(edit);
  },
  updateSettings(edit: WorkspaceEdit) {
    requireAction("change-local-preference", detectPlatform());
    internalWorkspaceStore.update((draft) => {
      const protectedBefore = protectedWorkspaceEnvelope(draft);
      const next = edit(draft) ?? draft;
      if (protectedWorkspaceEnvelope(next) !== protectedBefore)
        throw new Error("Settings actions cannot modify core workspace data.");
      return next;
    });
  },
  /** The timetable was compared with LMS; its cache TTL restarts now. */
  markTimetableReviewed(at = new Date().toISOString()) {
    internalWorkspaceStore.update((draft) => {
      draft.importStatus = { ...draft.importStatus, timetableReviewedAt: at };
      return draft;
    });
  },
  recordScheduledTaskRun(
    taskId: string,
    occurrenceAt: string,
    runAt: string,
    result: string,
  ) {
    requireAction("change-local-preference", detectPlatform());
    internalWorkspaceStore.update((draft) => {
      const task = draft.scheduledTasks.find(
        (candidate) => candidate.id === taskId && !candidate.deletedAt,
      );
      if (!task) throw new Error("Scheduled task no longer exists.");
      task.lastOccurrenceAt = occurrenceAt;
      task.lastRunAt = runAt;
      task.lastResult = result;
      task.rev += 1;
      task.updatedAt = runAt;
      task.updatedBy = draft.deviceId;
    });
    return internalWorkspaceStore.flush();
  },
  applySchoolImport(result: SchoolImportResult) {
    requireAction("import-school", detectPlatform());
    internalWorkspaceStore.update((draft) =>
      publishSchoolImport(draft, result),
    );
  },
  replaceWorkspace(workspace: Workspace) {
    requireAction("resolve-cloud-manually", detectPlatform());
    internalWorkspaceStore.update(() => workspace);
  },
  async reset() {
    const workspace = await repository.reset();
    internalWorkspaceStore.update(() => workspace);
    await internalWorkspaceStore.flush();
  },
  exportData() {
    return repository.export();
  },
  async importData(document: string) {
    if (!repository.import)
      throw new Error("This repository cannot import an export document.");
    const workspace = await repository.import(document);
    internalWorkspaceStore.update(() => workspace);
    await internalWorkspaceStore.flush();
  },
};
