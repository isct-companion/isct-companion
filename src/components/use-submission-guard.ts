import { useCallback, useRef, useState } from "react";

type Submission = () => Promise<unknown> | void;

export function useSubmissionGuard() {
  const locked = useRef(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const run = useCallback(async (submission: Submission) => {
    // State updates render later, so the ref also blocks a second click in this frame.
    if (locked.current) return false;
    locked.current = true;
    setBusy(true);
    setError("");
    try {
      await submission();
      return true;
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      return false;
    } finally {
      locked.current = false;
      setBusy(false);
    }
  }, []);

  return { busy, error, run };
}
