import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { t } from "../../../i18n";
import { readNote, saveNote, moveNote, type CourseFolder } from "./files";
const MarkdownDocument = lazy(() =>
  import("./MarkdownDocument").then((m) => ({ default: m.MarkdownDocument })),
);
const RECOVERY_PREFIX = "lms-note-recovery:";
const RECOVERY_TTL_MS = 30 * 24 * 60 * 60 * 1000;
const RECOVERY_MAX_ENTRIES = 20;
const RECOVERY_MAX_BYTES = 5 * 1024 * 1024;
type RecoveryDraft = { version: 1; updatedAt: string; text: string };
const recoveryKey = (id: string) => `${RECOVERY_PREFIX}${id}`;
const recoveryBytes = (value: string) => new TextEncoder().encode(value).length;
function recoveryEntries() {
  const entries: Array<{ key: string; raw: string; draft: RecoveryDraft }> = [];
  const expired = Date.now() - RECOVERY_TTL_MS;
  for (let index = localStorage.length - 1; index >= 0; index--) {
    const key = localStorage.key(index);
    if (!key?.startsWith(RECOVERY_PREFIX)) continue;
    const raw = localStorage.getItem(key);
    if (raw == null) continue;
    try {
      const parsed = JSON.parse(raw) as RecoveryDraft;
      if (parsed.version !== 1 || typeof parsed.text !== "string" || Date.parse(parsed.updatedAt) < expired)
        throw new Error("expired");
      entries.push({ key, raw, draft: parsed });
    } catch {
      // Old unbounded/plain-text recovery values are discarded deliberately.
      localStorage.removeItem(key);
    }
  }
  return entries.sort((a, b) => Date.parse(b.draft.updatedAt) - Date.parse(a.draft.updatedAt));
}
function pruneRecovery() {
  let bytes = 0;
  for (const [index, entry] of recoveryEntries().entries()) {
    bytes += recoveryBytes(entry.raw);
    if (index >= RECOVERY_MAX_ENTRIES || bytes > RECOVERY_MAX_BYTES)
      localStorage.removeItem(entry.key);
  }
}
function writeRecovery(id: string, text: string) {
  if (recoveryBytes(text) > 1_000_000) return;
  try {
    localStorage.setItem(recoveryKey(id), JSON.stringify({ version: 1, updatedAt: new Date().toISOString(), text } satisfies RecoveryDraft));
    pruneRecovery();
  } catch {
    pruneRecovery();
  }
}
function readRecovery(id: string) {
  pruneRecovery();
  const raw = localStorage.getItem(recoveryKey(id));
  if (!raw) return null;
  try { return (JSON.parse(raw) as RecoveryDraft).text; }
  catch { localStorage.removeItem(recoveryKey(id)); return null; }
}
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
  const loadedNote = useRef<string | undefined>(undefined);
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
            localStorage.removeItem(recoveryKey(id));
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
          writeRecovery(id, draft.current.text);
        } catch {
          /* UI keeps the draft. */
        }
      });
  };
  useEffect(() => {
    mounted.current = true;
    const noteKey = JSON.stringify([
      id,
      context.term,
      context.course,
      context.section,
      reload,
    ]);
    let cancelled = false;
    // Revealing an Activity must keep the live draft, even while its save is pending.
    if (loadedNote.current !== noteKey) {
      setText(undefined);
      setFailure(false);
      setStatus("Loading…");
      void readNote(context, id)
        .then((file) => {
          if (cancelled) return;
          if (file.text == null)
            throw new Error(
              "This note exceeds the 1 MB editing limit. Open it from its storage folder.",
            );
          const recovery = readRecovery(id);
          const value = recovery ?? file.text ?? "";
          loadedNote.current = noteKey;
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
    }
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
      localStorage.removeItem(recoveryKey(id));
      onDeleted?.();
    } catch (error) { removed.current = false; setStatus(String(error)); setFailure(true); setDeleting(false); }
  };
  return (
    <div className="lmsNoteEditor">
      <div className="lmsSourceBar">
        <span>{t("Local note · stored only on this device")}</span>
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
              writeRecovery(id, value);
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
