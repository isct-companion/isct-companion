import { useCallback, useEffect, useRef, useState } from "react";
import type {
  ExamAction,
  ExamAnswer,
  ExamLibrary,
} from "../../generated/persistence";

/** Keep the last read across tabs. A hidden library reads a dirty revision
 * when it next appears, and an older response cannot replace a write's result. */
export function useExamLibrary(
  call: (action: ExamAction) => Promise<ExamAnswer | undefined>,
  visible: boolean,
) {
  const [library, setLibrary] = useState<ExamLibrary>();
  const [failed, setFailed] = useState(false);
  const [revision, setRevision] = useState(0);
  const current = useRef(0);
  const requested = useRef(-1);

  const refresh = useCallback(() => setRevision(++current.current), []);
  const replace = useCallback((value: ExamLibrary | undefined) => {
    const next = ++current.current;
    if (value) requested.current = next;
    setLibrary(value);
    setFailed(false);
    setRevision(next);
  }, []);

  useEffect(() => {
    if (
      !visible ||
      revision !== current.current ||
      requested.current === revision
    )
      return;
    requested.current = revision;
    setFailed(false);
    void call({ kind: "library" }).then((answer) => {
      if (current.current !== revision) return;
      if (answer?.kind === "library") setLibrary(answer.value);
      else setFailed(true);
    });
  }, [call, revision, visible]);

  return { library, failed, refresh, replace };
}
