import { useCallback, useRef } from "react";
import type {
  Factor,
  FinderContext,
  FinderCourse,
  FinderRow,
  Suggestion,
} from "../lib/course-query";
import type { CourseEligibility } from "../generated/persistence";
import { useViewQuery } from "./view-query";
import { getDesktopBridge } from "./desktop-bridge";
import type { QueryState } from "./core-query";

export type { FinderAction, FinderView } from "../generated/persistence";
import type {
  FinderAction,
  FinderRequest as CoreFinderRequest,
  FinderView,
} from "../generated/persistence";
export type FinderRequest = Omit<
  CoreFinderRequest,
  "includeIneligible" | "keys" | "input" | "action"
> &
  Partial<
    Pick<
      CoreFinderRequest,
      "includeIneligible" | "keys" | "input" | "action"
    >
  >;

export function finderQuery(request: FinderRequest): Promise<FinderView> {
  const bridge = getDesktopBridge();
  if (!bridge)
    return Promise.reject(new Error("Course search requires the native Core."));
  return bridge.finderQuery(request);
}

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
  return useViewQuery(
    "finder",
    request,
    () => finderQuery(request),
    enabled,
    refresh,
  );
}
