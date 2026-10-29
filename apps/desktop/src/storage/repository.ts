import { t } from "../i18n";
import { invoke } from "../platform/native-error";

/** Backups. Pages read their records through Core's queries, not here. */
export interface WorkspaceRepository {
  export(): Promise<string>;
  /** Restores a backup; its commit tells the pages what to query again. */
  import?(document: string): Promise<void>;
}

/** Browser preview has no persistence or domain engine. */
export class LocalRepository implements WorkspaceRepository {
  async export(): Promise<string> {
    throw new Error(t("repository.workspaceExportNeedsTheNative"));
  }

  async import(_document: string): Promise<void> {
    throw new Error(t("repository.workspaceImportNeedsTheNative"));
  }
}

/** Desktop adapter. Rust owns validation, migration, persistence and backups. */
export class SqliteRepository implements WorkspaceRepository {
  async export() {
    return invoke<string>("export_data");
  }

  async import(document: string) {
    await invoke<void>("import_data", { document });
  }
}

export const repository: WorkspaceRepository =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window
    ? new SqliteRepository()
    : new LocalRepository();
