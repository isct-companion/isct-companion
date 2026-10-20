// @vitest-environment jsdom
import { createMemoryRouter } from "react-router";
import { afterEach, describe, expect, it } from "vitest";

import {
  parseRoute,
  paths,
  routeRedirect,
  workspaceRoutes,
  type DrawerTab,
} from "@app/app/routes";
import { seedWorkspace, type Workspace } from "@app/lib/model";
import type {
  CoreWorkspacePatch,
  CoreWorkspaceSnapshot,
  WorkspaceIntent,
  WorkspaceMutation,
} from "@app/lib/workspace-intents";
import type { WorkspaceChange } from "@app/generated/persistence";
import type { SectionContext } from "@app/pages/settings/registry";
import {
  can,
  performAction,
  requireAction,
  type AppAction,
} from "@app/platform/actions";
import {
  detectPlatform,
  resolveCapabilities,
  type CapabilityRuntime,
} from "@app/platform/capabilities";
import {
  scheduleQueries,
  scheduleQuery,
  type ScheduleQuery,
} from "@app/platform/core-query";
import {
  createTauriDesktopBridge,
  type InvokeNative,
  type ListenNative,
  type SchoolDownloadEvent,
  type SchoolRecords,
  type SchoolSessionEvent,
  type WorkspaceChangedEvent,
} from "@app/platform/desktop-bridge";
import { finderQuery } from "@app/platform/finder-query";
import {
  pageProgress,
  progressLine,
  retryScope,
  runsFor,
  type RunView,
  type SyncSnapshot,
} from "@app/platform/sync";
import { LocalRepository, SqliteRepository } from "@app/storage/repository";
import {
  createBrowserPreviewService,
  createWorkspaceStore,
  workspaceActions,
  workspaceStore,
  type WorkspaceSnapshot,
} from "@app/store/workspace";
import {
  DEVICE,
  emptySchoolRecords,
  installFakeCore,
  workspaceFixture,
  type FakeCore,
  type FakeCoreOptions,
} from "@tests/harness";

/**
 * The workspace store, the read-model query caches it keys on, the repository
 * adapters, the route table and the platform contracts, grouped by behaviour.
 * The suites it replaces were one file per source module; the guarantees below
 * are the ones those files encoded.
 *
 * Nothing here mounts React: the store takes its transport as an argument and
 * `createTauriDesktopBridge` takes `invoke`/`listen`, so the real modules are
 * driven through their own seams. `installFakeCore` gives the real Tauri
 * `invoke` a scripted host for the repository and query-batch cases.
 */

// `getDesktopBridge` caches the bridge it builds on first use, so the
// browser-preview refusal has to run before any case installs a host.
describe("queries without a native host", () => {
  it("refuses schedule and finder reads instead of inventing data", async () => {
    await expect(
      scheduleQuery({ kind: "declaredCredits", term: "2026-3Q" }),
    ).rejects.toThrow("native Core");
    await expect(finderQuery({ year: 2026, term: "2026-3Q" })).rejects.toThrow(
      "native Core",
    );
  });
});

const hosts: FakeCore[] = [];

/** Installs the scripted Tauri host and restores the window afterwards. */
function installHost(options: FakeCoreOptions): FakeCore {
  const core = installFakeCore(options);
  hosts.push(core);
  return core;
}

afterEach(() => {
  for (const core of hosts.splice(0)) core.restore();
});

/** Lets the store's microtask chain reach a scripted host. */
const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

const settingsWith = (changes: Partial<Workspace["settings"]>): Workspace => {
  const workspace = workspaceFixture();
  workspace.settings = { ...workspace.settings, ...changes };
  return workspace;
};

/** A settings-form intent changing one preference of the seed settings. */
const appearance = (
  value: Workspace["settings"]["appearance"],
): WorkspaceIntent => ({
  kind: "replaceSettings",
  settings: { ...seedWorkspace().settings, appearance: value },
});

const bumpRevision = (current: string) => {
  const match = /^r(\d+)$/.exec(current);
  return match ? `r${Number(match[1]) + 1}` : `${current}+1`;
};

type SnapshotAnswer =
  | CoreWorkspaceSnapshot
  | Promise<CoreWorkspaceSnapshot>
  | (() => Promise<CoreWorkspaceSnapshot>);

type MutationScript = (
  request: WorkspaceMutation,
  host: CoreHost,
) => Promise<CoreWorkspaceSnapshot>;

type CoreHost = {
  service: {
    snapshot(): Promise<CoreWorkspaceSnapshot>;
    records(changes: readonly WorkspaceChange[]): Promise<CoreWorkspacePatch>;
    mutate(request: WorkspaceMutation): Promise<CoreWorkspaceSnapshot>;
  };
  /** The revision each read started from, in order. */
  reads: string[];
  /** The change names of each record read, in order. */
  recordReads: WorkspaceChange[][];
  answerRecords(
    answer: CoreWorkspacePatch | (() => Promise<CoreWorkspacePatch>),
  ): void;
  /** Every mutation the store submitted, in order. */
  mutations: WorkspaceMutation[];
  publish(workspace: Workspace, revision?: string): void;
  answerRead(answer: SnapshotAnswer): void;
  answerMutation(answer: SnapshotAnswer | MutationScript | Error): void;
  snapshotAt(revision: string, workspace?: Workspace): CoreWorkspaceSnapshot;
};

async function resolveSnapshot(
  answer: SnapshotAnswer | undefined,
  fallback: () => CoreWorkspaceSnapshot,
): Promise<CoreWorkspaceSnapshot> {
  if (answer === undefined) return fallback();
  return typeof answer === "function" ? answer() : answer;
}

/**
 * A recording Core for the store suite. `installFakeCore` answers read models
 * by kind; the store asks for one snapshot and one mutation, so this host
 * scripts those two transport calls and records them like a command log.
 */
