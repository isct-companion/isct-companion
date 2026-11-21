import { invoke } from "../../platform/native-error";
import { nativeOs, type NativeOs } from "../../platform/desktop-bridge";
import type { SchoolSite, Viewer } from "../../generated/persistence";
/** Where a note lives: a section of a course; Core names the course's folder. */
export type CourseFolder = { courseId: string; section: string };
export type LocalFile = {
  id: string;
  name: string;
  section: string;
  path: string;
  bytes: number;
  cached: boolean;
  text?: string | null;
  /** How the reader shows it (Core's `file_preview`). */
  viewer: Viewer;
};
/** Writes to one note run one after another. */
const queues = new Map<string, Promise<unknown>>();
const key = (context: CourseFolder, id: string) =>
  `${context.courseId}/${context.section}/${id}`;
/** Why Core could not open an item (`MaterialFailure`). */
export type LmsFileReason =
  | "login"
  | "unresolved"
  | "network"
  | "invalid"
  | "timeout"
  | "cancelled"
  | "denied";

/**
 * The app URI a page loads a course file from (`lms_protocol.rs`): Core
 * serves the file of `source` for the course `courseId`.
 */
export function lmsFileUrl(
  source: string,
  courseId: string,
  options: { download?: boolean; foreground?: boolean; os?: NativeOs } = {},
) {
  // WebView2 only serves custom schemes over http://<scheme>.localhost.
  const os = options.os ?? nativeOs();
  const origin =
    os === "windows" ? "http://lmsfile.localhost" : "lmsfile://localhost";
  const query = new URLSearchParams({ u: source, course: courseId });
  if (options.download) query.set("download", "1");
  if (options.foreground) query.set("foreground", "1");
  return `${origin}/get?${query}`;
}

export { previewKind } from "../../lib/file-types";
/** The notes of course `courseId` in `sections`. */
export const listNotes = (
  courseId: string,
  sections: string[],
): Promise<LocalFile[]> => invoke("list_course_notes", { courseId, sections });
export async function readNote(
  context: CourseFolder,
  id: string,
): Promise<LocalFile> {
  await queues.get(key(context, id))?.catch(() => undefined);
  return invoke("read_course_note", { context, id });
}
export function saveNote(
  context: CourseFolder,
  id: string,
  text: string,
): Promise<LocalFile> {
  const name = key(context, id || "new");
  const previous = queues.get(name) ?? Promise.resolve();
  const next = previous
    .catch(() => undefined)
    .then(() => invoke<LocalFile>("save_course_note", { context, id, text }));
  queues.set(name, next);
  void next
    .finally(() => {
      if (queues.get(name) === next) queues.delete(name);
    })
    .catch(() => undefined);
  return next;
}
/** A school page: where an LMS source lives, or a link to a school site. */
export type SchoolPage = { site: SchoolSite; url: string };
/** A file inside a downloaded ZIP (Core's `ArchiveFile`): `url` is the
 * source it opens as, like a folder file's. */
export type ArchiveFile = { name: string; url: string; bytes: number };
/**
 * What an item or link opens as (Core's `Material`): a course file on disk or
 * why it is not (yet), a folder's or a ZIP's files, a school page, or an
 * outside page. `page` is the LMS page of an LMS source, offered instead of
 * the file.
 */
export type Material =
  | { state: "ready"; file: LocalFile; page: SchoolPage }
  | { state: "missing"; page: SchoolPage }
  | { state: "tooLarge"; bytes: number; page: SchoolPage }
  | {
      state: "folder";
      files: { name: string; url: string }[];
      page: SchoolPage;
    }
  | {
      state: "archive";
      file: LocalFile;
      files: ArchiveFile[];
      page: SchoolPage;
    }
  | ({ state: "page" } & SchoolPage)
  | { state: "external"; url: string }
  | { state: "failed"; reason: LmsFileReason; page: SchoolPage | null };

/**
 * Asks Core what an item or link opens as. A file is read from the course
 * folder; with `download`, Core fetches it first with its signed-in LMS
 * session. Files over `limit` bytes report their size.
 */
export async function openMaterial(
  courseId: string,
  url: string,
  {
    download = false,
    readText = false,
    limit,
  }: { download?: boolean; readText?: boolean; limit?: number } = {},
): Promise<Material> {
  return invoke("open_course_material", {
    courseId,
    url,
    download,
    readText,
    limit,
  });
}
export const openStoredFile = (path: string, reveal = false): Promise<void> =>
  invoke("open_course_file", { path, reveal });
/** Puts a stored file itself on the system clipboard (macOS, Windows). */
export const copyStoredFile = (path: string): Promise<void> =>
  invoke("copy_course_file", { path });
/** Opens the system share UI for a stored file at `anchor` (macOS, Windows). */
export const shareStoredFile = (
  path: string,
  anchor: { x: number; y: number; width: number; height: number },
): Promise<void> => invoke("share_course_file", { path, anchor });
export async function moveNote(
  context: CourseFolder,
  id: string,
  restore = false,
): Promise<void> {
  const name = key(context, id);
  const previous = queues.get(name) ?? Promise.resolve();
  const operation = previous
    .catch(() => undefined)
    .then(() => invoke("move_course_note", { context, id, restore }));
  queues.set(name, operation);
  try {
    await operation;
  } finally {
    if (queues.get(name) === operation) queues.delete(name);
  }
}
