// @vitest-environment jsdom
import { createMemoryRouter } from "react-router";
import { afterEach, describe, expect, it } from "vitest";

import {
  parseRoute,
  paths,
  routeRedirect,
  workspaceRoutes,
} from "@app/app/routes";
import { seedWorkspace, type Workspace } from "@app/lib/model";
import type {
  CoreWorkspaceCommit,
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
  getDesktopBridge,
  type InvokeNative,
  type ListenNative,
  type SchoolDownloadEvent,
  type SchoolRecords,
  type SchoolSessionEvent,
  type WorkspaceChangedEvent,
} from "@app/platform/desktop-bridge";
import { finderQuery } from "@app/platform/finder-query";
import {
  changesCommitted,
  freshAnswer,
  queryClient,
  resetQueries,
  type CoreAnswer,
} from "@app/platform/query-client";
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
  type WorkspaceState,
} from "@app/store/workspace";
import {
  DEVICE,
  emptySchoolRecords,
  installFakeCore,
  workspaceFixture,
  type FakeCore,
  type FakeCoreOptions,
  schoolProfiles,
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

/** A settings-form intent changing one preference of the seed settings. */
const appearance = (
  value: Workspace["settings"]["appearance"],
): WorkspaceIntent => ({
  kind: "replaceSettings",
  settings: { ...seedWorkspace().settings, appearance: value },
});

type CoreHost = {
  service: {
    revision(): Promise<string>;
    mutate(request: WorkspaceMutation): Promise<CoreWorkspaceCommit>;
  };
  /** How many times the store asked for the revision. */
  revisionReads(): number;
  /** Every mutation the store submitted, in order. */
  mutations: WorkspaceMutation[];
  /** Another client committed: the stored revision moves on. */
  publish(revision: string): void;
  answerRevision(answer: Promise<string> | Error): void;
  answerMutation(answer: Promise<CoreWorkspaceCommit> | Error): void;
};

/**
 * A recording Core for the store suite: the revision the next intent is
 * made against, and the commits of the intents themselves. Records never
 * reach the store; they live in the query cache.
 */
function coreHost(): CoreHost {
  let revision = "r0";
  let reads = 0;
  let seq = 0;
  const mutations: WorkspaceMutation[] = [];
  const revisionAnswers: (Promise<string> | Error)[] = [];
  const mutationAnswers: (Promise<CoreWorkspaceCommit> | Error)[] = [];
  return {
    service: {
      async revision() {
        reads += 1;
        const answer = revisionAnswers.shift();
        if (answer instanceof Error) throw answer;
        return answer ?? revision;
      },
      async mutate(request) {
        mutations.push(request);
        const answer = mutationAnswers.shift();
        if (answer instanceof Error) throw answer;
        if (answer) return answer;
        if (request.expectedRevision !== revision)
          throw new Error("revision conflict");
        revision = `r${Number(revision.slice(1)) + 1}`;
        return { changes: ["settings"], revision, seq: ++seq };
      },
    },
    revisionReads: () => reads,
    mutations,
    publish(next) {
      revision = next;
    },
    answerRevision(answer) {
      revisionAnswers.push(answer);
    },
    answerMutation(answer) {
      mutationAnswers.push(answer);
    },
  };
}

describe("opening the workspace", () => {
  it("reads the revision once when several consumers initialize together", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);

    const first = store.initialize();
    const second = store.initialize();
    expect(store.getState().status).toBe("loading");
    await Promise.all([first, second]);

    expect(store.getState()).toMatchObject({ status: "ready", revision: "r0" });
    expect(host.revisionReads()).toBe(1);
  });

  it("reports an open failure and refuses to write before it opens", async () => {
    const host = coreHost();
    host.answerRevision(new Error("database locked"));
    const store = createWorkspaceStore(host.service);

    await store.initialize();

    expect(store.getState().status).toBe("error");
    await expect(store.flush()).rejects.toThrow("could not be opened");
    await expect(store.mutate(appearance("dark"))).rejects.toThrow("not ready");
    expect(host.mutations).toEqual([]);
  });
});

