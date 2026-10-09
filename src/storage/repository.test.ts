// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { seedWorkspace, type Workspace } from "../core/model";

type NativeRow = Record<string, unknown> & { id: string; rev: number };

const native = vi.hoisted(() => ({
  calls: [] as Array<{ command: string; payload: unknown }>,
  workspace: undefined as Workspace | undefined,
  exported: "",
  /** When set, user-record commands follow the Rust revision rules. */
  tables: undefined as Map<string, Map<string, NativeRow>> | undefined,
  clock: 0,
}));

// Mirrors DataStore::put_user_record_checked / delete_user_record_checked.
function nativeContent(row: Record<string, unknown> | undefined) {
  if (!row) return undefined;
  const { rev: _r, updatedAt: _a, updatedBy: _b, deletedAt: _d, ...rest } = row;
  return JSON.stringify(rest);
}
function nativePut(payload: unknown) {
  const { table, record, expectedRev } = payload as {
    table: string;
    record: NativeRow;
    expectedRev?: number;
  };
  const rows = native.tables!.get(table) ?? new Map<string, NativeRow>();
  native.tables!.set(table, rows);
  const stored = rows.get(record.id);
  const actual = stored?.rev ?? 0;
  if (expectedRev !== undefined && expectedRev !== actual)
    throw new Error(
      `revision conflict: ${table}/${record.id} expected ${expectedRev}, actual ${actual}`,
    );
  if (
    stored &&
    !stored.deletedAt &&
    nativeContent(stored) === nativeContent(record)
  )
    return structuredClone(stored);
  const { deletedAt: _deleted, ...live } = record;
  const next = {
    ...live,
    rev: actual + 1,
    updatedAt: new Date(Date.UTC(2026, 9, 9, 0, 0, ++native.clock)).toISOString(),
    updatedBy: "desktop",
  };
  rows.set(record.id, next);
  return structuredClone(next);
}
function nativeDelete(payload: unknown) {
  const { table, id, expectedRev } = payload as {
    table: string;
    id: string;
    expectedRev?: number;
  };
  const stored = native.tables!.get(table)?.get(id);
  const actual = stored?.rev ?? 0;
  if (expectedRev !== undefined && expectedRev !== actual)
    throw new Error(
      `revision conflict: ${table}/${id} expected ${expectedRev}, actual ${actual}`,
    );
  if (!stored) return false;
  Object.assign(stored, {
    rev: actual + 1,
    updatedAt: new Date(Date.UTC(2026, 9, 9, 0, 0, ++native.clock)).toISOString(),
    deletedAt: "2026-10-09T00:00:00.000Z",
  });
  return true;
}

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (command: string, payload: unknown) => {
    native.calls.push({ command, payload });
    if (command === "load_workspace") return structuredClone(native.workspace);
    if (command === "put_workspace_setting") {
      const { key, value } = payload as { key: string; value: unknown };
      if (key === "devicePreferences" && native.workspace)
        Object.assign(native.workspace.settings, value);
      return;
    }
    if (native.tables && command === "put_user_record") return nativePut(payload);
    if (native.tables && command === "delete_user_record") return nativeDelete(payload);
    if (command === "put_user_record" || command === "delete_user_record") return;
    if (command === "export_data") return native.exported;
    if (command === "import_data") return structuredClone(native.workspace);
    if (command === "reset_workspace") return structuredClone(seedWorkspace());
    throw new Error(`Unexpected command ${command}`);
  }),
}));

import { createWorkspaceStore } from "../store/workspace";
import {
  LMS_BACKGROUND_REFRESH_TASK_ID,
  lmsBackgroundRefreshTask,
  referenceRefreshTask,
} from "../core/model";
import {
  LocalRepository,
  SqliteRepository,
  UnsupportedWorkspaceSchemaError,
  type ExportDocument,
} from "./repository";

const STORAGE_KEY = "isct-companion-workspace-v3";

beforeEach(() => {
  localStorage.clear();
  native.calls.length = 0;
  native.tables = undefined;
  native.clock = 0;
  native.workspace = seedWorkspace();
  native.exported = JSON.stringify({
    schemaVersion: 6,
    exportedAt: "2026-09-29T00:00:00.000Z",
    deviceId: "local-backup",
    kind: "IsctCompanion.local-backup",
    localNotes: [],
    tables: {},
  });
});

