import { useCallback, useRef, useState } from "react";
import { errorText } from "../app/shared";
import { t } from "../i18n";

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
      // Same wording as the toast the rest of the app shows for a failed save.
      setError(t(errorText(cause) ?? "unknown error"));
      return false;
    } finally {
      locked.current = false;
      setBusy(false);
    }
  }, []);

  return { busy, error, run };
}
