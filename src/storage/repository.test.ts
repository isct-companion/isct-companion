// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { seedWorkspace, type Workspace } from "../core/model";

const nativeStore = vi.hoisted(() => ({
  values: new Map<string, unknown>(),
  constructors: [] as Array<{ path: string; options: unknown }>,
  saves: 0,
}));

vi.mock("@tauri-apps/plugin-store", () => ({
  LazyStore: class {
    constructor(path: string, options: unknown) {
      nativeStore.constructors.push({ path, options });
    }
    async get<T>(key: string) {
      return nativeStore.values.get(key) as T | undefined;
    }
    async set(key: string, value: unknown) {
      nativeStore.values.set(key, structuredClone(value));
    }
    async save() {
      nativeStore.saves += 1;
    }
  },
}));

import {
  LocalRepository,
  TauriStoreRepository,
  UnsupportedWorkspaceSchemaError,
} from "./repository";

const LEGACY_KEY = "stws-workspace-v2";

beforeEach(() => {
  localStorage.clear();
  nativeStore.values.clear();
  nativeStore.constructors.length = 0;
  nativeStore.saves = 0;
});

describe("LocalRepository browser preview", () => {
  it("seeds and persists an empty preview", async () => {
    const repository = new LocalRepository();
    const value = await repository.load();
    expect(value.schemaVersion).toBe(2);
    expect(JSON.parse(localStorage.getItem(LEGACY_KEY) ?? "null")).toEqual(
      value,
    );
  });

  it("round-trips workspace data and exports formatted JSON", async () => {
    const repository = new LocalRepository();
    const value = seedWorkspace();
    value.settings.locale = "zh";
    await repository.save(value);
    const loaded = await repository.load();
    expect(loaded.settings.locale).toBe("zh");
    expect(JSON.parse(await repository.export())).toEqual(loaded);
  });

  it("normalizes fields introduced by later versions of schema 2", async () => {
    const legacy = seedWorkspace() as Workspace;
    legacy.terms = undefined;
    legacy.settings.customization = undefined;
    legacy.settings.cloud.redirectUri = undefined;
    localStorage.setItem(LEGACY_KEY, JSON.stringify(legacy));
    const loaded = await new LocalRepository().load();
    expect(loaded.terms).toEqual([]);
    expect(loaded.settings.customization?.sites.portal.enabled).toBe(false);
    expect(loaded.settings.cloud.redirectUri).toContain("127.0.0.1");
  });

  it("preserves unsupported and corrupt bytes", async () => {
    localStorage.setItem(LEGACY_KEY, JSON.stringify({ schemaVersion: 1 }));
    await expect(new LocalRepository().load()).rejects.toBeInstanceOf(
      UnsupportedWorkspaceSchemaError,
    );
    expect(localStorage.getItem(LEGACY_KEY)).toBe('{"schemaVersion":1}');

    localStorage.setItem(LEGACY_KEY, "not-json");
    await expect(new LocalRepository().load()).rejects.toThrow("corrupt");
    expect(localStorage.getItem(LEGACY_KEY)).toBe("not-json");
  });

  it("reset replaces the preview with a fresh workspace", async () => {
    const repository = new LocalRepository();
    const changed = seedWorkspace();
    changed.settings.appearance = "dark";
    await repository.save(changed);
    const reset = await repository.reset();
    expect(reset.settings.appearance).toBe("system");
    expect(await repository.load()).toEqual({ ...reset, terms: [] });
  });
});

describe("Tauri Store repository", () => {
  it("uses one explicit workspace store and durable save", async () => {
    const repository = new TauriStoreRepository();
    const value = seedWorkspace();
    value.settings.locale = "ja";
    await repository.save(value);
    expect(nativeStore.constructors).toEqual([
      { path: "workspace.json", options: { autoSave: false } },
    ]);
    expect(nativeStore.values.get("workspace")).toEqual(value);
    expect(nativeStore.saves).toBe(1);
    expect((await repository.load()).settings.locale).toBe("ja");
  });

  it("migrates the former local workspace once", async () => {
    const legacy = seedWorkspace();
    legacy.settings.appearance = "dark";
    localStorage.setItem(LEGACY_KEY, JSON.stringify(legacy));
    const repository = new TauriStoreRepository();
    const migrated = await repository.load();
    expect(migrated.settings.appearance).toBe("dark");
    expect(nativeStore.values.get("workspace")).toEqual(migrated);
    expect(nativeStore.saves).toBe(1);
  });

  it("does not replace a stored unsupported native schema", async () => {
    nativeStore.values.set("workspace", { schemaVersion: 9 });
    const repository = new TauriStoreRepository();
    await expect(repository.load()).rejects.toThrow("schema 9");
    expect(nativeStore.saves).toBe(0);
    expect(nativeStore.values.get("workspace")).toEqual({ schemaVersion: 9 });
  });
});