function coreHost(initial: Workspace = workspaceFixture()): CoreHost {
  let revision = "r0";
  let workspace = structuredClone(initial);
  const reads: string[] = [];
  const recordReads: WorkspaceChange[][] = [];
  const recordAnswers: (
    CoreWorkspacePatch | (() => Promise<CoreWorkspacePatch>)
  )[] = [];
  const mutations: WorkspaceMutation[] = [];
  const readAnswers: SnapshotAnswer[] = [];
  const mutationAnswers: (SnapshotAnswer | MutationScript | Error)[] = [];

  const snapshotAt = (
    nextRevision: string,
    next: Workspace = workspace,
  ): CoreWorkspaceSnapshot => ({
    revision: nextRevision,
    contentRevision: nextRevision,
    workspace: structuredClone(next),
    schoolRecords: emptySchoolRecords(),
  });

  const host: CoreHost = {
    service: {
      async snapshot() {
        reads.push(revision);
        const answer = readAnswers.shift();
        return resolveSnapshot(answer, () => snapshotAt(revision));
      },
      async records(changes) {
        recordReads.push([...changes]);
        const answer = recordAnswers.shift();
        if (typeof answer === "function") return answer();
        // Unscripted, the host answers every field: a superset is a valid patch.
        return answer ?? snapshotAt(revision);
      },
      async mutate(request) {
        mutations.push(request);
        const answer = mutationAnswers.shift();
        if (answer instanceof Error) throw answer;
        if (typeof answer === "function") return answer(request, host);
        if (answer !== undefined) return answer;
        // The default path commits the one intent this suite submits.
        if (request.intent.kind === "replaceSettings") {
          workspace = {
            ...workspace,
            settings: structuredClone(request.intent.settings),
          };
        }
        revision = bumpRevision(revision);
        return snapshotAt(revision);
      },
    },
    reads,
    recordReads,
    answerRecords(answer) {
      recordAnswers.push(answer);
    },
    mutations,
    publish(next, nextRevision) {
      workspace = structuredClone(next);
      if (nextRevision !== undefined) revision = nextRevision;
    },
    answerRead(answer) {
      readAnswers.push(answer);
    },
    answerMutation(answer) {
      mutationAnswers.push(answer);
    },
    snapshotAt,
  };
  return host;
}

describe("opening the workspace", () => {
  it("reads the snapshot once when several consumers initialize together", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);

    const first = store.initialize();
    const second = store.initialize();
    expect(store.getState().status).toBe("loading");
    await Promise.all([first, second]);

    expect(host.reads).toEqual(["r0"]);
    expect(store.getState()).toMatchObject({ status: "ready", revision: "r0" });
  });

  it("reports an open failure and refuses to flush or export unopened data", async () => {
    const host = coreHost();
    host.answerRead(() => Promise.reject(new Error("database is locked")));
    const store = createWorkspaceStore(host.service);

    await store.initialize();

    expect(store.getState().status).toBe("error");
    expect(store.getState().error).toContain("Workspace could not be opened");
    await expect(store.flush()).rejects.toThrow(
      "Workspace could not be opened",
    );
    expect(() => store.export()).toThrow("Workspace is not ready.");
  });

  it("publishes one shared snapshot to every subscriber", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);
    expect(store.getInitialState()).toMatchObject({
      status: "idle",
      revision: "",
    });

    const seen: WorkspaceSnapshot[] = [];
    const off = store.subscribe((state) => seen.push(state));
    await store.initialize();
    off();

    expect(seen.length).toBeGreaterThan(0);
    expect(seen.at(-1)).toBe(store.getState());
    expect(store.getState().revision).toBe("r0");
  });
});

describe("submitting coarse intents", () => {
  it("sends an intent against the revision it has adopted and adopts the answer", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);
    await store.initialize();

    await store.mutate(appearance("dark"));

    expect(host.mutations).toEqual([
      { expectedRevision: "r0", intent: appearance("dark") },
    ]);
    expect(store.getState()).toMatchObject({ revision: "r1" });
    expect(store.getState().workspace?.settings.appearance).toBe("dark");
  });

  it("serializes rapid intents against the revision each preceding write committed", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);
    await store.initialize();

    const first = store.mutate(appearance("dark"));
    const second = store.mutate(appearance("light"));
    await Promise.all([first, second]);

    expect(host.mutations.map((mutation) => mutation.expectedRevision)).toEqual(
      ["r0", "r1"],
    );
    expect(store.getState().workspace?.settings.appearance).toBe("light");
  });

  it("resolves a form intent against the snapshot the preceding write committed", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);
    await store.initialize();

    const first = store.mutate((draft): WorkspaceIntent => {
      draft.settings.appearance = "dark";
      return { kind: "replaceSettings", settings: draft.settings };
    });
    const second = store.mutate((draft): WorkspaceIntent => {
      draft.settings.locale = "ja";
      return { kind: "replaceSettings", settings: draft.settings };
    });
    await Promise.all([first, second]);

    expect(host.mutations[1]).toMatchObject({
      expectedRevision: "r1",
      intent: { settings: { appearance: "dark", locale: "ja" } },
    });
    expect(store.getState().workspace?.settings).toMatchObject({
      appearance: "dark",
      locale: "ja",
    });
  });

  it("refuses to submit an intent before the workspace is open", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);

    await expect(store.mutate(appearance("dark"))).rejects.toThrow(
      "Workspace is not ready.",
    );

    expect(host.mutations).toEqual([]);
    expect(host.reads).toEqual([]);
  });
});

