import { useEffect, useState } from "react";

/** A stable shell clock fact. Core owns every decision derived from it. */
export function useClock(intervalMs = 60_000, active = true): string {
  const [now, setNow] = useState(() => new Date().toISOString());
  useEffect(() => {
    if (!active) return;
    setNow(new Date().toISOString());
    const timer = window.setInterval(
      () => setNow(new Date().toISOString()),
      intervalMs,
    );
    return () => window.clearInterval(timer);
  }, [active, intervalMs]);
  return now;
}
