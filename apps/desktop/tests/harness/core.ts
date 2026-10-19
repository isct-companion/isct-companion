import { expect } from "vitest";
import type { WorkspaceIntent } from "@app/lib/workspace-intents";
import type { Workspace } from "@app/lib/model";
import type { SchoolRecords } from "@app/platform/desktop-bridge";
import { emptySchoolRecords } from "./fixtures";

/**
 * A scripted stand-in for the Tauri host.
 *
 * Tests install it before the first `getDesktopBridge()` call, then drive the
 * application exactly as the shipped code does: real bridge, real query caches,
 * real store. Nothing is `vi.mock`ed, so a suite asserts behaviour instead of
 * asserting that a mock was called.
 */

export type CommandCall = { command: string; args: Record<string, unknown> };

/**
 * A read model, or a function that derives it from the live fake store. The
 * union keeps object literals assignable while still contextually typing the
 * function form, so `(_query, core) => …` needs no annotation.
 */
export type QueryAnswer =
  | ((query: unknown, core: FakeCore) => unknown)
  | Record<string, unknown>
  | unknown[]
  | string
  | number
  | boolean
  | null;

export type QueryTable = Record<string, QueryAnswer>;

export type FakeCoreOptions = {
  workspace: Workspace;
  schoolRecords?: SchoolRecords;
  /** Starts at `revision`; `mutateWorkspace` advances it. */
  revision?: string;
  revisionAfterMutation?: string;
  capabilities?: Record<string, unknown>;
  credentialStatus?: unknown;
  /** Answer `schedule_query` by query kind. Unlisted kinds resolve to null. */
  queries?: QueryTable;
  /** Answer `finder_query`; the second argument is the live fake store. */
  finder?: (request: Record<string, unknown>, core: FakeCore) => unknown;
  /** Answer any other command; return `undefined` to fall through to null. */
  handle?: (
    command: string,
    args: Record<string, unknown>,
    core: FakeCore,
  ) => unknown;
  /** Apply an accepted mutation to the fake store. */
  apply?: (mutation: WorkspaceIntent, core: FakeCore) => void;
};

export type FakeCore = {
  /** Every command the application sent, in order. */
  calls: CommandCall[];
  /** The `intent` of every accepted `mutate_workspace`, in order. */
  intents: WorkspaceIntent[];
  /** Every `mutate_workspace` payload, including the expected revision. */
  mutations: { expectedRevision: string; intent: WorkspaceIntent }[];
  revision: string;
  workspace: Workspace;
  schoolRecords: SchoolRecords;
  /** Replaces the stored snapshot, as another client would. */
  publish(workspace: Workspace, revision?: string): void;
  /** Delivers a native event to every registered listener. */
  emit(event: string, payload: unknown): void;
  listenerCount(event: string): number;
  commands(name: string): CommandCall[];
  restore(): void;
};

type Listener = {
  id: number;
  event: string;
  callback: (payload: unknown) => void;
};

let nextCallbackId = 1;
let nextRevision = 1;

/**
 * A fresh snapshot revision for each install.
 *
 * The real query caches key their answers by snapshot revision and live in
 * module scope, so a suite that mounts more than once would otherwise let the
 * first test's scripted answers satisfy the second test's reads.
 */
export function freshRevision(): string {
  return `revision:${nextRevision++}`;
}