describe("recovering from a rejected or external write", () => {
  it("never applies a rejected intent and re-reads the authoritative snapshot", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);
    await store.initialize();
    const before = store.getState();

    host.answerMutation(new Error("revision conflict"));
    await expect(store.mutate(appearance("dark"))).rejects.toThrow(
      "revision conflict",
    );

    expect(store.getState().revision).toBe(before.revision);
    expect(store.getState().workspace).toEqual(before.workspace);
    expect(host.reads).toEqual(["r0", "r0"]);
  });

  it("adopts an external write after a conflict and lets the user retry explicitly", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);
    await store.initialize();

    host.answerMutation(async () => {
      host.publish(settingsWith({ locale: "ja" }), "external");
      throw new Error("workspace revision conflict");
    });
    const intent = (workspace: Workspace): WorkspaceIntent => ({
      kind: "replaceSettings",
      settings: { ...workspace.settings, appearance: "dark" },
    });

    await expect(store.mutate(intent)).rejects.toThrow("revision conflict");
    expect(store.getState().revision).toBe("external");
    expect(store.getState().workspace?.settings).toMatchObject({
      locale: "ja",
      appearance: "system",
    });
    expect(host.mutations).toHaveLength(1);

    await store.mutate(intent);

    expect(host.mutations.map((mutation) => mutation.expectedRevision)).toEqual(
      ["r0", "external"],
    );
    expect(store.getState().workspace?.settings).toMatchObject({
      locale: "ja",
      appearance: "dark",
    });
  });

  it("reloads when a user-intent event names another client's revision", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);
    await store.initialize();
    host.publish(settingsWith({ locale: "ja" }), "external-r1");

    await store.handleWorkspaceChanged({
      source: "user-intent",
      revision: "external-r1",
      changes: ["settings"],
    });

    // Another client's intent still reconciles through the whole snapshot.
    expect(host.reads).toEqual(["r0", "external-r1"]);
    expect(host.recordReads).toEqual([]);
    expect(store.getState()).toMatchObject({ revision: "external-r1" });
    expect(store.getState().workspace?.settings.locale).toBe("ja");
  });

  it("shares one recovery read with an event that waits on the failed write", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);
    await store.initialize();

    const failure = deferred<CoreWorkspaceSnapshot>();
    host.answerMutation(() => failure.promise);
    const mutation = store.mutate(appearance("dark"));
    await settle();
    const event = store.handleWorkspaceChanged({
      source: "user-intent",
      revision: "external-r1",
      changes: ["settings"],
    });
    host.publish(settingsWith({ locale: "ja" }), "external-r1");
    failure.reject(new Error("revision conflict"));

    await expect(mutation).rejects.toThrow("revision conflict");
    await event;

    expect(host.reads).toEqual(["r0", "external-r1"]);
    expect(store.getState().revision).toBe("external-r1");
    expect(store.getState().workspace?.settings.locale).toBe("ja");
  });

  it("recognizes its own committed revision and does not re-read", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);
    await store.initialize();
    await store.mutate(appearance("dark"));
    await store.mutate(appearance("light"));

    await store.handleWorkspaceChanged({
      source: "user-intent",
      revision: "r1",
      changes: ["settings"],
    });

    expect(host.reads).toEqual(["r0"]);
    expect(host.recordReads).toEqual([]);
    expect(store.getState().revision).toBe("r2");
  });

  it("waits for an in-flight write before treating an event as its own", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);
    await store.initialize();

    const release = deferred<CoreWorkspaceSnapshot>();
    host.answerMutation(async () => {
      const snapshot = await release.promise;
      host.publish(snapshot.workspace, snapshot.revision);
      return snapshot;
    });

    const mutation = store.mutate(appearance("dark"));
    await settle();
    expect(host.mutations).toHaveLength(1);

    const event = store.handleWorkspaceChanged({
      source: "user-intent",
      revision: "r1",
      changes: ["settings"],
    });
    const newer = store.mutate(appearance("light"));
    release.resolve(
      host.snapshotAt("r1", settingsWith({ appearance: "dark" })),
    );
    await Promise.all([mutation, event, newer]);

    expect(host.reads).toEqual(["r0"]);
    expect(store.getState()).toMatchObject({ revision: "r2" });
    expect(store.getState().workspace?.settings.appearance).toBe("light");
  });

  it("patches only the records a background change names, without a snapshot read", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);
    await store.initialize();
    const before = store.getState();
    const freshness = {
      "lms.course/7": {
        fetchedAt: "2026-10-19T02:46:13Z",
        outcome: "complete",
        message: null,
        failures: 0,
        nextEligible: null,
        lastError: null,
      },
    } as unknown as SchoolRecords["freshness"];
    host.answerRecords({
      revision: "r0",
      contentRevision: "c1",
      workspace: {},
      schoolRecords: { freshness, pageAvailability: {} },
    });

    await store.handleWorkspaceChanged({
      source: "school-pull",
      changes: ["freshness"],
    });

    expect(host.reads).toEqual(["r0"]);
    expect(host.recordReads).toEqual([["freshness"]]);
    const after = store.getState();
    expect(after.schoolRecords?.freshness).toEqual(freshness);
    expect(after.contentRevision).toBe("c1");
    // Untouched records keep their identity, so nothing derived from them
    // recomputes; only views that read `freshness` get a new cache key.
    expect(after.schoolRecords?.lmsCourses).toBe(
      before.schoolRecords?.lmsCourses,
    );
    expect(after.workspace?.courses).toBe(before.workspace?.courses);
    expect(after.viewRevisions).toEqual({
      ...before.viewRevisions,
      freshness: 1,
    });
  });

  it("merges changes heard during a patch into one follow-up read", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);
    await store.initialize();
    const slow = deferred<CoreWorkspacePatch>();
    host.answerRecords(() => slow.promise);

    const first = store.handleWorkspaceChanged({
      source: "school-pull",
      changes: ["assignments"],
    });
    await settle();
    const second = store.handleWorkspaceChanged({
      source: "reference-pull",
      changes: ["exams"],
    });
    const third = store.handleWorkspaceChanged({
      source: "school-pull",
      changes: ["courses", "exams"],
    });
    slow.resolve(host.snapshotAt("r0"));
    await Promise.all([first, second, third]);

    expect(host.recordReads).toEqual([["assignments"], ["exams", "courses"]]);
    expect(host.reads).toEqual(["r0"]);
  });

  it("reads a patch again when a mutation committed while it was in flight", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);
    await store.initialize();
    const stale = deferred<CoreWorkspacePatch>();
    host.answerRecords(() => stale.promise);

    const patch = store.handleWorkspaceChanged({
      source: "school-pull",
      changes: ["grades"],
    });
    await settle();
    await store.mutate(appearance("dark"));
    stale.resolve(
      host.snapshotAt("r0", settingsWith({ appearance: "system" })),
    );
    await patch;

    // The late answer predates the committed intent and is not adopted.
    expect(host.recordReads).toEqual([["grades"], ["grades"]]);
    expect(store.getState().revision).toBe("r1");
    expect(store.getState().workspace?.settings.appearance).toBe("dark");
  });

  it("discards a reload that finished before a mutation committed inside it", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);
    await store.initialize();

    const stale = deferred<CoreWorkspaceSnapshot>();
    host.answerRead(() => stale.promise);
    const reload = store.reload();
    await settle();

    await store.mutate(appearance("dark"));
    stale.resolve(
      host.snapshotAt("r0", settingsWith({ appearance: "system" })),
    );
    await reload;

    expect(host.reads).toEqual(["r0", "r0", "r1"]);
    expect(store.getState().revision).toBe("r1");
    expect(store.getState().workspace?.settings.appearance).toBe("dark");
  });
});

