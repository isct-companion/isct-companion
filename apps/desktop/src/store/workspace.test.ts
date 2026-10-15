import { describe, expect, it, vi } from "vitest";
import { seedWorkspace, type Workspace } from "../lib/model";
import type {
  CoreWorkspaceSnapshot,
  WorkspaceIntent,
  WorkspaceMutation,
} from "../lib/workspace-intents";
import { createBrowserPreviewService, createWorkspaceStore } from "./workspace";

const schoolRecords = () => ({
  lastRuns: {},
  lmsCourses: [],
  registrations: [],
  registrationStates: [],
  grades: [],
  gpaByTerm: [],
  requirements: [],
});
/** A settings-form intent changing one preference of the seed settings. */
const appearance = (
  value: Workspace["settings"]["appearance"],
): WorkspaceIntent => ({
  kind: "replaceSettings",
  settings: { ...seedWorkspace().settings, appearance: value },
});
function coreService(initial: Workspace = seedWorkspace()) {
  let generation = 0;
  let workspace = structuredClone(initial);
  const result = (): CoreWorkspaceSnapshot => ({
    revision: `r${generation}`,
    contentRevision: `r${generation}`,
    workspace: structuredClone(workspace),
    schoolRecords: schoolRecords(),
  });
  return {
    snapshot: vi.fn(async () => result()),
    mutate: vi.fn(async (request: WorkspaceMutation) => {
      if (request.expectedRevision !== `r${generation}`)
        throw new Error("revision conflict");
      if (request.intent.kind === "replaceSettings")
        workspace.settings = structuredClone(request.intent.settings);
      generation++;
      return result();
    }),
  };
}