describe("submitting coarse intents", () => {
  it("serializes rapid intents against the revision each preceding write committed", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);
    await store.initialize();

    await Promise.all([
      store.mutate(appearance("dark")),
      store.mutate(appearance("light")),
    ]);

    expect(host.mutations.map((mutation) => mutation.expectedRevision)).toEqual(
      ["r0", "r1"],
    );
    expect(store.getState().revision).toBe("r2");
    // Each commit counts its changes for the finder's open page.
    expect(store.getState().viewRevisions.settings).toBe(2);
  });

  it("builds a form intent from the records Core holds once the preceding write committed", async () => {
    const stored = workspaceFixture();
    const core = installHost({
      workspace: stored,
      revision: "r0",
      apply: (intent, live) => {
        if (intent.kind === "replaceSettings")
          live.workspace.settings = structuredClone(intent.settings);
        live.revision = `${live.revision}+`;
      },
    });
    const bridge = getDesktopBridge()!;
    const store = createWorkspaceStore({
      revision: () => bridge.workspaceRevision(),
      mutate: (mutation) => bridge.mutateWorkspace(mutation),
    });
    await store.initialize();
    const edit = (change: (draft: Workspace) => void) =>
      store.mutate({
        fields: ["settings"],
        build: (draft) => {
          change(draft);
          return { kind: "replaceSettings", settings: draft.settings };
        },
      });

    await Promise.all([
      edit((draft) => {
        draft.settings.appearance = "dark";
      }),
      edit((draft) => {
        draft.settings.density = "compact";
      }),
    ]);

    // The second edit started from what the first committed.
    const last = core.intents.at(-1);
    expect(last).toMatchObject({
      kind: "replaceSettings",
      settings: { appearance: "dark", density: "compact" },
    });
  });
});

describe("recovering from a rejected or external write", () => {
  it("never replays a rejected intent and reads where the next one starts", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);
    await store.initialize();
    host.publish("external-r1");

    await expect(store.mutate(appearance("dark"))).rejects.toThrow(
      "revision conflict",
    );

    expect(host.mutations).toHaveLength(1);
    expect(store.getState().revision).toBe("external-r1");
    await store.mutate(appearance("dark"));
    expect(host.mutations.at(-1)?.expectedRevision).toBe("external-r1");
  });

  it("adopts the revision a user-intent event names for another client", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);
    await store.initialize();

    await store.handleWorkspaceChanged({
      source: "user-intent",
      revision: "external-r1",
      changes: ["settings"],
      seq: 4,
    });

    expect(store.getState().revision).toBe("external-r1");
    expect(host.revisionReads()).toBe(1);
  });

  it("recognizes its own committed revision after waiting for the write", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);
    await store.initialize();
    const slow = deferred<CoreWorkspaceCommit>();
    host.answerMutation(slow.promise);

    const write = store.mutate(appearance("dark"));
    await settle();
    // Core emits before the command resolves.
    const heard = store.handleWorkspaceChanged({
      source: "user-intent",
      revision: "r1",
      changes: ["settings"],
      seq: 1,
    });
    slow.resolve({ changes: ["settings"], revision: "r1", seq: 1 });
    await Promise.all([write, heard]);

    expect(store.getState().revision).toBe("r1");
    expect(host.revisionReads()).toBe(1);
  });

  it("reads everything again when a user intent names no revision", async () => {
    const host = coreHost();
    const store = createWorkspaceStore(host.service);
    await store.initialize();
    host.publish("r5");

    await store.handleWorkspaceChanged({
      source: "user-intent",
      changes: ["settings"],
      seq: 2,
    });

    expect(store.getState().revision).toBe("r5");
    expect(store.getState().viewRevisions.all).toBe(1);
  });
});

describe("browser preview", () => {
  it("opens without Core and refuses every mutation", async () => {
    const store = createWorkspaceStore(createBrowserPreviewService());
    await store.initialize();

    expect(store.getState()).toMatchObject({
      status: "ready",
      revision: "preview",
    });
    await expect(store.mutate(appearance("dark"))).rejects.toThrow(
      "native app",
    );
  });

  it("keeps the shipped store on the preview service", async () => {
    await workspaceStore.initialize();

    expect(workspaceStore.getState().revision).toBe("preview");
    await expect(
      workspaceActions.mutate({ kind: "markTimetableReviewed" }),
    ).rejects.toThrow("native app");
  });
});