describe("browser preview", () => {
  it("opens a detached read-only sample and refuses every mutation", async () => {
    const stored = workspaceFixture();
    const repository = {
      async load() {
        return structuredClone(stored);
      },
      async export() {
        return "";
      },
      async import(_document: string) {
        return structuredClone(stored);
      },
    };
    const store = createWorkspaceStore(createBrowserPreviewService(repository));
    await store.initialize();

    expect(store.getState().status).toBe("ready");
    await expect(store.mutate(appearance("dark"))).rejects.toThrow(
      "native app",
    );
    await expect(
      store.mutate({ kind: "markTimetableReviewed" }),
    ).rejects.toThrow("native app");
    expect(stored.settings.appearance).toBe("system");
  });

  it("keeps the shipped store on the read-only preview service", async () => {
    await workspaceStore.initialize();

    expect(workspaceStore.getState().status).toBe("ready");
    expect(workspaceStore.getState().revision).toMatch(/^preview:/);
    await expect(
      workspaceActions.mutate({ kind: "markTimetableReviewed" }),
    ).rejects.toThrow("native app");
  });
});

describe("schedule query batches", () => {
  it("sends one batched request and returns its values in query order", async () => {
    const queries: ScheduleQuery[] = [
      { kind: "declaredCredits", term: "2026-3Q" },
      { kind: "quarterRange", term: "2026-3Q" },
    ];
    const core = installHost({
      workspace: workspaceFixture(),
      queries: {
        declaredCredits: 6,
        quarterRange: { from: "2026-09-28", to: "2026-12-02" },
      },
    });

    const values =
      await scheduleQueries<[number, { from: string; to: string }]>(queries);

    expect(values).toEqual([6, { from: "2026-09-28", to: "2026-12-02" }]);
    expect(core.commands("schedule_query")).toEqual([
      { command: "schedule_query", args: { queries } },
    ]);
    await expect(
      scheduleQuery<number>({ kind: "declaredCredits", term: "2026-3Q" }),
    ).resolves.toBe(6);
    expect(core.commands("schedule_query")).toHaveLength(2);
  });

  it("rejects a snapshot with a different number of values than queries", async () => {
    installHost({
      workspace: workspaceFixture(),
      handle: (command) =>
        command === "schedule_query"
          ? { revision: "r1", values: [] }
          : undefined,
    });

    await expect(
      scheduleQueries([{ kind: "declaredCredits", term: "2026-3Q" }]),
    ).rejects.toThrow("invalid schedule snapshot");
  });

  it("rejects a snapshot whose value kinds are reordered", async () => {
    installHost({
      workspace: workspaceFixture(),
      handle: (command) =>
        command === "schedule_query"
          ? {
              revision: "r1",
              values: [{ kind: "quarterRange", value: { from: "a", to: "b" } }],
            }
          : undefined,
    });

    await expect(
      scheduleQueries([{ kind: "declaredCredits", term: "2026-3Q" }]),
    ).rejects.toThrow("invalid schedule snapshot");
  });

  it("accepts the academic-profile and academic-term query aliases", async () => {
    const profile = { name: "test", gradePath: [], years: [] };

    installHost({
      workspace: workspaceFixture(),
      handle: (command, args) => {
        if (command !== "schedule_query") return undefined;
        const coreKinds: Record<string, string> = {
          validateAcademicProfile: "academicProfile",
          academicTermForDate: "academicTerm",
        };
        return {
          revision: "r1",
          values: (args.queries as { kind: string }[]).map((query) => ({
            kind: coreKinds[query.kind] ?? query.kind,
            value: query.kind === "validateAcademicProfile" ? true : "2026-3Q",
          })),
        };
      },
    });

    await expect(
      scheduleQueries<[boolean, string]>([
        { kind: "validateAcademicProfile", profile },
        { kind: "academicTermForDate", date: "2026-10-01", profile },
      ]),
    ).resolves.toEqual([true, "2026-3Q"]);
  });

  it("retries a projection after the host answered with a failure", async () => {
    let attempts = 0;
    const core = installHost({
      workspace: workspaceFixture(),
      handle: (command) => {
        if (command !== "schedule_query") return undefined;
        attempts += 1;
        if (attempts === 1) throw new Error("temporary failure");
        return {
          revision: "r1",
          values: [{ kind: "declaredCredits", value: 4 }],
        };
      },
    });

    await expect(
      scheduleQueries([{ kind: "declaredCredits", term: "2026-3Q" }]),
    ).rejects.toThrow("temporary failure");
    await expect(
      scheduleQueries([{ kind: "declaredCredits", term: "2026-3Q" }]),
    ).resolves.toEqual([4]);
    expect(core.commands("schedule_query")).toHaveLength(2);
  });
});

describe("local browser repository", () => {
  it("loads a detached read-only sample", async () => {
    const repository = new LocalRepository();

    const first = await repository.load();
    first.settings.locale = "zh";

    expect((await repository.load()).settings.locale).not.toBe("zh");
  });

  it("refuses export and import without the native app", async () => {
    const repository = new LocalRepository();

    await expect(repository.export()).rejects.toThrow("native app");
    await expect(repository.import("{}")).rejects.toThrow("native app");
  });
});

describe("SQLite repository", () => {
  it("forwards each exact command and returns Core's answer untouched", async () => {
    const workspace = workspaceFixture();
    const exported = '{"kind":"IsctCompanion.local-backup"}';
    const core = installHost({
      workspace,
      handle: (command) => {
        if (command === "export_data") return exported;
        if (command === "load_workspace" || command === "import_data")
          return workspace;
        return undefined;
      },
    });
    const repository = new SqliteRepository();

    expect((await repository.load()).deviceId).toBe(DEVICE);
    expect(await repository.export()).toBe(exported);
    expect((await repository.import("unparsed bytes for Core")).deviceId).toBe(
      DEVICE,
    );
    expect(core.calls).toEqual([
      { command: "load_workspace", args: {} },
      { command: "export_data", args: {} },
      {
        command: "import_data",
        args: { document: "unparsed bytes for Core" },
      },
    ]);
  });

  it("surfaces Core's own error without substituting local validation", async () => {
    installHost({
      workspace: workspaceFixture(),
      handle: (command) => {
        if (command === "import_data") throw new Error("core migration failed");
        return undefined;
      },
    });

    await expect(
      new SqliteRepository().import("not json at all"),
    ).rejects.toThrow("core migration failed");
  });
});

