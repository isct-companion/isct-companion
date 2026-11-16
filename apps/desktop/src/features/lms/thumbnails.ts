import { lmsFileUrl } from "./files";
import { schoolSessions } from "../../platform/school-session";

/**
 * Images use the same app URI as documents. The backend owns caching,
 * single-flight and its four-lane small-file pool, so the frontend keeps no
 * second scheduler or local/remote URL state; assigning the URI again
 * retries it.
 */
export const thumbnailUrl = (courseId: string, url: string) =>
  lmsFileUrl(url, courseId);

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