describe("the query cache", () => {
  afterEach(resetQueries);

  it("invalidates only the answers that read a committed change", async () => {
    const answer = (reads: WorkspaceChange[]): CoreAnswer<number> => ({
      value: 1,
      reads,
      seq: 1,
    });
    await queryClient.fetchQuery({
      queryKey: ["calendar"],
      queryFn: async () => answer(["calendar", "settings"]),
    });
    await queryClient.fetchQuery({
      queryKey: ["tasks"],
      queryFn: async () => answer(["assignments", "settings"]),
    });

    changesCommitted(["calendar"], 2);

    const state = (key: string) => queryClient.getQueryState([key]);
    expect(state("calendar")?.isInvalidated).toBe(true);
    expect(state("tasks")?.isInvalidated).toBe(false);
    changesCommitted(["settings"], 3);
    expect(state("tasks")?.isInvalidated).toBe(true);
  });

  it("asks again for an answer older than the newest change heard", async () => {
    changesCommitted(["assignments"], 7);
    const seqs = [5, 7];
    const loads: number[] = [];
    const fresh = await freshAnswer(async () => {
      const seq = seqs.shift()!;
      loads.push(seq);
      return { value: seq, reads: ["assignments"], seq };
    });
    expect(fresh.seq).toBe(7);
    expect(loads).toEqual([5, 7]);
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
          reads: [],
          seq: 0,
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
          reads: ["registrations", "settings"],
          seq: 0,
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
  it("refuses export and import without the native app", async () => {
    const repository = new LocalRepository();

    await expect(repository.export()).rejects.toThrow("native app");
    await expect(repository.import("{}")).rejects.toThrow("native app");
  });
});

describe("SQLite repository", () => {
  it("forwards each exact command and returns the exported document untouched", async () => {
    const workspace = workspaceFixture();
    const exported = '{"kind":"IsctCompanion.local-backup"}';
    const core = installHost({
      workspace,
      handle: (command) => {
        if (command === "export_data") return exported;
        return undefined;
      },
    });
    const repository = new SqliteRepository();

    expect(await repository.export()).toBe(exported);
    await repository.import("unparsed bytes for Core");
    expect(core.calls).toEqual([
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

  it("redirects an unknown path to home while keeping the query", () => {
    expect(routeRedirect("/")).toBe(paths.home());
    expect(routeRedirect("/unknown?q=c&view=week")).toBe(
      `${paths.home()}?q=c&view=week`,
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
          initialEntries: [`${initial}?q=c&view=week`],
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
      expect(router.state.location.search).toBe("?q=c&view=week");
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
    expect(
      resolveCapabilities(
        "native-desktop",
        false,
        "denied",
        runtime({
          native: {
            ...runtime().native!,
            notifications: false,
            notificationUnavailableReason: "Open the installed app.",
          },
        }),
      )["local-notifications"],
    ).toMatchObject({
      support: "unsupported",
      reason: "Open the installed app.",
    });
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
        case "connect_school":
          return { state: "failed", problem: null, verified: false };
        case "navigate_school_web":
        case "set_school_web_zoom":
        case "school_web_forward":
          return true;
        case "school_session_snapshot":
          return {
            revision: 4,
            profiles: schoolProfiles("active", "pending"),
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

    await expect(
      bridge.connectSchool("science", credentials),
    ).resolves.toMatchObject({ state: "failed", verified: false });
    await expect(bridge.schoolSessionSnapshot()).resolves.toEqual({
      revision: 4,
      profiles: schoolProfiles("active", "pending"),
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
        command: "connect_school",
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
        case "notification_permission":
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
    await expect(bridge.getNotificationPermission()).rejects.toThrow(
      "notification_permission returned an invalid result",
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
      seq: 4,
    });
    seam.deliver({ source: "mcp", changes: ["events"], seq: 5 });
    seam.deliver({
      source: "future-source",
      changes: ["future-change"],
      seq: 6,
    });
    seam.deliver({ source: 1, changes: [], seq: 7 });
    seam.deliver({ source: "user-intent", revision: 1, changes: [], seq: 7 });
    // Without its change names a shell could not tell what to read, and
    // without its sequence it could not tell which answers it outdates.
    seam.deliver({ source: "school-pull", seq: 7 });
    seam.deliver({ source: "school-pull", changes: [1], seq: 7 });
    seam.deliver({ source: "school-pull", changes: ["grades"] });
    seam.deliver(null);
    expect(received).toEqual([
      { source: "user-intent", revision: "r1", changes: ["settings"], seq: 4 },
      { source: "mcp", changes: ["events"], seq: 5 },
      { source: "future-source", changes: ["future-change"], seq: 6 },
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
    // A scope that is not a year names no catalog.
    expect(
      retryScope(run({ dataset: "catalog.official", scope: "202602431" })),
    ).toEqual({ kind: "everything" });
    expect(retryScope(run({ dataset: "reference" }))).toEqual({
      kind: "references",
    });
  });
});