describe("shared workspace routes", () => {
  const desktop: SectionContext = {
    platform: "native-desktop",
    developer: true,
    extensionsAvailable: true,
  };

  it("round-trips page, section, course and assignment paths including encoded ids", () => {
    expect(parseRoute(paths.home()).homeView).toBe("day");
    expect(parseRoute(paths.home("week")).homeView).toBe("week");
    expect(parseRoute(paths.web()).page).toBe("web");
    expect(parseRoute(paths.lectures()).courseId).toBeUndefined();
    expect(parseRoute(paths.settings("appearance")).section).toBe("appearance");

    const courseId = "中文 /?#%";
    const assignmentId = "task /?#%";
    expect(parseRoute(paths.lectures(courseId)).courseId).toBe(courseId);
    expect(parseRoute(paths.lectures(courseId, assignmentId))).toMatchObject({
      page: "lectures",
      courseId,
      assignmentId,
    });
  });

  it.each<DrawerTab>(["overview", "materials", "assignments", "syllabus"])(
    "round-trips the %s drawer tab on every page",
    (tab) => {
      for (const page of [
        paths.home("week"),
        paths.lectures("c", "a"),
        paths.web(),
        paths.settings("study"),
      ]) {
        const target = paths.drawer(`${page}?other=value`, "课程 /?#", tab);
        expect(parseRoute(target).drawer).toEqual({
          courseId: "课程 /?#",
          tab,
        });
        expect(paths.drawer(target, null)).toBe(`${page}?other=value`);
      }
    },
  );

  it("defaults a missing or unrecognised drawer tab to overview", () => {
    for (const query of ["?course=c", "?course=c&tab=bad", "?course=c&tab="]) {
      expect(parseRoute(paths.web() + query).drawer).toEqual({
        courseId: "c",
        tab: "overview",
      });
    }
    expect(parseRoute(paths.home()).drawer).toEqual({
      courseId: null,
      tab: "overview",
    });
  });

  it("redirects an unknown path to home while keeping the query", () => {
    expect(routeRedirect("/")).toBe(paths.home());
    expect(routeRedirect("/unknown?course=c&tab=syllabus")).toBe(
      `${paths.home()}?course=c&tab=syllabus`,
    );
  });

  it("redirects a bare or unknown settings path to the first section", () => {
    expect(routeRedirect(paths.settings())).toBe(paths.settings("accounts"));
    expect(routeRedirect("/settings/bad")).toBe(paths.settings("accounts"));
  });

  it("redirects a missing or deleted course back to the lecture list", () => {
    expect(routeRedirect(paths.lectures("missing"), { courses: [] })).toBe(
      paths.lectures(),
    );
    expect(
      routeRedirect(paths.lectures("deleted", "a"), {
        courses: [{ id: "deleted", deletedAt: "now" }],
      }),
    ).toBe(paths.lectures());
    expect(
      routeRedirect(paths.lectures("c"), { courses: [{ id: "c" }] }),
    ).toBeUndefined();
  });

  it("keeps desktop-only and developer sections off other platforms", () => {
    expect(
      routeRedirect(paths.settings("files"), {
        context: { ...desktop, platform: "browser-preview" },
      }),
    ).toBe(paths.settings("accounts"));
    for (const section of ["diagnostics", "extensions"] as const) {
      expect(
        routeRedirect(paths.settings(section), {
          context: { ...desktop, developer: false },
        }),
      ).toBe(paths.settings("accounts"));
    }
    expect(
      routeRedirect(paths.settings("extensions"), {
        context: { ...desktop, extensionsAvailable: false },
      }),
    ).toBe(paths.settings("accounts"));
  });

  it("keeps the nested academic calendar reachable on the desktop", () => {
    expect(
      routeRedirect(paths.settings("calendar"), { context: desktop }),
    ).toBeUndefined();
  });

  it.each(["/", "/unknown", "/settings"])(
    "the data router replaces %s with the canonical path",
    async (initial) => {
      const router = createMemoryRouter(
        workspaceRoutes(() => null),
        {
          initialEntries: [`${initial}?course=c&tab=syllabus`],
        },
      );
      await new Promise<void>((resolve) => {
        const off = router.subscribe((state) => {
          if (state.initialized && state.navigation.state === "idle") {
            off();
            resolve();
          }
        });
      });

      expect(router.state.location.pathname).toBe(
        initial === "/settings" ? paths.settings("accounts") : paths.home(),
      );
      expect(router.state.location.search).toBe("?course=c&tab=syllabus");
      expect(router.state.historyAction).toBe("REPLACE");
      router.dispose();
    },
  );
});

describe("public action policy", () => {
  it.each<AppAction>(["edit-academic-data", "import-school", "push-cloud"])(
    "blocks %s in the browser preview but allows it on the native shell",
    (action) => {
      expect(can(action, "browser-preview").allowed).toBe(false);
      expect(() => requireAction(action, "browser-preview")).toThrow(
        "read-only",
      );
      expect(can(action, "native-desktop").allowed).toBe(true);
    },
  );

  it.each<AppAction>([
    "edit-calendar-advanced",
    "customize-school-site",
    "configure-unattended-schedule",
    "resolve-cloud-manually",
  ])("keeps %s desktop-only", (action) => {
    expect(can(action, "browser-preview").allowed).toBe(false);
    expect(can(action, "native-desktop").allowed).toBe(true);
  });

  it("does not run a blocked operation", async () => {
    let ran = false;

    await expect(
      performAction("import-school", "browser-preview", () => {
        ran = true;
        return 1;
      }),
    ).rejects.toThrow("desktop app");
    expect(ran).toBe(false);

    await expect(
      performAction("view", "browser-preview", () => 7),
    ).resolves.toBe(7);
  });
});

