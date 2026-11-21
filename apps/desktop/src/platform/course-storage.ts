import { convertFileSrc } from "@tauri-apps/api/core";
import { invoke } from "./native-error";

export type StorageInfo = { root: string; custom: boolean; canChoose: boolean };
export const storageInfo = (): Promise<StorageInfo> =>
  invoke("course_storage_info");
export async function chooseStorage(reset = false): Promise<StorageInfo> {
  return invoke("choose_course_storage", { reset });
}

export const localFileUrl = (path: string) => convertFileSrc(path);

/** Remove downloaded files not opened for `days` days. */
export const cleanMaterials = (days: number): Promise<number> =>
  invoke("clean_course_materials", { days });

/** Remove workspaces unchanged for a month; returns how many. */
export const cleanWorkspaces = () => invoke<number>("clean_workspaces");
