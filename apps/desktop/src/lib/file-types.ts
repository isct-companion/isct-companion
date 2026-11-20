import {
  FILE_KINDS,
  type FileKind,
  type Viewer,
} from "../generated/persistence";

export type PreviewKind =
  | "pdf" | "markdown" | "text" | "image" | "video" | "audio" | "archive";
/** Visual family of an LMS item; drives the row icon and its tint. */
export type FileFamily =
  | PreviewKind
  | "doc" | "slides" | "sheet" | "file"
  | "link" | "folder" | "forum" | "page" | "note";

// Kinds, extensions and viewers are Core's (`file_preview`). Before a file is
// downloaded its address is all there is, so its name picks the row's icon
// and whether it previews; once it is on disk, the file's own `viewer`
// decides (`previewOf`).
const kinds = new Map<string, (typeof FILE_KINDS)[number]>(
  FILE_KINDS.flatMap((entry) =>
    entry.extensions.map((extension) => [extension, entry] as const),
  ),
);
/** The icon each kind shows; a notebook or web page reads as text. */
const familyOf: Record<FileKind, FileFamily> = {
  pdf: "pdf",
  markdown: "markdown",
  text: "text",
  image: "image",
  video: "video",
  audio: "audio",
  doc: "doc",
  slides: "slides",
  sheet: "sheet",
  zip: "archive",
  archive: "archive",
  notebook: "text",
  web: "text",
};

/** What the reader shows for a viewer Core chose, or undefined for a file
 * another app opens. */
export function previewOf(viewer: Viewer): PreviewKind | undefined {
  switch (viewer) {
    case "pdf":
    case "markdown":
    case "text":
    case "image":
    case "video":
    case "audio":
      return viewer;
    default:
      return undefined;
  }
}

/** What follows a ZIP's source to name one of its files (Core's
 * `archive_entry_source`). */
const ENTRY_MARK = "#entry=";

export function extension(url: string): string {
  let path: string;
  const entry = url.indexOf(ENTRY_MARK);
  try {
    path = entry >= 0
      ? url.slice(entry + ENTRY_MARK.length)
      : new URL(url).pathname;
  } catch {
    path = url;
  }
  try {
    path = decodeURIComponent(path);
  } catch { /* Keep the raw path when it is not valid percent-encoding. */ }
  return path.match(/\.([a-z0-9]{1,8})$/i)?.[1].toLowerCase() ?? "";
}

export function fileFamily(url: string, module?: string): FileFamily {
  const kind = kinds.get(extension(url))?.kind;
  if (kind) return familyOf[kind];
  if (module === "folder") return "folder";
  if (module === "forum") return "forum";
  if (module === "url") return "link";
  if (module === "page" || module === "label") return "page";
  return "file";
}

/** In-app preview this address will have, or undefined to download it. */
export function previewKind(url: string): PreviewKind | undefined {
  const entry = kinds.get(extension(url));
  if (!entry) return undefined;
  // Core lists a ZIP's files (`Material::Archive`); a ZIP inside a ZIP
  // downloads.
  if (entry.kind === "zip")
    return url.includes(ENTRY_MARK) ? undefined : "archive";
  return previewOf(entry.viewer);
}

/** A ZIP: wherever it is listed or linked, its files open in place as a
 * folder's do, in the outline and never in the reader. */
export const listsAsFolder = (url: string) => previewKind(url) === "archive";

/**
 * A file of a known type the reader cannot show (an archive, a slide deck):
 * it is downloaded only when asked. An address without a known file type
 * (an LMS activity, a link) is not one.
 */
export const downloadOnly = (url: string) =>
  !previewKind(url) && fileFamily(url) !== "file";
