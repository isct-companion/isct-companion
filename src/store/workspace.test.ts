import { describe, expect, it, vi } from "vitest";
import { seedWorkspace, type Workspace } from "../core/model";
import { createWorkspaceHooks, createWorkspaceStore } from "./workspace";

const clone = <T>(value: T): T => structuredClone(value);

function memoryRepository(initial: Workspace = seedWorkspace()) {
  let stored = clone(initial);
  const saves: Workspace[] = [];
  return {
    saves,
    load: vi.fn(async () => clone(stored)),
    save: vi.fn(async (next: Workspace) => {
      stored = clone(next);
      saves.push(clone(next));
    }),
    read: () => clone(stored),
  };
}

describe("Zustand workspace store", () => {
  it("loads one immutable snapshot and deduplicates concurrent initialization", async () => {
    const storage = memoryRepository();
    const store = createWorkspaceStore(storage);
    const first = store.initialize();
    const second = store.initialize();

    await Promise.all([first, second]);

    expect(storage.load).toHaveBeenCalledTimes(1);
    expect(store.getState()).toMatchObject({
      status: "ready",
      revision: 0,
      persistedRevision: 0,
    });
    expect(Object.isFrozen(store.getState().workspace)).toBe(true);
    expect(Object.isFrozen(store.getState().workspace?.settings)).toBe(true);
  });

  it("publishes through Zustand subscriptions and persists an edit", async () => {
    const storage = memoryRepository();
    const store = createWorkspaceStore(storage);
    await store.initialize();
    const listener = vi.fn();
    const unsubscribe = store.subscribe(listener);

    store.update((draft) => {
      draft.settings.appearance = "dark";
    });
    await store.flush();

    expect(listener).toHaveBeenCalled();
    expect(store.getState().workspace?.settings.appearance).toBe("dark");
    expect(store.getState()).toMatchObject({
      revision: 1,
      persistedRevision: 1,
    });
    expect(storage.read().settings.appearance).toBe("dark");
    unsubscribe();
  });

  it("does not create a revision for a no-op transaction", async () => {
    const storage = memoryRepository();
    const store = createWorkspaceStore(storage);
    await store.initialize();
    store.update(() => undefined);
    await store.flush();
    expect(store.getState().revision).toBe(0);
    expect(storage.save).not.toHaveBeenCalled();
  });

  it("serializes rapid saves in revision order", async () => {
    const initial = seedWorkspace();
    const resolvers: Array<() => void> = [];
    const written: string[] = [];
    const storage = {
      load: vi.fn(async () => initial),
      save: vi.fn(
        (data: Workspace) =>
          new Promise<void>((resolve) => {
            written.push(data.settings.appearance);
            resolvers.push(resolve);
          }),
      ),
    };
    const store = createWorkspaceStore(storage);
    await store.initialize();
    store.update((draft) => void (draft.settings.appearance = "dark"));
    store.update((draft) => void (draft.settings.appearance = "light"));

    await vi.waitFor(() => expect(written).toEqual(["dark"]));
    resolvers.shift()?.();
    await vi.waitFor(() => expect(written).toEqual(["dark", "light"]));
    resolvers.shift()?.();
    await store.flush();
    expect(store.getState().persistedRevision).toBe(2);
  });

  it("keeps valid in-memory state when persistence fails and can retry", async () => {
    const storage = memoryRepository();
    storage.save.mockRejectedValueOnce(new Error("disk full"));
    const store = createWorkspaceStore(storage);
    await store.initialize();
    store.update((draft) => void (draft.settings.locale = "ja"));
    await expect(store.flush()).rejects.toThrow("remain in memory");
    expect(store.getState().workspace?.settings.locale).toBe("ja");

    await store.retrySave();
    expect(store.getState().error).toBeUndefined();
    expect(store.getState().persistedRevision).toBe(1);
  });

  it("rejects duplicate records and non-serializable values before publish", async () => {
    const storage = memoryRepository();
    const store = createWorkspaceStore(storage);
    await store.initialize();
    const original = store.getState().workspace;

    expect(() =>
      store.update((draft) => {
        draft.courses.push(clone(draft.courses[0]));
      }),
    ).toThrow("duplicate courses");
    expect(() =>
      store.update((draft) => {
        (draft.settings as unknown as { invalid: bigint }).invalid = 1n;
      }),
    ).toThrow("non-serializable");
    expect(store.getState().workspace).toBe(original);
  });

  it("reports load failure without overwriting storage", async () => {
    const storage = {
      load: vi.fn(async () => {
        throw new Error("corrupt");
      }),
      save: vi.fn(),
    };
    const store = createWorkspaceStore(storage);
    await store.initialize();
    expect(store.getState()).toMatchObject({ status: "error" });
    expect(store.getState().workspace).toBeUndefined();
    expect(storage.save).not.toHaveBeenCalled();
  });

  it("exports a detached, formatted JSON snapshot", async () => {
    const store = createWorkspaceStore(memoryRepository());
    await store.initialize();
    const exported = store.export();
    expect(JSON.parse(exported)).toEqual(store.getState().workspace);
    expect(exported).toContain('\n  "schemaVersion"');
  });
});

describe("workspace action boundary", () => {
  it("allows local preferences in browser preview", async () => {
    const store = createWorkspaceStore(memoryRepository());
    await store.initialize();
    const hooks = createWorkspaceHooks(store, () => "browser-preview");
    hooks.actions.updatePreferences({ appearance: "dark", locale: "zh" });
    await store.flush();
    expect(store.getState().workspace?.settings).toMatchObject({
      appearance: "dark",
      locale: "zh",
    });
  });

  it("rejects invalid preference keys and values", async () => {
    const store = createWorkspaceStore(memoryRepository());
    await store.initialize();
    const hooks = createWorkspaceHooks(store, () => "native-desktop");
    expect(() =>
      hooks.actions.updatePreferences({ appearance: "neon" as "dark" }),
    ).toThrow("Invalid preference");
    expect(() =>
      hooks.actions.updatePreferences({ density: "compact" } as never),
    ).toThrow("Invalid preference");
  });

  it("keeps academic edits inside their permitted envelope", async () => {
    const store = createWorkspaceStore(memoryRepository());
    await store.initialize();
    const hooks = createWorkspaceHooks(store, () => "native-desktop");
    hooks.actions.editAcademic((draft) => {
      draft.courses[0].title = "Updated title";
    });
    expect(store.getState().workspace?.courses[0].title).toBe("Updated title");
    expect(() =>
      hooks.actions.editAcademic((draft) => {
        draft.settings.locale = "ja";
      }),
    ).toThrow("settings");
    expect(() =>
      hooks.actions.editAcademic((draft) => {
        draft.deviceId = "foreign-device";
      }),
    ).toThrow("workspace identity");
  });

  it("blocks academic mutation in browser preview", async () => {
    const store = createWorkspaceStore(memoryRepository());
    await store.initialize();
    const hooks = createWorkspaceHooks(store, () => "browser-preview");
    expect(() => hooks.actions.editAcademic(() => undefined)).toThrow(
      "read-only",
    );
  });
});