describe("Zustand workspace snapshot cache", () => {
  it("deduplicates initialization and adopts one Core snapshot", async () => {
    const service = coreService();
    const store = createWorkspaceStore(service);
    await Promise.all([store.initialize(), store.initialize()]);
    expect(service.snapshot).toHaveBeenCalledTimes(1);
    expect(store.getState()).toMatchObject({ status: "ready", revision: "r0" });
  });
  it("sends coarse intents with the current revision and adopts the result", async () => {
    const service = coreService();
    const store = createWorkspaceStore(service);
    await store.initialize();
    await store.mutate(appearance("dark"));
    expect(service.mutate).toHaveBeenCalledWith({
      expectedRevision: "r0",
      intent: appearance("dark"),
    });
    expect(store.getState()).toMatchObject({ revision: "r1" });
    expect(store.getState().workspace?.settings.appearance).toBe("dark");
  });
  it("serializes rapid mutations using the preceding Core revision", async () => {
    const service = coreService();
    const store = createWorkspaceStore(service);
    await store.initialize();
    const first = store.mutate(appearance("dark"));
    const second = store.mutate(appearance("light"));
    await Promise.all([first, second]);
    expect(
      service.mutate.mock.calls.map(([request]) => request.expectedRevision),
    ).toEqual(["r0", "r1"]);
    expect(store.getState().workspace?.settings.appearance).toBe("light");
  });
  it("uses mutation responses when an event arrives before a newer queued write", async () => {
    const service = coreService();
    const store = createWorkspaceStore(service);
    await store.initialize();
    let release!: (snapshot: CoreWorkspaceSnapshot) => void;
    service.mutate
      .mockImplementationOnce(
        () => new Promise((resolve) => void (release = resolve)),
      )
      .mockImplementationOnce(async (request) => {
        expect(request.expectedRevision).toBe("r1");
        return {
          revision: "r2",
          contentRevision: "r2",
          workspace: {
            ...seedWorkspace(),
            settings: { ...seedWorkspace().settings, appearance: "light" },
          },
          schoolRecords: schoolRecords(),
        };
      });
    const mutation = store.mutate(appearance("dark"));
    await vi.waitFor(() => expect(service.mutate).toHaveBeenCalledOnce());
    const event = store.handleWorkspaceChanged({
      source: "user-intent",
      revision: "r1",
    });
    const newerMutation = store.mutate(appearance("light"));
    release({
      revision: "r1",
      contentRevision: "r1",
      workspace: {
        ...seedWorkspace(),
        settings: { ...seedWorkspace().settings, appearance: "dark" },
      },
      schoolRecords: schoolRecords(),
    });
    await Promise.all([mutation, event, newerMutation]);
    expect(service.snapshot).toHaveBeenCalledOnce();
    expect(store.getState()).toMatchObject({ revision: "r2" });
  });
  it("reloads a user-intent revision committed by another client", async () => {
    const service = coreService();
    const store = createWorkspaceStore(service);
    await store.initialize();
    service.snapshot.mockResolvedValueOnce({
      revision: "external-r1",
      contentRevision: "external-r1",
      workspace: seedWorkspace(),
      schoolRecords: schoolRecords(),
    });
    await store.handleWorkspaceChanged({
      source: "user-intent",
      revision: "external-r1",
    });
    expect(service.snapshot).toHaveBeenCalledTimes(2);
    expect(store.getState().revision).toBe("external-r1");
  });
  it("shares conflict recovery with an external event waiting on the failed write", async () => {
    const service = coreService();
    const store = createWorkspaceStore(service);
    await store.initialize();
    let reject!: (error: Error) => void;
    service.mutate.mockImplementationOnce(
      () => new Promise((_resolve, fail) => void (reject = fail)),
    );
    service.snapshot.mockResolvedValueOnce({
      revision: "external-r1",
      contentRevision: "external-r1",
      workspace: seedWorkspace(),
      schoolRecords: schoolRecords(),
    });
    const mutation = store.mutate(appearance("dark"));
    await vi.waitFor(() => expect(service.mutate).toHaveBeenCalledOnce());
    const event = store.handleWorkspaceChanged({
      source: "user-intent",
      revision: "external-r1",
    });
    reject(new Error("revision conflict"));
    await expect(mutation).rejects.toThrow("revision conflict");
    await event;
    expect(service.snapshot).toHaveBeenCalledTimes(2);
    expect(store.getState().revision).toBe("external-r1");
  });
  it("keeps non-user events visible even when the user revision is unchanged", async () => {
    const service = coreService();
    const store = createWorkspaceStore(service);
    await store.initialize();
    await store.handleWorkspaceChanged({ source: "mcp" });
    await store.handleWorkspaceChanged({ source: "school-pull" });
    expect(service.snapshot).toHaveBeenCalledTimes(3);
  });
  it("recognizes a delayed event after a newer local write", async () => {
    const service = coreService();
    const store = createWorkspaceStore(service);
    await store.initialize();
    await store.mutate(appearance("dark"));
    await store.mutate(appearance("light"));
    await store.handleWorkspaceChanged({
      source: "user-intent",
      revision: "r1",
    });
    expect(service.snapshot).toHaveBeenCalledOnce();
    expect(store.getState().revision).toBe("r2");
  });
  it("resolves successive form intents against the preceding committed snapshot", async () => {
    const service = coreService();
    const store = createWorkspaceStore(service);
    await store.initialize();
    const first = store.mutate((draft) => {
      draft.settings.appearance = "dark";
      return { kind: "replaceSettings", settings: draft.settings };
    });
    const second = store.mutate((draft) => {
      draft.settings.locale = "ja";
      return { kind: "replaceSettings", settings: draft.settings };
    });
    await Promise.all([first, second]);
    expect(store.getState().workspace?.settings).toMatchObject({
      appearance: "dark",
      locale: "ja",
    });
    expect(service.mutate.mock.calls[1][0]).toMatchObject({
      expectedRevision: "r1",
      intent: { settings: { appearance: "dark", locale: "ja" } },
    });
  });
  it("keeps the cached snapshot unchanged when Core rejects an intent", async () => {
    const service = coreService();
    const store = createWorkspaceStore(service);
    await store.initialize();
    const before = store.getState();
    service.mutate.mockRejectedValueOnce(new Error("revision conflict"));
    await expect(store.mutate(appearance("dark"))).rejects.toThrow(
      "revision conflict",
    );
    expect(store.getState().revision).toBe(before.revision);
    expect(store.getState().workspace).toEqual(before.workspace);
    expect(service.snapshot).toHaveBeenCalledTimes(2);
  });
  it("adopts an external write after conflict and lets the user retry explicitly", async () => {
    const workspace = seedWorkspace();
    let revision = "r0";
    let rejectOnce = true;
    const snapshot = (): CoreWorkspaceSnapshot => ({
      revision,
      contentRevision: revision,
      workspace: structuredClone(workspace),
      schoolRecords: schoolRecords(),
    });
    const service = {
      snapshot: vi.fn(async () => snapshot()),
      mutate: vi.fn(async (request: WorkspaceMutation) => {
        if (rejectOnce) {
          rejectOnce = false;
          workspace.settings.locale = "ja";
          revision = "external";
          throw new Error("workspace revision conflict");
        }
        expect(request.expectedRevision).toBe("external");
        if (request.intent.kind === "replaceSettings")
          workspace.settings = structuredClone(request.intent.settings);
        revision = "retry";
        return snapshot();
      }),
    };
    const store = createWorkspaceStore(service);
    await store.initialize();
    // A form intent resolves against the snapshot adopted after the conflict.
    const intent = (draft: Workspace): WorkspaceIntent => ({
      kind: "replaceSettings",
      settings: { ...draft.settings, appearance: "dark" },
    });
    await expect(store.mutate(intent)).rejects.toThrow("revision conflict");
    expect(store.getState()).toMatchObject({ revision: "external" });
    expect(store.getState().workspace?.settings).toMatchObject({
      locale: "ja",
      appearance: "system",
    });
    expect(service.mutate).toHaveBeenCalledTimes(1);

    await store.mutate(intent);
    expect(service.mutate).toHaveBeenCalledTimes(2);
    expect(store.getState()).toMatchObject({ revision: "retry" });
    expect(store.getState().workspace?.settings).toMatchObject({
      locale: "ja",
      appearance: "dark",
    });
  });
  it("reload cannot replace a mutation that completes while its read is pending", async () => {
    const service = coreService();
    const store = createWorkspaceStore(service);
    await store.initialize();
    let release!: (snapshot: CoreWorkspaceSnapshot) => void;
    service.snapshot.mockImplementationOnce(
      () => new Promise((resolve) => void (release = resolve)),
    );
    const reload = store.reload();
    await Promise.resolve();
    const stale = {
      revision: "r0",
      contentRevision: "r0",
      workspace: seedWorkspace(),
      schoolRecords: schoolRecords(),
    };
    await store.mutate(appearance("dark"));
    release(stale);
    await reload;
    expect(service.snapshot).toHaveBeenCalledTimes(3);
    expect(store.getState().workspace?.settings.appearance).toBe("dark");
  });
  it("reloads the unified snapshot after a school pull", async () => {
    const service = coreService();
    const school = { pull: vi.fn(async () => []) };
    const store = createWorkspaceStore(service, school);
    await store.initialize();
    await store.pullSchool("science-lms");
    expect(school.pull).toHaveBeenCalledWith("science-lms");
    expect(service.snapshot).toHaveBeenCalledTimes(2);
  });
});

describe("workspace action boundary", () => {
  it("keeps browser preview read-only without a parallel mutation engine", async () => {
    const stored = seedWorkspace();
    const repository = {
      load: vi.fn(async () => structuredClone(stored)),
      export: vi.fn(async () => ""),
      reset: vi.fn(async () => seedWorkspace()),
    };
    const store = createWorkspaceStore(createBrowserPreviewService(repository));
    await store.initialize();
    await expect(store.mutate(appearance("dark"))).rejects.toThrow(
      "native app",
    );
    expect(stored.settings.appearance).toBe("system");
    await expect(
      store.mutate({ kind: "markTimetableReviewed" }),
    ).rejects.toThrow("native app");
  });
});
