import {
  invoke as tauriInvoke,
  type InvokeArgs,
  type InvokeOptions,
} from "@tauri-apps/api/core";
import type { ErrorCode } from "../generated/persistence";
import { t } from "../i18n";

/** A native command's rejection. Core rejects with a stable code, shown as
 * the shared catalogs' `error.<code>` copy; its `detail` is for logs only. */
export class NativeError extends Error {
  constructor(
    readonly code: ErrorCode,
    readonly detail?: string,
    params?: Record<string, string>,
  ) {
    super(t(`error.${code}`, params));
    this.name = "NativeError";
  }
}

/** Normalizes what a native command rejects with: a Core failure
 * (`{ code, params?, detail }`), the desktop shell's own copy
 * (`{ message }`), or a plain string from older paths and test hosts. */
export function nativeError(error: unknown, fallback: string): Error {
  if (error instanceof Error) return error;
  if (typeof error === "string") return new Error(error);
  if (error && typeof error === "object") {
    if ("code" in error && typeof error.code === "string") {
      const params =
        "params" in error && error.params && typeof error.params === "object"
          ? (error.params as Record<string, string>)
          : undefined;
      const detail =
        "detail" in error && typeof error.detail === "string"
          ? error.detail
          : undefined;
      return new NativeError(error.code as ErrorCode, detail, params);
    }
    if ("message" in error && typeof error.message === "string")
      return new Error(error.message);
  }
  return new Error(fallback);
}

/** The Core code of a rejection, to branch on instead of its text. */
export const errorCode = (error: unknown): ErrorCode | undefined =>
  error instanceof NativeError ? error.code : undefined;

/** `invoke` whose rejections are `nativeError`s. */
export async function invoke<T>(
  command: string,
  args?: InvokeArgs,
  options?: InvokeOptions,
): Promise<T> {
  try {
    return await tauriInvoke<T>(command, args, options);
  } catch (error) {
    throw nativeError(error, `${command} failed`);
  }
}
