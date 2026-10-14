import { invoke } from "@tauri-apps/api/core";
import { seedWorkspace, type Workspace } from "../core/model";

export interface WorkspaceRepository {
  load(): Promise<Workspace>;
  export(): Promise<string>;
  import?(document: string): Promise<Workspace>;
  reset(): Promise<Workspace>;
}

/** Browser preview has no persistence or domain engine. */
export class LocalRepository implements WorkspaceRepository {
  async load() {
    return seedWorkspace();
  }

  async export(): Promise<string> {
    throw new Error("Workspace export needs the native app.");
  }

  async import(_document: string): Promise<Workspace> {
    throw new Error("Workspace import needs the native app.");
  }

  async reset(): Promise<Workspace> {
    throw new Error("Workspace reset needs the native app.");
  }
}

/** Desktop adapter. Rust owns validation, migration, persistence and backups. */
export class SqliteRepository implements WorkspaceRepository {
  async load() {
    return invoke<Workspace>("load_workspace");
  }

  async export() {
    return invoke<string>("export_data");
  }

  async import(document: string) {
    return invoke<Workspace>("import_data", { document });
  }

  async reset() {
    return invoke<Workspace>("reset_workspace");
  }
}

export const repository: WorkspaceRepository =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window
    ? new SqliteRepository()
    : new LocalRepository();
