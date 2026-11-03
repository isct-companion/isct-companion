import { lmsFileUrl, type CourseFolder } from "./files";
import { schoolSessions } from "../../platform/school-session";

/**
 * Images use the same app URI as documents. The backend owns caching,
 * single-flight and its four-lane small-file pool, so the frontend keeps no
 * second scheduler or local/remote URL state.
 */
export const cachedThumbnail = (
  context: CourseFolder,
  url: string,
  download = false,
) => lmsFileUrl(url, context, { download });

export const loadThumbnail = (
  context: CourseFolder,
  url: string,
  _limit: number,
) => Promise.resolve(lmsFileUrl(url, context));

/** The URI can be retried by assigning it again after sign-in. */
export const dropThumbnail = (_url: string) => undefined;

/** Calls `retry` when the LMS account turns `normal` again: a sign-in
 * finished, or what stood in its way was cleared. */
export function onSignedIn(retry: () => void) {
  let normal = schoolSessions.get("science")?.state === "normal";
  return schoolSessions.subscribe(() => {
    const next = schoolSessions.get("science")?.state === "normal";
    if (next && !normal) retry();
    normal = next;
  });
}
