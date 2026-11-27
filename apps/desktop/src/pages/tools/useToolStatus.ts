import { useCallback, useEffect, useRef, useState } from "react";
import { useEventListener } from "usehooks-ts";

/**
 * A tool's network state, read when the page is shown and again whenever
 * the window regains focus: the Wi-Fi or account may have changed while the
 * app sat in the background. A retained page re-runs its effects each time
 * its `<Activity>` becomes visible, so the mount effect covers returning to
 * the page. Only the latest read lands.
 */
export function useToolStatus<T>(read: () => Promise<T | undefined>) {
  const [status, setStatus] = useState<T>();
  const [checking, setChecking] = useState(true);
  const latest = useRef(0);
  const readRef = useRef(read);
  readRef.current = read;

  const check = useCallback(async () => {
    const call = ++latest.current;
    setChecking(true);
    const next = await readRef.current().catch(() => undefined);
    if (call !== latest.current) return;
    setStatus(next);
    setChecking(false);
  }, []);

  useEffect(() => {
    void check();
    return () => {
      latest.current++;
    };
  }, [check]);
  useEventListener("focus", () => void check());

  return { status, setStatus, checking, check };
}