const runtime = (
  overrides: Partial<CapabilityRuntime> = {},
): CapabilityRuntime => ({
  native: {
    schoolImport: true,
    schoolWeb: true,
    notifications: true,
    backgroundSchedule: true,
    cloudSync: true,
    matrixOcr: true,
  },
  school: { status: "not-configured" },
  background: { scheduler: "off" },
  noBackendPush: true,
  ...overrides,
});

describe("runtime capabilities", () => {
  it("reports a validated native school session as ready", () => {
    const capabilities = resolveCapabilities(
      "native-desktop",
      false,
      "granted",
      runtime({
        school: { status: "connected", lastResult: "fresh" },
        background: { scheduler: "active" },
      }),
    );

    expect(capabilities["school-web-session"].readiness).toBe("ready");
    expect(capabilities["background-sync"].readiness).toBe("ready");
    expect(capabilities["site-customization"].support).toBe("supported");
  });

  it("does not claim native adapters exist until the runtime acknowledges them", () => {
    const capabilities = resolveCapabilities(
      "native-desktop",
      false,
      "default",
      runtime({ native: undefined }),
    );

    expect(capabilities["school-web-session"].support).toBe("unknown");
    expect(capabilities["background-sync"].support).toBe("unknown");
    expect(capabilities["site-customization"].support).toBe("unknown");
  });

  it("describes browser preview without PWA capabilities", () => {
    const capabilities = resolveCapabilities(
      "browser-preview",
      false,
      "denied",
      runtime(),
    );

    expect(capabilities["school-web-session"].support).toBe("unsupported");
    expect(capabilities["background-sync"].support).toBe("unsupported");
    expect(capabilities["site-customization"].support).toBe("unsupported");
  });

  it("reports notification readiness from the granted permission", () => {
    expect(
      resolveCapabilities("native-desktop", false, "granted", runtime())[
        "local-notifications"
      ],
    ).toMatchObject({ support: "supported", readiness: "ready" });
    expect(
      resolveCapabilities("native-desktop", false, "denied", runtime())[
        "local-notifications"
      ],
    ).toMatchObject({
      support: "supported",
      readiness: "temporarily-unavailable",
    });
    expect(
      resolveCapabilities(
        "native-desktop",
        false,
        "granted",
        runtime({
          native: undefined,
        }),
      )["local-notifications"],
    ).toMatchObject({ support: "unsupported" });
  });

  it("detects the platform from the injected native target", () => {
    expect(detectPlatform()).toBe("browser-preview");

    installHost({ workspace: workspaceFixture() });
    expect(detectPlatform()).toBe("native-desktop");
  });
});

type Invocation = { command: string; payload?: Record<string, unknown> };

/** Records every native command the bridge sends, then answers it. */
function invokeSeam(
  answer: (command: string, payload?: Record<string, unknown>) => unknown,
) {
  const calls: Invocation[] = [];
  const invoke: InvokeNative = async (command, payload) => {
    calls.push({ command, payload });
    return answer(command, payload);
  };
  return { invoke, calls };
}

/** Captures the handler the bridge registers and counts its unlistens. */
function listenerSeam() {
  const names: string[] = [];
  let deliver: ((value: unknown) => void) | undefined;
  let unlistens = 0;
  const listen = (async (event: string, callback: (value: unknown) => void) => {
    names.push(event);
    deliver = callback;
    return () => {
      unlistens += 1;
    };
  }) as ListenNative;
  return {
    listen,
    names,
    deliver(value: unknown) {
      deliver?.(value);
    },
    unlistens: () => unlistens,
  };
}

const snapshot = (generation: number, done: number): SyncSnapshot => ({
  generation,
  resetting: false,
  runs: [
    {
      run: "run-1",
      dataset: "lms.course",
      label: { key: "sync.dataset.lms.course", args: {} },
      priority: "visible",
      done,
      total: 9,
      current: null,
      scope: "42",
      held: null,
      failure: null,
    },
  ],
  sources: [],
});

