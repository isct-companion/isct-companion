import { useEffect, useRef, useState } from "react";
import { t } from "../../i18n";
import {
  listNotes,
  saveNote,
  type CourseFolder,
  type LocalFile,
} from "./files";
import type { OpenedContent } from "./reading-state";

/** A course section notes are kept under. */
type NoteSection = { id: number; title: string };

export function useCourseNotes(
  folder: CourseFolder,
  sections: NoteSection[],
  active: boolean,
  open: (content: OpenedContent) => void,
) {
  const [notes, setNotes] = useState<LocalFile[]>([]);
  const [noteError, setNoteError] = useState("");
  const [deletedNote, setDeletedNote] = useState<{
    id: string;
    section: string;
    title: string;
  } | null>(null);
  const noteCreation = useRef(false);
  const [creatingNote, setCreatingNote] = useState(false);
  const refreshNotes = () =>
    void listNotes(
      folder,
      sections.map((s) => `section-${s.id}`),
    )
      .then((value) => setNotes(value.sort((a, b) => a.id.localeCompare(b.id))))
      .catch((error) => setNoteError(String(error)));
  useEffect(() => {
    if (!active) return;
    refreshNotes();
    window.addEventListener("course-storage-changed", refreshNotes);
    return () =>
      window.removeEventListener("course-storage-changed", refreshNotes);
  }, [active, folder, sections]);
  const newNote = async (section: NoteSection) => {
    if (noteCreation.current) return;
    noteCreation.current = true;
    setCreatingNote(true);
    setNoteError("");
    const context = { ...folder, section: `section-${section.id}` };
    try {
      const existing = (await listNotes(folder, [context.section])).sort(
        (a, b) => a.id.localeCompare(b.id),
      )[0];
      let id = existing?.id;
      if (!existing)
        id = (
          await saveNote(
            context,
            "",
            `# ${section.title}

`,
          )
        ).id;
      refreshNotes();
      open({
        id: id!,
        title: existing?.name.trim() || `${section.title} · ${t("lms.note")}`,
        kind: "note",
        section: context.section,
      });
    } catch (error) {
      setNoteError(String(error));
    } finally {
      noteCreation.current = false;
      setCreatingNote(false);
    }
  };

  return {
    notes,
    noteError,
    setNoteError,
    deletedNote,
    setDeletedNote,
    creatingNote,
    refreshNotes,
    newNote,
  };
}
