import { useCallback, useEffect, useRef, useState } from "react";
import type {
  Factor,
  FinderContext,
  FinderCourse,
  FinderRow,
  RemotePlan,
  Suggestion,
} from "../lib/course-query";
import { useWorkspace } from "../store/useWorkspace";
import { getDesktopBridge } from "./desktop-bridge";
import type { QueryState } from "./core-query";

export type FinderAction =
  | { kind: "commit"; key: string }
  | { kind: "toggleTime"; key: string; append: boolean }
  | { kind: "paste"; text: string };
export type FinderRequest = {
  year: number;
  term: string;
  keys: string[];
  input?: string;
  remoteIds?: string[];
  action?: FinderAction;
};
export type FinderView = {
  context: Omit<FinderContext, "recommended"> & {
    recommended: Record<
      string,
      { group: string; mark?: "required" | "recommended" }
    >;
  };
  courses: FinderCourse[];
  factors: Factor[];
  duplicates: number;
  keys: string[];
  plan: RemotePlan;
  rows: FinderRow[];
  suggestions: Suggestion[];
  live: string;
  clashes: string[];
  replacements: Record<string, string[]>;
  credits: Record<string, number>;
  onTimetable: string[];
  activeSlots: string[];
  commitKey?: string | null;
  pasteHandled: boolean;
  pastedIds: string[];
  missingCodes: string[];
  detailIds: string[];
};

export function finderQuery(request: FinderRequest): Promise<FinderView> {
  const bridge = getDesktopBridge();
  if (!bridge)
    return Promise.reject(new Error("Course search requires the native Core."));
  return bridge.finderQuery(request);
}

const pendingFinderQueries = new Map<string, Promise<FinderView>>();

export type FinderEditQueue = {
  run(action: FinderAction): Promise<FinderView | undefined>;
  replace(keys: string[] | ((current: string[]) => string[])): void;
};

/** Serialize finder edits so every action starts with the keys returned by Core
 * for the preceding action. A term or external-key change starts a new session;
 * an in-flight response from the old session is then ignored. */
export function useFinderEditQueue({
  year,
  term,
  keys,
  setKeys,
}: {
  year: number;
  term: string;
  keys: string[];
  setKeys: (keys: string[]) => void;
}): FinderEditQueue {
  const callbacks = useRef({ year, term, setKeys });
  callbacks.current = { year, term, setKeys };
  const identity = `${year}:${term}`;
  const externalKey = JSON.stringify(keys);
  const state = useRef({
    identity,
    externalKey,
    keys,
    generation: 0,
    tail: Promise.resolve(),
  });
  if (
    state.current.identity !== identity ||
    state.current.externalKey !== externalKey
  ) {
    state.current.identity = identity;
    state.current.externalKey = externalKey;
    state.current.keys = keys;
    state.current.generation += 1;
    state.current.tail = Promise.resolve();
  }

  const replace = useCallback(
    (update: string[] | ((current: string[]) => string[])) => {
      const current = state.current;
      const nextKeys =
        typeof update === "function" ? update(current.keys) : update;
      current.generation += 1;
      current.keys = nextKeys;
      current.externalKey = JSON.stringify(nextKeys);
      current.tail = Promise.resolve();
      callbacks.current.setKeys(nextKeys);
    },
    [],
  );

  const run = useCallback((action: FinderAction) => {
    const current = state.current;
    const generation = current.generation;
    const identity = current.identity;
    const operation = current.tail.then(async () => {
      if (
        state.current.generation !== generation ||
        state.current.identity !== identity
      )
        return undefined;
      const callback = callbacks.current;
      const next = await finderQuery({
        year: callback.year,
        term: callback.term,
        keys: state.current.keys,
        action,
      });
      if (
        state.current.generation !== generation ||
        state.current.identity !== identity
      )
        return undefined;
      state.current.keys = next.keys;
      state.current.externalKey = JSON.stringify(next.keys);
      callback.setKeys(next.keys);
      return next;
    });
    current.tail = operation.then(
      () => undefined,
      () => undefined,
    );
    return operation;
  }, []);

  return { run, replace };
}

export function useFinderQuery(
  request: FinderRequest,
  refresh: unknown = 0,
  enabled = true,
): QueryState<FinderView> {
  const revision = useWorkspace((state) => state.contentRevision);
  const requestKey = JSON.stringify(request);
  const key = JSON.stringify([revision, request, refresh]);
  const [state, setState] = useState<
    QueryState<FinderView> & { key?: string; requestKey?: string }
  >({ pending: true });
  useEffect(() => {
    if (!enabled) return;
    let active = true;
    let pending = pendingFinderQueries.get(key);
    if (!pending) {
      pending = finderQuery(request);
      pendingFinderQueries.set(key, pending);
      while (pendingFinderQueries.size > 128)
        pendingFinderQueries.delete(pendingFinderQueries.keys().next().value!);
      void pending.catch(() => pendingFinderQueries.delete(key));
    }
    void pending.then(
      (value) => {
        if (active) setState({ key, requestKey, value, pending: false });
      },
      (reason) => {
        if (active)
          setState({
            key,
            requestKey,
            error: reason instanceof Error ? reason.message : String(reason),
            pending: false,
          });
      },
    );
    return () => {
      active = false;
    };
  }, [key, enabled]);
  if (!enabled) return { pending: false };
  return state.key === key
    ? state
    : {
        value: state.requestKey === requestKey ? state.value : undefined,
        pending: true,
      };
}