describe("Tauri bridge contract", () => {
  it("sends hints under the `hint` argument the native command reads", async () => {
    const { invoke, calls } = invokeSeam(() => null);
    const bridge = createTauriDesktopBridge(invoke, listenerSeam().listen);

    await bridge.syncHint!({
      kind: "refresh",
      scope: { kind: "course", lmsCourseId: 42 },
    });
    await bridge.syncHint!({
      kind: "open",
      view: { kind: "finder", year: 2026 },
    });

    expect(calls).toEqual([
      {
        command: "sync_hint",
        payload: {
          hint: { kind: "refresh", scope: { kind: "course", lmsCourseId: 42 } },
        },
      },
      {
        command: "sync_hint",
        payload: {
          hint: { kind: "open", view: { kind: "finder", year: 2026 } },
        },
      },
    ]);
  });

  it("listens for sync snapshots before reading one and ignores a late stale read", async () => {
    let releaseRead: ((value: unknown) => void) | undefined;
    const { invoke } = invokeSeam((command) => {
      if (command === "sync_snapshot")
        return new Promise((resolve) => {
          releaseRead = resolve;
        });
      throw new Error(`unexpected command ${command}`);
    });
    const seam = listenerSeam();
    const bridge = createTauriDesktopBridge(invoke, seam.listen);

    const received: SyncSnapshot[] = [];
    const dispose = bridge.onSyncSnapshot!((value) => received.push(value));
    await settle();
    expect(seam.names).toEqual(["sync-snapshot"]);

    const current = snapshot(3, 5);
    seam.deliver(current);
    seam.deliver({ not: "a snapshot" });
    releaseRead?.(snapshot(2, 1));
    await settle();
    expect(received).toEqual([current]);

    dispose();
    seam.deliver(snapshot(4, 6));
    expect(received).toEqual([current]);
    expect(seam.unlistens()).toBe(1);
  });

  it("maps credential, session and webview calls to narrow commands", async () => {
    const { invoke, calls } = invokeSeam((command) => {
      switch (command) {
        case "save_school_credentials":
        case "navigate_school_web":
        case "set_school_web_zoom":
        case "school_web_forward":
          return true;
        case "school_session_snapshot":
          return {
            revision: 4,
            states: { science: "active", tokyoTech: "pending" },
          };
        case "school_session_lifecycle":
          return null;
        case "school_web_back":
          return false;
        case "open_school_site":
          return {
            site: "science-portal",
            url: "https://example.test",
            phase: "pageLoaded",
          };
        default:
          throw new Error(`unexpected command ${command}`);
      }
    });
    const bridge = createTauriDesktopBridge(invoke, listenerSeam().listen);
    const credentials = {
      username: "student",
      password: "secret",
      otpSecret: "OTP",
      matrix: "A".repeat(70),
    };

    await bridge.saveSchoolCredentials("science", credentials);
    await expect(bridge.schoolSessionSnapshot()).resolves.toEqual({
      revision: 4,
      states: { science: "active", tokyoTech: "pending" },
    });
    await bridge.schoolSessionLifecycle({ kind: "focus", visible: true });
    await bridge.openSchoolSite("science-portal", "body{}", "window.x=1", {
      x: 1,
      y: 2,
      width: 3,
      height: 4,
    });
    await bridge.navigateSchoolWeb?.(
      "science-portal",
      "https://isct.ex-tic.com/idm/user/portal/",
    );
    await bridge.setSchoolWebZoom?.("science-portal", 1.2);
    await expect(bridge.schoolWebBack?.("kyomu")).resolves.toBe(false);
    await expect(bridge.schoolWebForward?.("kyomu")).resolves.toBe(true);

    expect(calls).toEqual([
      {
        command: "save_school_credentials",
        payload: { profile: "science", credentials },
      },
      { command: "school_session_snapshot", payload: undefined },
      {
        command: "school_session_lifecycle",
        payload: { kind: "focus", visible: true },
      },
      {
        command: "open_school_site",
        payload: {
          site: "science-portal",
          css: "body{}",
          js: "window.x=1",
          bounds: { x: 1, y: 2, width: 3, height: 4 },
          targetUrl: null,
        },
      },
      {
        command: "navigate_school_web",
        payload: {
          site: "science-portal",
          url: "https://isct.ex-tic.com/idm/user/portal/",
        },
      },
      {
        command: "set_school_web_zoom",
        payload: { site: "science-portal", scale: 1.2 },
      },
      { command: "school_web_back", payload: { site: "kyomu" } },
      { command: "school_web_forward", payload: { site: "kyomu" } },
    ]);
  });

  it("copies the saved Matrix through its own command", async () => {
    const { invoke, calls } = invokeSeam(() => true);
    const bridge = createTauriDesktopBridge(invoke, listenerSeam().listen);

    await expect(bridge.copySchoolMatrix()).resolves.toBe(true);
    expect(calls).toEqual([
      { command: "copy_school_matrix", payload: undefined },
    ]);
  });

  it("reveals a saved school download and filters download events", async () => {
    const { invoke, calls } = invokeSeam(() => true);
    const seam = listenerSeam();
    const bridge = createTauriDesktopBridge(invoke, seam.listen);
    const path = "/Users/student/Downloads/LAIIAssignment1.pdf";

    await expect(bridge.revealSchoolDownload?.(path)).resolves.toBe(true);
    expect(calls).toEqual([
      { command: "reveal_school_download", payload: { path } },
    ]);

    const received: SchoolDownloadEvent[] = [];
    const dispose = bridge.onSchoolDownload!((event) => received.push(event));
    await settle();

    const site = "science-lms";
    seam.deliver({ site, state: "started", name: "a.docx", url: "https://x" });
    seam.deliver({ site, state: "done", name: "a.docx", path });
    seam.deliver({ site, state: "failed", name: "a.docx" });
    seam.deliver({ site, state: "done", name: "a.docx" });
    seam.deliver({ site, state: "paused", name: "a.docx" });
    seam.deliver({ site, state: "done", name: 1, path });
    seam.deliver(null);
    expect(received).toEqual([
      { site, state: "started", name: "a.docx", path: undefined },
      { site, state: "done", name: "a.docx", path },
      { site, state: "failed", name: "a.docx", path: undefined },
    ]);

    dispose();
    seam.deliver({ site, state: "started", name: "b.docx" });
    expect(received).toHaveLength(3);
    expect(seam.unlistens()).toBe(1);
  });

  it("maps saveFile to the native save_file command and returns its answer", async () => {
    const { invoke, calls } = invokeSeam(() => false);
    const bridge = createTauriDesktopBridge(invoke, listenerSeam().listen);
    const name = "campus-life-2026-2Q.ics";
    const contents = "BEGIN:VCALENDAR\r\nEND:VCALENDAR";

    await expect(bridge.saveFile(name, contents)).resolves.toBe(false);
    expect(calls).toEqual([
      { command: "save_file", payload: { name, contents } },
    ]);
  });

  it("rejects a native answer that is not the promised shape", async () => {
    const { invoke } = invokeSeam((command) => {
      switch (command) {
        case "delete_school_credentials":
          return "yes";
        case "request_notification_permission":
          return "maybe";
        case "sync_snapshot":
          return { runs: "later" };
        case "school_session_snapshot":
          return { revision: -1, states: {} };
        case "school_credential_status":
          return [];
        default:
          return undefined;
      }
    });
    const bridge = createTauriDesktopBridge(invoke, listenerSeam().listen);

    await expect(bridge.deleteSchoolCredentials("science")).rejects.toThrow(
      "delete_school_credentials returned an invalid result",
    );
    await expect(bridge.requestNotificationPermission()).rejects.toThrow(
      "request_notification_permission returned an invalid result",
    );
    await expect(bridge.syncSnapshot!()).rejects.toThrow(
      "sync_snapshot returned an invalid result",
    );
    await expect(bridge.schoolSessionSnapshot()).rejects.toThrow(
      "school_session_snapshot returned an invalid result",
    );
    await expect(bridge.schoolCredentialStatus()).rejects.toThrow(
      "school_credential_status returned an invalid result",
    );
  });

  it("filters malformed session events and disposes its listener", async () => {
    const seam = listenerSeam();
    const bridge = createTauriDesktopBridge(
      invokeSeam(() => undefined).invoke,
      seam.listen,
    );
    const received: SchoolSessionEvent[] = [];
    const dispose = await bridge.onSchoolSession((event) =>
      received.push(event),
    );

    seam.deliver({ profile: "science", state: "active", revision: 2 });
    seam.deliver({ profile: "other", state: "active", revision: 3 });
    seam.deliver({ profile: "science", state: "broken", revision: 4 });
    seam.deliver({ profile: "science", state: "failed", revision: -1 });
    expect(received).toEqual([
      { profile: "science", state: "active", revision: 2 },
    ]);

    dispose();
    seam.deliver({ profile: "science", state: "active", revision: 5 });
    expect(received).toHaveLength(1);
    expect(seam.unlistens()).toBe(1);
  });

  it("forwards correlated workspace changes and filters malformed payloads", async () => {
    const seam = listenerSeam();
    const bridge = createTauriDesktopBridge(
      invokeSeam(() => undefined).invoke,
      seam.listen,
    );
    const received: WorkspaceChangedEvent[] = [];
    const dispose = await bridge.onWorkspaceChanged((event) =>
      received.push(event),
    );

    seam.deliver({
      source: "user-intent",
      revision: "r1",
      changes: ["settings"],
    });
    seam.deliver({ source: "mcp", changes: ["events"] });
    seam.deliver({ source: "future-source", changes: ["future-change"] });
    seam.deliver({ source: 1, changes: [] });
    seam.deliver({ source: "user-intent", revision: 1, changes: [] });
    // Without its change names a shell could not tell what to read.
    seam.deliver({ source: "school-pull" });
    seam.deliver({ source: "school-pull", changes: [1] });
    seam.deliver(null);
    expect(received).toEqual([
      { source: "user-intent", revision: "r1", changes: ["settings"] },
      { source: "mcp", changes: ["events"] },
      { source: "future-source", changes: ["future-change"] },
    ]);

    dispose();
    expect(seam.unlistens()).toBe(1);
  });

  it("unlistens a handler that was disposed before its registration resolved", async () => {
    let release: ((off: () => void) => void) | undefined;
    let unlistens = 0;
    const listen = (async () =>
      new Promise<() => void>((resolve) => {
        release = resolve;
      })) as ListenNative;
    const bridge = createTauriDesktopBridge(
      invokeSeam(() => undefined).invoke,
      listen,
    );

    const dispose = bridge.onSchoolLoginStatus!(() => undefined);
    dispose();
    release?.(() => {
      unlistens += 1;
    });
    await settle();

    expect(unlistens).toBe(1);
  });
});

