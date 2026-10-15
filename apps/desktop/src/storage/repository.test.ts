// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { seedWorkspace, type Workspace } from "../lib/model";

const native = vi.hoisted(() => ({
  calls: [] as Array<{ command: string; payload: unknown }>,
  workspace: undefined as Workspace | undefined,
  exported: "",
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (command: string, payload: unknown) => {
    native.calls.push({ command, payload });
    if (command === "load_workspace") return structuredClone(native.workspace);
    if (command === "export_data") return native.exported;
    if (command === "import_data") return structuredClone(native.workspace);
    if (command === "reset_workspace") return structuredClone(seedWorkspace());
    throw new Error(`Unexpected command ${command}`);
  }),
}));
import { LocalRepository, SqliteRepository } from "./repository";

beforeEach(() => {
  native.calls.length = 0;
  native.workspace = seedWorkspace();
  native.exported = '{"kind":"IsctCompanion.local-backup"}';
});

describe("LocalRepository browser preview", () => {
  it("loads a detached read-only sample", async () => {
    const repository = new LocalRepository();
    const first = await repository.load();
    first.settings.locale = "zh";
    expect((await repository.load()).settings.locale).not.toBe("zh");
  });

  it("does not duplicate Core backup and mutation behavior", async () => {
    const repository = new LocalRepository();
    await expect(repository.export()).rejects.toThrow("native app");
    await expect(repository.import("{}")).rejects.toThrow("native app");
    await expect(repository.reset()).rejects.toThrow("native app");
  });
});

describe("SqliteRepository desktop bridge", () => {
  it("forwards reads and backup operations without local validation", async () => {
    const repository = new SqliteRepository();
    expect((await repository.load()).schemaVersion).toBe(3);
    expect(await repository.export()).toBe(native.exported);
    await repository.import("unparsed bytes for Core");
    expect((await repository.reset()).schemaVersion).toBe(3);
    expect(native.calls).toEqual([
      { command: "load_workspace", payload: undefined },
      { command: "export_data", payload: undefined },
      {
        command: "import_data",
        payload: { document: "unparsed bytes for Core" },
      },
      { command: "reset_workspace", payload: undefined },
    ]);
  });
});