describe("LocalRepository browser preview", () => {
  it("seeds and persists a real empty v3 preview", async () => {
    const repository = new LocalRepository();
    const value = await repository.load();
    expect(value.schemaVersion).toBe(3);
    expect(value.courses).toEqual([]);
    expect(value.assignments).toEqual([]);
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null")).toEqual(
      value,
    );
  });

  it("round-trips workspace data and exports the shared document format", async () => {
    const repository = new LocalRepository();
    const value = seedWorkspace();
    value.settings.locale = "zh";
    await repository.save(value);
    expect((await repository.load()).settings.locale).toBe("zh");
    const exported = JSON.parse(
      await repository.export(),
    ) as ExportDocument;
    expect(exported.schemaVersion).toBe(6);
    expect(exported.deviceId).toBe("local-backup");
    expect(exported.tables.settings).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          key: "userPreferences",
        }),
      ]),
    );
    expect(exported.tables).not.toHaveProperty("offering_content");
  });

  it("imports the same document format read-only previews export", async () => {
    const repository = new LocalRepository();
    const workspace = seedWorkspace();
    workspace.settings.notifications.classes = false;
    await repository.save(workspace);
    const document = await repository.export();
    await repository.reset();
    const imported = await repository.import(document);
    expect(imported.settings.notifications.classes).toBe(false);
  });

  it("normalizes optional v3 fields", async () => {
    const value = seedWorkspace();
    value.terms = undefined;
    value.settings.customization = undefined;
    value.settings.cloud.redirectUri = undefined;
    localStorage.setItem(STORAGE_KEY, JSON.stringify(value));
    const loaded = await new LocalRepository().load();
    expect(loaded.terms).toEqual([]);
    expect(loaded.settings.customization?.sites.portal.enabled).toBe(false);
    expect(loaded.settings.cloud.redirectUri).toContain("127.0.0.1");
  });

  it("preserves unsupported and corrupt bytes", async () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ schemaVersion: 2 }));
    await expect(new LocalRepository().load()).rejects.toBeInstanceOf(
      UnsupportedWorkspaceSchemaError,
    );
    expect(localStorage.getItem(STORAGE_KEY)).toBe('{"schemaVersion":2}');

    localStorage.setItem(STORAGE_KEY, "not-json");
    await expect(new LocalRepository().load()).rejects.toThrow("corrupt");
    expect(localStorage.getItem(STORAGE_KEY)).toBe("not-json");
  });
});

describe("SqliteRepository desktop bridge", () => {
  it("loads and saves only through narrow Rust commands", async () => {
    const repository = new SqliteRepository();
    const value = await repository.load();
    value.settings.locale = "ja";
    value.settings.contentLanguage = "en";
    await repository.save(value);
    expect(native.calls.map((call) => call.command)).toEqual([
      "load_workspace",
      "put_workspace_setting",
    ]);
    expect(native.workspace?.settings.locale).toBe("ja");
    expect(native.workspace?.settings.contentLanguage).toBe("en");
  });

  it("delegates export, import and reset to Rust", async () => {
    const repository = new SqliteRepository();
    expect(await repository.export()).toBe(native.exported);
    await repository.import(native.exported);
    const reset = await repository.reset();
    expect(reset.schemaVersion).toBe(3);
    expect(native.calls.map((call) => call.command)).toEqual([
      "export_data",
      "import_data",
      "reset_workspace",
    ]);
  });

  it("keeps saving after record edits whose content later stops changing", async () => {
    const tasks = [
      lmsBackgroundRefreshTask("device"),
      referenceRefreshTask("device"),
    ];
    native.workspace!.scheduledTasks = structuredClone(tasks);
    native.tables = new Map([
      ["scheduled_tasks", new Map(tasks.map((task) => [task.id, { ...task }]))],
    ]);
    const store = createWorkspaceStore(new SqliteRepository());
    await store.initialize();
    const settle = () => store.flush();
    const toggle = (enabled: boolean) =>
      store.update((draft) => {
        const task = draft.scheduledTasks.find(
          (item) => item.id === LMS_BACKGROUND_REFRESH_TASK_ID,
        )!;
        task.enabled = enabled;
        task.rev += 1;
        task.updatedAt = new Date().toISOString();
      });

    toggle(true);
    await settle();
    // Unrelated saves after the record edit, the way Appearance did.
    for (const appearance of ["dark", "light", "system", "dark"] as const) {
      store.update((draft) => {
        draft.settings.appearance = appearance;
      });
      await settle();
    }
    // A local-only rev bump with unchanged content (subscription polling did this).
    store.update((draft) => {
      draft.scheduledTasks[0]!.rev += 5;
    });
    await settle();
    toggle(false);
    await settle();
    toggle(true);
    await settle();

    expect(store.getState().error).toBeUndefined();
    expect(store.getState().persistedRevision).toBe(store.getState().revision);
    const stored = native.tables
      .get("scheduled_tasks")!
      .get(LMS_BACKGROUND_REFRESH_TASK_ID)!;
    expect(stored).toMatchObject({ enabled: true, rev: 4 });
    const puts = native.calls.filter((call) => call.command === "put_user_record");
    expect(puts.map((call) => (call.payload as { expectedRev: number }).expectedRev))
      .toEqual([1, 2, 3]);
  });
});
