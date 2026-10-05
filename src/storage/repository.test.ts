// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { seedWorkspace, type Workspace } from "../core/model";

const native = vi.hoisted(() => ({
  calls: [] as Array<{ command: string; payload: unknown }>,
  workspace: undefined as Workspace | undefined,
  exported: "",
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (command: string, payload: unknown) => {
    native.calls.push({ command, payload });
    if (command === "load_workspace") return structuredClone(native.workspace);
    if (command === "save_workspace") {
      native.workspace = structuredClone(
        (payload as { workspace: Workspace }).workspace,
      );
      return;
    }
    if (command === "export_data") return native.exported;
    if (command === "import_data") return structuredClone(native.workspace);
    if (command === "reset_workspace") return structuredClone(seedWorkspace());
    throw new Error(`Unexpected command ${command}`);
  }),
}));

import {
  LocalRepository,
  SqliteRepository,
  UnsupportedWorkspaceSchemaError,
  type ExportDocument,
} from "./repository";

const STORAGE_KEY = "stws-workspace-v3";

beforeEach(() => {
  localStorage.clear();
  native.calls.length = 0;
  native.workspace = seedWorkspace();
  native.exported = JSON.stringify({
    schemaVersion: 3,
    exportedAt: "2026-09-29T00:00:00.000Z",
    deviceId: native.workspace.deviceId,
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
    expect(exported.schemaVersion).toBe(3);
    expect(exported.deviceId).toBe(value.deviceId);
    expect(exported.tables.settings).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          key: "settings",
          value: expect.objectContaining({ locale: "zh" }),
        }),
      ]),
    );
    expect(exported.tables).not.toHaveProperty("offering_content");
  });

  it("imports the same document format read-only previews export", async () => {
    const repository = new LocalRepository();
    const workspace = seedWorkspace();
    workspace.settings.appearance = "dark";
    await repository.save(workspace);
    const document = await repository.export();
    await repository.reset();
    const imported = await repository.import(document);
    expect(imported.settings.appearance).toBe("dark");
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
    await repository.save(value);
    expect(native.calls.map((call) => call.command)).toEqual([
      "load_workspace",
      "save_workspace",
    ]);
    expect(native.workspace?.settings.locale).toBe("ja");
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
});
