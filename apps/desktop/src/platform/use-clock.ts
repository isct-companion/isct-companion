import { useEffect, useState } from "react";

const MINUTE = 60_000;

/** The current time floored to the minute. Every Core query that takes `now`
 * uses this one clock, so consumers share request keys within a minute. */
export function minuteNow(at = Date.now()): string {
  return new Date(at - (at % MINUTE)).toISOString();
}

/** A stable shell clock fact. Core owns every decision derived from it.
 * It ticks on each minute boundary while active. */
export function useClock(active = true): string {
  const [now, setNow] = useState(() => minuteNow());
  useEffect(() => {
    if (!active) return;
    let timer: number;
    const tick = () => {
      setNow(minuteNow());
      timer = window.setTimeout(tick, MINUTE - (Date.now() % MINUTE));
    };
    tick();
    return () => window.clearTimeout(timer);
  }, [active]);
  return now;
}
