import { nativeError } from "../native-error";

/** A file saved to Downloads by any desktop source. */
export type SavedDownload = { name: string; path: string };

export type InvokeNative = (
  command: string,
  payload?: Record<string, unknown>,
) => Promise<unknown>;
export type ListenNative = <T>(
  event: string,
  callback: (payload: T) => void,
) => Promise<() => void>;

/** Listens to a native event, decoding payloads for the caller. */
export function subscribe<T>(
  listen: ListenNative,
  event: string,
  decode: (payload: T) => T | undefined,
  callback: (payload: T) => void,
): () => void {
  let disposed = false;
  let unlisten: (() => void) | undefined;
  void listen<T>(event, (payload) => {
    if (disposed) return;
    const decoded = decode(payload);
    if (decoded !== undefined) callback(decoded);
  })
    .then((fn) => {
      if (disposed) quietly(fn);
      else unlisten = fn;
    })
    .catch(() => undefined);
  return () => {
    disposed = true;
    if (unlisten) quietly(unlisten);
  };
}

/** Tauri's unlisten throws, or rejects, once the runtime is gone (window closing, tests). */
export const quietly = (unlisten: () => unknown) => {
  try {
    void Promise.resolve(unlisten()).catch(() => undefined);
  } catch {
    // Already gone.
  }
};
const requireBoolean = (command: string, value: unknown) => {
  if (typeof value !== "boolean")
    throw new Error(`${command} returned an invalid result.`);
  return value;
};
export const requireString = (command: string, value: unknown) => {
  if (typeof value !== "string")
    throw new Error(`${command} returned an invalid result.`);
  return value;
};
const requireObject = <T>(command: string, value: unknown): T => {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error(`${command} returned an invalid result.`);
  return value as T;
};

export function createBridgeCommands(invoke: InvokeNative) {
  const invokeCommand = async <T>(
    command: string,
    payload?: Record<string, unknown>,
  ): Promise<T> => {
    try {
      return (await invoke(command, payload)) as T;
    } catch (error) {
      throw nativeError(error, `${command} failed`);
    }
  };
  const result = async <T>(
    command: string,
    payload?: Record<string, unknown>,
  ) => requireObject<T>(command, await invokeCommand(command, payload));
  const booleanCommand = async (
    command: string,
    payload?: Record<string, unknown>,
  ) => requireBoolean(command, await invokeCommand(command, payload));
  return { invokeCommand, result, booleanCommand };
}

export type BridgeCommands = ReturnType<typeof createBridgeCommands>;