export function installFakeCore(options: FakeCoreOptions): FakeCore {
  const calls: CommandCall[] = [];
  const intents: WorkspaceIntent[] = [];
  const mutations: { expectedRevision: string; intent: WorkspaceIntent }[] = [];
  const listeners: Listener[] = [];
  const callbacks = new Map<number, (payload: unknown) => void>();

  const core: FakeCore = {
    calls,
    intents,
    mutations,
    revision: options.revision ?? freshRevision(),
    workspace: structuredClone(options.workspace),
    schoolRecords: structuredClone(
      options.schoolRecords ?? emptySchoolRecords(),
    ),
    publish(workspace, revision) {
      core.workspace = structuredClone(workspace);
      if (revision) core.revision = revision;
    },
    emit(event, payload) {
      // Tauri hands the listener its `Event` envelope, and the bridge adapter
      // destructures `{ payload }` from it. Delivering the bare payload would
      // silently drop every native event.
      for (const listener of [...listeners]) {
        if (listener.event === event) {
          listener.callback({ event, id: listener.id, payload });
        }
      }
    },
    listenerCount: (event) =>
      listeners.filter((listener) => listener.event === event).length,
    commands: (name) => calls.filter((call) => call.command === name),
    restore() {
      delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    },
  };

  const snapshot = () => ({
    revision: core.revision,
    contentRevision: core.revision,
    workspace: structuredClone(core.workspace),
    schoolRecords: structuredClone(core.schoolRecords),
  });

  const answerQuery = (query: unknown): unknown => {
    if (!query || typeof query !== "object") return null;
    const kind = (query as { kind?: string }).kind;
    if (!kind) return null;
    const entry = options.queries?.[kind];
    if (entry === undefined) return null;
    return typeof entry === "function"
      ? (entry as (value: unknown, core: FakeCore) => unknown)(query, core)
      : entry;
  };

  const invoke = async (
    command: string,
    args: Record<string, unknown> = {},
  ): Promise<unknown> => {
    calls.push({ command, args });
    const custom = options.handle?.(command, args, core);
    if (custom !== undefined) return custom;
    switch (command) {
      case "plugin:event|listen": {
        const id = nextCallbackId++;
        const handlerId = args.handler as number;
        const callback = callbacks.get(handlerId);
        if (callback) {
          listeners.push({ id, event: String(args.event), callback });
        }
        return id;
      }
      case "plugin:event|unlisten": {
        const index = listeners.findIndex(
          (listener) => listener.id === args.eventId,
        );
        if (index >= 0) listeners.splice(index, 1);
        return null;
      }
      case "plugin:event|emit":
      case "plugin:event|emit_to":
        core.emit(String(args.event), args.payload);
        return null;
      case "workspace_snapshot":
        return snapshot();
      case "mutate_workspace": {
        const mutation = args.mutation as {
          expectedRevision: string;
          intent: WorkspaceIntent;
        };
        expect(mutation.expectedRevision, "mutation revision").toBe(
          core.revision,
        );
        mutations.push(mutation);
        intents.push(mutation.intent);
        if (options.apply) {
          options.apply(mutation.intent, core);
        } else {
          core.revision = options.revisionAfterMutation ?? `${core.revision}+1`;
        }
        return snapshot();
      }
      case "schedule_query":
        return {
          revision: core.revision,
          values: (args.queries as unknown[]).map((query) => ({
            // These requests return a normalized value with a different tag,
            // matching WorkspaceQueryResult on the native side.
            kind:
              (
                {
                  validateAcademicProfile: "academicProfile",
                  academicTermForDate: "academicTerm",
                } as Record<string, string>
              )[(query as { kind: string }).kind] ??
              (query as { kind: string }).kind,
            value: answerQuery(query),
          })),
        };
      case "finder_query":
        return (
          options.finder?.(args.request as Record<string, unknown>, core) ??
          null
        );
      case "check_app_update":
      case "app_update_state":
        return {
          phase: "current",
          version: "1.0.0",
          signedUpdates: true,
          releaseUrl:
            "https://github.com/isct-companion/isct-companion/releases/latest",
        };
      case "runtime_capabilities":
        return options.capabilities ?? {};
      case "school_credential_status":
        return options.credentialStatus ?? emptyCredentialStatus();
      case "pull_state":
        return [];
      case "school_status":
        // Core folds pulls into this; a test drives it with `school-status`.
        return {
          headline: { kind: "idle" },
          sources: (["science-lms", "kyomu"] as const).map((site) => ({
            site,
            activity: { kind: "idle" },
            signIn: null,
            lastUpdated: null,
          })),
        };
      case "catalog_offerings":
      case "catalog_sync_offerings":
      case "catalog_sync_department":
        return [];
      case "catalog_query":
        return { items: [], total: 0, returned: 0, truncated: false };
      default:
        return null;
    }
  };

  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
    invoke,
    transformCallback(callback: (payload: unknown) => void) {
      const id = nextCallbackId++;
      callbacks.set(id, callback);
      return id;
    },
    unregisterCallback(id: number) {
      callbacks.delete(id);
    },
    convertFileSrc: (path: string) => `asset://localhost/${path}`,
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { label: "main" },
    },
  };
  // `@tauri-apps/api/core`'s `isTauri()` reads this, and production code such as
  // the OCR pipeline refuses to run without it.
  (globalThis as unknown as Record<string, unknown>).isTauri = true;
  (window as unknown as Record<string, unknown>).isTauri = true;

  return core;
}

export function emptyCredentialStatus() {
  const profile = {
    saved: false,
    username: "",
    hasOtp: false,
    hasMatrix: false,
  };
  return { science: profile, tokyoTech: { ...profile } };
}
