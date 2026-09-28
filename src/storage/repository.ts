import { seedWorkspace, type Workspace } from "../core/model";
import { LazyStore } from "@tauri-apps/plugin-store";
const KEY = "stws-workspace-v2";
const NATIVE_STORE_KEY = "workspace";
export interface WorkspaceRepository {
  load(): Promise<Workspace>;
  save(data: Workspace): Promise<void>;
  export(): Promise<string>;
  reset(): Promise<Workspace>;
}
export class UnsupportedWorkspaceSchemaError extends Error {
  override name = "UnsupportedWorkspaceSchemaError";
}
function normalize(data: Workspace): Workspace {
  data.terms ??= [];
  data.academicProfile ??= seedWorkspace().academicProfile;
  if (
    !["keep-current", "use-latest", "resolve-manually"].includes(
      data.settings.cloud.conflict,
    )
  )
    data.settings.cloud.conflict = "resolve-manually";
  data.settings.customization ??= {
    background: "",
    appCss: "",
    sites: {
      portal: { css: "", js: "", enabled: false },
      lms: { css: "", js: "", enabled: false },
      syllabus: { css: "", js: "", enabled: false },
    },
    hooks: [],
  };
  data.settings.cloud.redirectUri ??= "http://127.0.0.1:42813/oauth/callback";
  return data;
}
export class LocalRepository implements WorkspaceRepository {
  async load() {
    const raw = localStorage.getItem(KEY);
    if (raw) {
      try {
        const value = JSON.parse(raw);
        if (value.schemaVersion === 2) return normalize(value as Workspace);
        throw new UnsupportedWorkspaceSchemaError(
          `Workspace schema ${value?.schemaVersion ?? "unknown"} is unsupported. Stored data was preserved.`,
        );
      } catch (error) {
        if (error instanceof UnsupportedWorkspaceSchemaError) throw error;
        throw new UnsupportedWorkspaceSchemaError(
          "Workspace data is corrupt. Stored bytes were preserved.",
        );
      }
    }
    const data = seedWorkspace();
    await this.save(data);
    return data;
  }
  async save(data: Workspace) {
    localStorage.setItem(KEY, JSON.stringify(data));
  }
  async export() {
    return JSON.stringify(await this.load(), null, 2);
  }
  async reset() {
    const data = seedWorkspace();
    await this.save(data);
    return data;
  }
}
/** Desktop source of truth. The same store is addressable from Rust services. */
export class TauriStoreRepository implements WorkspaceRepository {
  private store = new LazyStore("workspace.json", { autoSave: false });
  async load() {
    const value = await this.store.get<Workspace>(NATIVE_STORE_KEY);
    if (value?.schemaVersion === 2) return normalize(value);
    if (value)
      throw new UnsupportedWorkspaceSchemaError(
        `Workspace schema ${(value as { schemaVersion?: unknown }).schemaVersion ?? "unknown"} is unsupported. Stored data was preserved.`,
      );
    // One-time migration from the former browser-owned local store.
    const legacy = localStorage.getItem(KEY);
    const workspace = legacy
      ? await new LocalRepository().load()
      : seedWorkspace();
    await this.save(workspace);
    return workspace;
  }
  async save(data: Workspace) {
    await this.store.set(NATIVE_STORE_KEY, data);
    await this.store.save();
  }
  async export() {
    return JSON.stringify(await this.load(), null, 2);
  }
  async reset() {
    const data = seedWorkspace();
    await this.save(data);
    return data;
  }
}

// LocalStorage exists only to keep the ordinary browser development preview usable.
export const repository: WorkspaceRepository =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window
    ? new TauriStoreRepository()
    : new LocalRepository();
