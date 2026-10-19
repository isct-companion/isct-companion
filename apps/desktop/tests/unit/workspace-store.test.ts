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
  CoreWorkspaceSnapshot,
  WorkspaceIntent,
  WorkspaceMutation,
} from "@app/lib/workspace-intents";
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
  schoolPullForDisplay,
  type InvokeNative,
  type ListenNative,
  type PullSnapshot,
  type SchoolDownloadEvent,
  type SchoolSessionEvent,
  type WorkspaceChangedEvent,
} from "@app/platform/desktop-bridge";
import { finderQuery } from "@app/platform/finder-query";
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
    mutate(request: WorkspaceMutation): Promise<CoreWorkspaceSnapshot>;
  };
  /** The revision each read started from, in order. */
  reads: string[];
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
    });

    expect(host.reads).toEqual(["r0", "external-r1"]);
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
    });

    expect(host.reads).toEqual(["r0"]);
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

  it("re-reads non-user events even when the user revision is unchanged", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);
    await store.initialize();

    host.publish(settingsWith({ locale: "ja" }));
    await store.handleWorkspaceChanged({ source: "mcp" });
    expect(store.getState().workspace?.settings.locale).toBe("ja");

    host.publish(settingsWith({ locale: "zh" }));
    await store.handleWorkspaceChanged({ source: "school-pull" });
    expect(store.getState().workspace?.settings.locale).toBe("zh");

    expect(host.reads).toEqual(["r0", "r0", "r0"]);
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

describe("school pulls", () => {
  it("reloads the unified snapshot after a pull", async () => {
    const host = coreHost();
    const pulls: string[] = [];
    const store = createWorkspaceStore(host.service, {
      async pull(site) {
        pulls.push(site);
        host.publish(settingsWith({ locale: "ja" }), "r5");
        return [];
      },
    });
    await store.initialize();

    await expect(store.pullSchool("science-lms")).resolves.toEqual([]);

    expect(pulls).toEqual(["science-lms"]);
    expect(host.reads).toEqual(["r0", "r5"]);
    expect(store.getState()).toMatchObject({ revision: "r5" });
    expect(store.getState().workspace?.settings.locale).toBe("ja");
  });

  it("refuses a school pull without the desktop coordinator", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);

    await expect(store.pullSchool("kyomu")).rejects.toThrow("desktop app");
    expect(host.reads).toEqual([]);
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
      async reset() {
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

  it("refuses export, import and reset without the native app", async () => {
    const repository = new LocalRepository();

    await expect(repository.export()).rejects.toThrow("native app");
    await expect(repository.import("{}")).rejects.toThrow("native app");
    await expect(repository.reset()).rejects.toThrow("native app");
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
        if (
          command === "load_workspace" ||
          command === "import_data" ||
          command === "reset_workspace"
        )
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
    expect((await repository.reset()).deviceId).toBe(DEVICE);
    expect(core.calls).toEqual([
      { command: "load_workspace", args: {} },
      { command: "export_data", args: {} },
      {
        command: "import_data",
        args: { document: "unparsed bytes for Core" },
      },
      { command: "reset_workspace", args: {} },
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

  it("accepts the bare settings route in the stacked shell", () => {
    expect(routeRedirect(paths.settings(), { stacked: true })).toBeUndefined();
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

const pull = (
  id: string,
  state: PullSnapshot["state"],
  courseId: number | null = null,
): PullSnapshot => ({
  id,
  target: { type: "school", site: "science-lms", courseId },
  priority: "automatic",
  state,
  phase: null,
  blockedBy: null,
  attempt: 1,
  waiterCount: 0,
  lastError: null,
});

describe("Tauri bridge contract", () => {
  it("shows the executing manual pull ahead of an older blocked full pull", () => {
    const jobs = [pull("full", "blocked"), pull("detail", "running", 42)];

    expect(schoolPullForDisplay(jobs, "science-lms")?.id).toBe("detail");

    jobs[1].state = "succeeded";
    expect(schoolPullForDisplay(jobs, "science-lms")?.id).toBe("full");
    expect(schoolPullForDisplay(jobs, "kyomu")).toBeUndefined();
  });

  it("passes an optional LMS course scope to the native pull", async () => {
    const { invoke, calls } = invokeSeam(() => []);
    const bridge = createTauriDesktopBridge(invoke, listenerSeam().listen);

    await bridge.pullSchool?.("science-lms", { lmsCourseId: 42 });

    expect(calls).toEqual([
      {
        command: "pull_school",
        payload: { site: "science-lms", lmsCourseId: 42 },
      },
    ]);
  });

  it("listens for pull state before reading it and ignores a late stale read", async () => {
    let releaseRead: ((value: unknown) => void) | undefined;
    const { invoke } = invokeSeam((command) => {
      if (command === "pull_state")
        return new Promise((resolve) => {
          releaseRead = resolve;
        });
      throw new Error(`unexpected command ${command}`);
    });
    const seam = listenerSeam();
    const bridge = createTauriDesktopBridge(invoke, seam.listen);

    const received: PullSnapshot[][] = [];
    const dispose = bridge.onPullState!((states) => received.push(states));
    await settle();
    expect(seam.names).toEqual(["pull-state-changed"]);

    const current = [pull("pull-1", "succeeded")];
    seam.deliver(current);
    releaseRead?.([pull("pull-1", "running")]);
    await settle();
    expect(received).toEqual([current]);

    dispose();
    seam.deliver([]);
    expect(received).toEqual([current]);
    expect(seam.unlistens()).toBe(1);
  });

  it("routes cancellation and reference refresh to the native facade", async () => {
    const { invoke, calls } = invokeSeam((command) =>
      command === "cancel_pull" ? true : "Updated",
    );
    const bridge = createTauriDesktopBridge(invoke, listenerSeam().listen);

    await expect(bridge.cancelPull!("pull-1")).resolves.toBe(true);
    await expect(bridge.refreshReferenceSubscriptions!(true)).resolves.toBe(
      "Updated",
    );
    expect(calls).toEqual([
      { command: "cancel_pull", payload: { id: "pull-1" } },
      { command: "refresh_reference_subscriptions", payload: { force: true } },
    ]);
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
        case "pull_school":
          return { not: "a list" };
        case "pull_state":
          return "later";
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
    await expect(bridge.pullSchool!("science-lms")).rejects.toThrow(
      "pull_school returned an invalid result",
    );
    await expect(bridge.getPullState!()).rejects.toThrow(
      "pull_state returned an invalid result",
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

    seam.deliver({ source: "user-intent", revision: "r1" });
    seam.deliver({ source: "mcp" });
    seam.deliver({ source: "future-source" });
    seam.deliver({ source: 1 });
    seam.deliver({ source: "user-intent", revision: 1 });
    seam.deliver(null);
    expect(received).toEqual([
      { source: "user-intent", revision: "r1" },
      { source: "mcp" },
      { source: "future-source" },
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
