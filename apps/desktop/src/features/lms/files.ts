import { convertFileSrc } from "@tauri-apps/api/core";
import { invoke } from "../../platform/native-error";
import { desktopBridge } from "../../app/native";
import { nativeOs, type NativeOs } from "../../platform/desktop-bridge";
import { COURSE_FILE_POLICY } from "../../lib/course-file-policy";
import type { Settings } from "../../lib/model";
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
const prefix = "isct-companion-lms-note:";
const queues = new Map<string, Promise<unknown>>();
const key = (context: CourseFolder, id: string) =>
  `${prefix}${context.term}/${context.course}/${context.section}/${id}`;
export const folderComponent = (value: string) => {
  const name = value.replace(/[^a-zA-Z0-9_.-]/g, "_").slice(0, 120);
  return !name || name === "." || name === ".." ? "course" : name;
};
export type LmsFileReason =
  | "login"
  | "unresolved"
  | "tooLarge"
  | "network"
  | "invalid"
  | "timeout"
  | "denied";

export class LmsFileError extends Error {
  constructor(
    public reason: LmsFileReason,
    public bytes?: number,
  ) {
    super(reason);
  }
}

export function lmsFileUrl(
  source: string,
  context: Pick<CourseFolder, "term" | "course">,
  options: {
    download?: boolean;
    foreground?: boolean;
    native?: boolean;
    os?: NativeOs;
  } = {},
) {
  const native = options.native ?? Boolean(desktopBridge);
  if (!native) return source;
  // WebView2 only serves custom schemes over http://<scheme>.localhost.
  const os = options.os ?? nativeOs();
  const origin =
    os === "windows" ? "http://lmsfile.localhost" : "lmsfile://localhost";
  const query = new URLSearchParams({
    u: source,
    term: context.term,
    course: context.course,
  });
  if (options.download) query.set("download", "1");
  if (options.foreground) query.set("foreground", "1");
  return `${origin}/get?${query}`;
}

export async function lmsFileResponse(
  source: string,
  context: Pick<CourseFolder, "term" | "course">,
  options: { download?: boolean; foreground?: boolean; range?: boolean } = {},
) {
  const response = await fetch(lmsFileUrl(source, context, options), {
    headers: options.range ? { Range: "bytes=0-0" } : undefined,
  });
  if (!response.ok) {
    const body = (await response.json().catch(() => ({}))) as {
      reason?: LmsFileReason;
      bytes?: number;
    };
    throw new LmsFileError(
      body.reason ??
        ((response.headers.get("X-Isct-Reason") as LmsFileReason | null) ||
          "network"),
      body.bytes,
    );
  }
  return response;
}
const boundedInteger = (
  value: number | undefined,
  policy: { default: number; min: number; max: number },
) => {
  const integer = Number.isFinite(value) ? Math.round(value!) : policy.default;
  return Math.min(policy.max, Math.max(policy.min, integer));
};
/** Byte limit for automatic downloads, from the settings value in MB. */
export const downloadLimit = (
  settings: Pick<Settings, "autoDownloadLimitMb">,
) =>
  boundedInteger(
    settings.autoDownloadLimitMb,
    COURSE_FILE_POLICY.autoDownloadLimitMb,
  ) *
  1024 *
  1024;
/** Days a downloaded file may remain unopened before launch cleanup removes it. */
export const retentionDays = (settings: Pick<Settings, "autoCleanDays">) =>
  boundedInteger(settings.autoCleanDays, COURSE_FILE_POLICY.retentionDays);
export { previewKind } from "./file-types";
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
  // Production IDs come from Rust (UUIDv7). Browser preview has no backend,
  // so its fallback is intentionally confined to localStorage.
  const recordId = id || (desktopBridge ? "" : crypto.randomUUID());
  const name = key(context, recordId || "new");
  const previous = queues.get(name) ?? Promise.resolve();
  const next = previous
    .catch(() => undefined)
    .then(async () => {
      if (new TextEncoder().encode(text).length > 1_000_000)
        throw new Error("Notes are limited to 1 MB");
      if (desktopBridge)
        return invoke<LocalFile>("save_course_note", {
          context,
          id: recordId,
          text,
        });
      const storageKey = key(context, recordId);
      localStorage.setItem(storageKey, text);
      return {
        id: recordId,
        section: context.section,
        name: text.split("\n")[0].replace(/^#+\s*/, "") || "Note",
        path: storageKey,
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
/** A course file on disk, or why it is not (yet). */
export type Material =
  | { state: "ready"; file: LocalFile }
  | { state: "missing" }
  | { state: "tooLarge"; bytes: number }
  | { state: "folder"; files: { name: string; url: string }[] }
  | { state: "external"; url: string }
  | { state: "failed"; reason: LmsFileReason };

export const localFileUrl = (path: string) => convertFileSrc(path);
/**
 * Read a course file from the course folder; with `download`, fetch it first
 * through the signed-in LMS view. Files over `limit` bytes report their size.
 */
export async function openMaterial(
  context: CourseFolder,
  url: string,
  {
    download = false,
    readText = false,
    limit,
  }: { download?: boolean; readText?: boolean; limit?: number } = {},
): Promise<Material> {
  if (!desktopBridge) return { state: "missing" };
  return invoke("open_course_material", {
    context,
    url,
    download,
    readText,
    limit,
  });
}
export const openStoredFile = (path: string, reveal = false): Promise<void> =>
  invoke("open_course_file", { path, reveal });
/** Remove downloaded files not opened for `days` days. */
export const cleanMaterials = (days: number): Promise<number> =>
  desktopBridge
    ? invoke("clean_course_materials", { days })
    : Promise.resolve(0);

export async function moveNote(
  context: CourseFolder,
  id: string,
  restore = false,
): Promise<void> {
  const name = key(context, id);
  const previous = queues.get(name) ?? Promise.resolve();
  const operation = previous
    .catch(() => undefined)
    .then(async () => {
      if (desktopBridge) {
        await invoke("move_course_note", { context, id, restore });
        return;
      }
      const trash = `${name}:deleted`;
      const source = restore ? trash : name;
      const target = restore ? name : trash;
      const text = localStorage.getItem(source);
      if (text === null) throw new Error("Note was not found");
      if (localStorage.getItem(target) !== null)
        throw new Error("A note already exists at the destination");
      localStorage.setItem(target, text);
      localStorage.removeItem(source);
    });
  queues.set(name, operation);
  try {
    await operation;
  } finally {
    if (queues.get(name) === operation) queues.delete(name);
  }
}
