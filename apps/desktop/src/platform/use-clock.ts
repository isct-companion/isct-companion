import { useSyncExternalStore } from "react";

const MINUTE = 60_000;

/** The current time floored to the minute. Every Core query that takes `now`
 * uses this one clock, so consumers share request keys within a minute. */
export function minuteNow(at = Date.now()): string {
  return new Date(at - (at % MINUTE)).toISOString();
}

// One clock for the whole shell. The snapshot is read from the wall clock on
// every render, so no component can hold a stale minute (a page kept alive by
// `<Activity>` or a throttled timer); the timer only wakes subscribers when
// the minute turns.
const listeners = new Set<() => void>();
let timer: ReturnType<typeof setTimeout> | undefined;

function notify() {
  for (const listener of listeners) listener();
}

function arm() {
  clearTimeout(timer);
  timer = listeners.size
    ? setTimeout(
        () => {
          notify();
          arm();
        },
        MINUTE - (Date.now() % MINUTE),
      )
    : undefined;
}

/** Wakes `listener` on each minute boundary until unsubscribed. */
export function subscribeClock(listener: () => void) {
  listeners.add(listener);
  arm();
  return () => {
    listeners.delete(listener);
    arm();
  };
}

const unsubscribed = () => () => {};

/** Re-renders every subscriber now. The lifecycle hook calls it when the
 * window comes back, since a hidden window's timers may not have fired. */
export function resyncClock() {
  notify();
  arm();
}

/** A stable shell clock fact. Core owns every decision derived from it.
 * It re-renders on each minute boundary while active. */
export function useClock(active = true): string {
  return useSyncExternalStore(
    active ? subscribeClock : unsubscribed,
    minuteNow,
  );
}
