import { invoke } from "@tauri-apps/api/core";

/** Rust is the ID authority in production. The fallback exists only for the
 * browser-only development preview, where no native backend is present. */
export async function newUserRecordId(): Promise<string> {
  if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window)
    return invoke<string>("new_user_record_id");
  return crypto.randomUUID();
}
