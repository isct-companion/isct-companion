import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { t } from "../../../i18n";
import { readNote, saveNote, moveNote, type CourseFolder } from "./files";
const MarkdownDocument = lazy(() =>
  import("./MarkdownDocument").then((m) => ({ default: m.MarkdownDocument })),
);
export function NoteEditor({
  context,
  id,
  onSaved,
  onDeleted,
}: {
  context: CourseFolder;
  id: string;
  onSaved: () => void;
  onDeleted?: () => void;
}) {
  const [text, setText] = useState<string>();
  const [status, setStatus] = useState("Loading…");
  const [failure, setFailure] = useState(false);
  const [reload, setReload] = useState(0);
  const [deleting, setDeleting] = useState(false);
  const removed = useRef(false);
  const draft = useRef<{ text: string; dirty: boolean }>({
    text: "",
    dirty: false,
  });
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const mounted = useRef(true);
  const savedCallback = useRef(onSaved);
  savedCallback.current = onSaved;
  const flush = () => {
    clearTimeout(timer.current);
    if (removed.current || !draft.current.dirty) return;
    const value = draft.current.text;
    draft.current.dirty = false;
    if (mounted.current) setStatus("Saving…");
    void saveNote(context, id, value)
      .then(() => {
        savedCallback.current();
        if (draft.current.text === value && !draft.current.dirty) {
          try {
            localStorage.removeItem(`lms-note-recovery:${id}`);
          } catch {
            /* Optional recovery storage. */
          }
        }
        if (mounted.current && draft.current.text === value) {
          setStatus("Saved locally");
          setFailure(false);
        }
      })
      .catch((error) => {
        draft.current.dirty = true;
        if (mounted.current) {
          setStatus(String(error));
          setFailure(true);
        }
        // Retain a recovery draft even when disk is full or a folder is unavailable.
        try {
          localStorage.setItem(`lms-note-recovery:${id}`, draft.current.text);
        } catch {
          /* UI keeps the draft. */
        }
      });
  };
  useEffect(() => {
    mounted.current = true;
    setText(undefined);
    setFailure(false);
    setStatus("Loading…");
    let cancelled = false;
    void readNote(context, id)
      .then((file) => {
        if (cancelled) return;
        if (file.text == null)
          throw new Error(
            "This note exceeds the 1 MB editing limit. Open it from its storage folder.",
          );
        const recovery = localStorage.getItem(`lms-note-recovery:${id}`);
        const value = recovery ?? file.text ?? "";
        draft.current = { text: value, dirty: recovery !== null };
        setText(value);
        setStatus(
          recovery !== null ? "Recovered unsaved draft" : "Saved locally",
        );
      })
      .catch((error) => {
        if (!cancelled) {
          setStatus(String(error));
          setFailure(true);
        }
      });
    const leave = () => flush();
    window.addEventListener("pagehide", leave);
    return () => {
      cancelled = true;
      mounted.current = false;
      window.removeEventListener("pagehide", leave);
      flush();
    };
    // A tab is keyed by note ID; context is fixed for its lifetime.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id, context.term, context.course, context.section, reload]);
  const remove = async () => {
    clearTimeout(timer.current); removed.current = true; setDeleting(true);
    try {
      if (draft.current.dirty) await saveNote(context, id, draft.current.text);
      await moveNote(context, id);
      localStorage.removeItem(`lms-note-recovery:${id}`);
      onDeleted?.();
    } catch (error) { removed.current = false; setStatus(String(error)); setFailure(true); setDeleting(false); }
  };
  return (
    <div className="lmsNoteEditor">
      <div className="lmsSourceBar">
        <span role="status">{t(status)}</span>
        <button
          type="button"
          disabled={deleting}
          onClick={() =>
            text === undefined ? setReload((value) => value + 1) : flush()
          }
        >
          {t(text === undefined ? "Retry" : failure ? "Retry save" : "Save")}
        </button>
        {onDeleted && <button type="button" disabled={deleting || text === undefined} onClick={() => void remove()}>{t(deleting ? "Deleting…" : "Delete note")}</button>}
      </div>
      {text !== undefined && (
        <Suspense fallback={<p>{t("Loading editor…")}</p>}>
          <MarkdownDocument
            value={text}
            onChange={(value) => {
              if (removed.current) return;
              draft.current = { text: value, dirty: true };
              setText(value);
              setStatus("Unsaved changes");
              clearTimeout(timer.current);
              timer.current = setTimeout(flush, 800);
            }}
          />
        </Suspense>
      )}
    </div>
  );
}
