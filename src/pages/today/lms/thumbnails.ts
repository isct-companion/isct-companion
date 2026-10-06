import { desktopBridge } from "../../../app/native";
import { schoolSessions } from "../../../platform/school-session";
import { fileSrc, openMaterial, type CourseFolder } from "./files";

/**
 * One loader for every LMS image thumbnail (outline rows and inline images).
 * Results are remembered per URL so collapsing a section or switching courses
 * never re-invokes the backend, and at most two loads run at once so a long
 * week of images cannot starve the file the person actually opened.
 */
const resolved = new Map<string, string>();
const pending = new Map<string, Promise<string | undefined>>();
const waiting: (() => void)[] = [];
let running = 0;

async function slot<T>(task: () => Promise<T>): Promise<T> {
  if (running >= 2) await new Promise<void>((resolve) => waiting.push(resolve));
  running++;
  try {
    return await task();
  } finally {
    running--;
    waiting.shift()?.();
  }
}

const attachment = (url: string) => {
  try {
    return new URL(url).pathname.includes("/pluginfile.php/");
  } catch {
    return false;
  }
};

export const cachedThumbnail = (url: string) =>
  !desktopBridge || !attachment(url) ? url : resolved.get(url);

/** Resolve a displayable src, or undefined when it is not available yet. */
export function loadThumbnail(context: CourseFolder, url: string, limit: number): Promise<string | undefined> {
  const hit = cachedThumbnail(url);
  if (hit) return Promise.resolve(hit);
  let job = pending.get(url);
  if (!job) {
    // Never start a download that can only wait for a sign-in that failed.
    const session = schoolSessions.get("science");
    const download = session !== "failed" && session !== "not-configured";
    job = slot(async () => {
      const material = await openMaterial(context, url, { download, limit });
      if (material.state !== "ready") return undefined;
      const src = fileSrc(material.file);
      resolved.set(url, src);
      return src;
    }).finally(() => pending.delete(url));
    pending.set(url, job);
  }
  return job;
}

/** Call `retry` whenever LMS sign-in succeeds, so unavailable thumbnails reload. */
export function onSignedIn(retry: () => void) {
  let active = schoolSessions.get("science") === "active";
  return schoolSessions.subscribe(() => {
    const now = schoolSessions.get("science") === "active";
    if (now && !active) retry();
    active = now;
  });
}

/** Forget a src that failed to decode so a later attempt can fetch again. */
export const dropThumbnail = (url: string) => void resolved.delete(url);