/* ------------------------------------------------------------------ *
 * Sync snapshot surfaces: which row of Core's snapshot a surface shows
 * ------------------------------------------------------------------ */

const run = (overrides: Partial<RunView>): RunView => ({
  run: "run",
  dataset: "lms.course",
  label: { key: "sync.dataset.lms.course", args: {} },
  priority: "visible",
  done: 0,
  total: null,
  current: null,
  scope: null,
  held: null,
  failure: null,
  ...overrides,
});
const syncOf = (...runs: RunView[]): SyncSnapshot => ({
  generation: 1,
  resetting: false,
  runs,
  sources: [],
});

describe("sync snapshot surfaces", () => {
  it("shows a bar only for a run of known size and a spinner for background-only work", () => {
    expect(progressLine(syncOf())).toEqual({ kind: "idle" });
    const background = run({ run: "bg", priority: "background", total: 40 });
    expect(progressLine(syncOf(background))).toMatchObject({
      kind: "background",
    });
    const unsized = run({ run: "vis" });
    expect(progressLine(syncOf(background, unsized))).toMatchObject({
      kind: "busy",
      run: { run: "vis" },
    });
    // The page the person opened outranks Core's visible work.
    const opened = run({
      run: "open",
      priority: "interactive",
      done: 4,
      total: 9,
    });
    expect(progressLine(syncOf(unsized, opened))).toEqual({
      kind: "bar",
      run: opened,
      done: 4,
      total: 9,
    });
  });

  it("names a held foreground run before quiet background work, and skips failed runs", () => {
    const held = run({ run: "held", held: { kind: "cellular" } });
    const failed = run({
      run: "failed",
      total: 3,
      failure: { kind: "http-503", keysFailed: 1, retryAt: null },
    });
    const background = run({ run: "bg", priority: "background" });
    expect(progressLine(syncOf(failed, held, background))).toMatchObject({
      kind: "held",
      held: { kind: "cellular" },
    });
    expect(progressLine(syncOf(failed))).toEqual({ kind: "idle" });
  });

  it("sums a page's runs by scope, and drops the total when any run's size is unknown", () => {
    const snapshot = syncOf(
      run({ run: "a", scope: "42", done: 2, total: 5 }),
      run({ run: "b", dataset: "lms.assign", scope: "42", done: 1, total: 1 }),
      run({ run: "c", scope: "43", done: 7, total: 7 }),
    );
    expect(
      pageProgress(runsFor(snapshot, ["lms.course", "lms.assign"], "42")),
    ).toEqual({ done: 3, total: 6 });
    expect(pageProgress(runsFor(snapshot, ["lms"]))).toEqual({
      done: 10,
      total: 13,
    });
    expect(
      pageProgress([run({ done: 1, total: 4 }), run({ done: 2 })]),
    ).toEqual({ done: 3, total: null });
    expect(pageProgress(runsFor(snapshot, ["kyomu"]))).toBeNull();
  });

  it("retries a failed dataset at its own scope", () => {
    expect(retryScope(run({ scope: "42" }))).toEqual({
      kind: "course",
      lmsCourseId: 42,
    });
    expect(retryScope(run({ dataset: "lms.deadlines" }))).toEqual({
      kind: "site",
      site: "science-lms",
    });
    expect(retryScope(run({ dataset: "kyomu.page", scope: "gpa" }))).toEqual({
      kind: "site",
      site: "kyomu",
    });
    expect(
      retryScope(run({ dataset: "catalog.official", scope: "2026" })),
    ).toEqual({ kind: "catalog", year: 2026 });
    // An offering id is not a year.
    expect(
      retryScope(run({ dataset: "catalog.detail", scope: "202602431" })),
    ).toEqual({ kind: "everything" });
    expect(retryScope(run({ dataset: "reference" }))).toEqual({
      kind: "references",
    });
  });
});
