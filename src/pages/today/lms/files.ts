import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { desktopBridge } from "../../../app/native";
export type CourseFolder = { term: string; course: string; section: string };
export type LocalFile = {
  id: string;
  name: string;
  section: string;
  path: string;
  bytes: number;
  cached: boolean;
  text?: string | null;
};
export type StorageInfo = { root: string; custom: boolean; canChoose: boolean };
const prefix = "stws-lms-note:";
const queues = new Map<string, Promise<unknown>>();
const key = (context: CourseFolder, id: string) =>
  `${prefix}${context.term}/${context.course}/${context.section}/${id}`;
export const folderComponent = (value: string) => {
  const name = value.replace(/[^a-zA-Z0-9_.-]/g, "_").slice(0, 120);
  return !name || name === "." || name === ".." ? "course" : name;
};
export const fileSrc = (file: LocalFile) => convertFileSrc(file.path);
export function previewKind(
  url: string,
): "pdf" | "markdown" | "text" | "image" | undefined {
  let path: string;
  try {
    path = new URL(url).pathname;
  } catch {
    path = url;
  }
  if (/\.pdf$/i.test(path)) return "pdf";
  if (/\.(md|markdown)$/i.test(path)) return "markdown";
  if (
    /\.(txt|tex|js|jsx|ts|tsx|java|py|r|c|cpp|cxx|cc|h|hpp|rs|go|rb|sh|sql|json|yaml|yml|toml|xml|css|csv|log)$/i.test(
      path,
    )
  )
    return "text";
  if (/\.(png|jpe?g|gif|webp)$/i.test(path)) return "image";
}
export async function storageInfo(): Promise<StorageInfo> {
  return desktopBridge
    ? invoke("course_storage_info")
    : {
        root: "Browser storage · desktop app saves to Documents",
        custom: false,
        canChoose: false,
      };
}
export async function chooseStorage(reset = false): Promise<StorageInfo> {
  return invoke("choose_course_storage", { reset });
}
export async function listNotes(
  context: CourseFolder,
  sections: string[],
): Promise<LocalFile[]> {
  if (desktopBridge) return invoke("list_course_notes", { context, sections });
  const notes: LocalFile[] = [];
  const base = `${prefix}${context.term}/${context.course}/`;
  for (let index = 0; index < localStorage.length; index++) {
    const name = localStorage.key(index);
    if (!name?.startsWith(base)) continue;
    const [section, id] = name.slice(base.length).split("/");
    if (id.endsWith(":deleted")) continue;
    if (!sections.includes(section)) continue;
    const text = localStorage.getItem(name) ?? "";
    notes.push({
      id,
      section,
      name: text.split("\n")[0].replace(/^#+\s*/, "") || "Note",
      path: name,
      bytes: text.length,
      cached: true,
    });
  }
  return notes;
}
export async function readNote(
  context: CourseFolder,
  id: string,
): Promise<LocalFile> {
  await queues.get(key(context, id))?.catch(() => undefined);
  if (desktopBridge) return invoke("read_course_note", { context, id });
  const text = localStorage.getItem(key(context, id));
  if (text === null) throw new Error("Note was not found");
  return {
    id,
    section: context.section,
    name: "Note",
    path: key(context, id),
    bytes: text.length,
    cached: true,
    text,
  };
}
export function saveNote(
  context: CourseFolder,
  id: string,
  text: string,
): Promise<LocalFile> {
  const name = key(context, id);
  const previous = queues.get(name) ?? Promise.resolve();
  const next = previous
    .catch(() => undefined)
    .then(async () => {
      if (new TextEncoder().encode(text).length > 1_000_000)
        throw new Error("Notes are limited to 1 MB");
      if (desktopBridge)
        return invoke<LocalFile>("save_course_note", { context, id, text });
      localStorage.setItem(name, text);
      return {
        id,
        section: context.section,
        name: text.split("\n")[0].replace(/^#+\s*/, "") || "Note",
        path: name,
        bytes: text.length,
        cached: false,
      };
    });
  queues.set(name, next);
  void next
    .finally(() => {
      if (queues.get(name) === next) queues.delete(name);
    })
    .catch(() => undefined);
  return next;
}
export async function openMaterial(
  context: CourseFolder,
  url: string,
  download: boolean,
  readText: boolean,
  autoSave = false,
): Promise<LocalFile | null> {
  if (!desktopBridge) return null;
  return invoke("open_course_material", { context, url, download, readText, autoSave });
}

export async function downloadMaterial(context: CourseFolder, url: string): Promise<string | null> { return invoke("save_course_material", { context, url }); }

export async function moveNote(context: CourseFolder, id: string, restore = false): Promise<void> {
  const name = key(context, id);
  const previous = queues.get(name) ?? Promise.resolve();
  const operation = previous.catch(() => undefined).then(async () => {
    if (desktopBridge) { await invoke("move_course_note", { context, id, restore }); return; }
    const trash = `${name}:deleted`;
    const source = restore ? trash : name; const target = restore ? name : trash;
    const text = localStorage.getItem(source);
    if (text === null) throw new Error("Note was not found");
    if (localStorage.getItem(target) !== null) throw new Error("A note already exists at the destination");
    localStorage.setItem(target, text); localStorage.removeItem(source);
  });
  queues.set(name, operation);
  try { await operation; } finally { if (queues.get(name) === operation) queues.delete(name); }
}
