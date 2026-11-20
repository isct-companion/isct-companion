import { useEffect, useRef, useState } from "react";
import type { MessageKey } from "../../i18n";
import type { Viewer } from "../../generated/persistence";
import { openMaterial, type LmsFileReason, type Material, type SchoolPage } from "./files";

type View =
  | { state: "loading" }
  | { state: "missing"; page: SchoolPage }
  | {
      state: "ready";
      path: string;
      name: string;
      /** How the reader shows it (Core's `file_preview`). */
      viewer: Viewer;
      text?: string;
      page: SchoolPage;
    }
  | { state: "folder"; files: { name: string; url: string }[] }
  | Extract<Material, { state: "archive" }>
  | ({ state: "page" } & SchoolPage)
  | { state: "external"; url: string }
  | { state: "tooLarge"; bytes: number; page: SchoolPage }
  | {
      state: "failed";
      reason: LmsFileReason;
      page: SchoolPage | null;
      note?: MessageKey;
    };
export type CourseFile = ReturnType<typeof useCourseFile>;
/** Shows a school page: the LMS in the reader, another site where it lives. */
export type OpenPage = (page: SchoolPage) => void;

/** The LMS page Core named for the item, if any. */
export const pageOf = (view: View): SchoolPage | null =>
  "page" in view ? view.page : null;

/**
 * What an item or link opens as, as Core decides (`open_material`), and a
 * file's state on disk. With `download`, a missing file within `limit` bytes
 * is downloaded at once; `fetchNow` downloads it at any size.
 */
export function useCourseFile(
  url: string,
  courseId: string,
  limit: number,
  { download, readText = false }: { download: boolean; readText?: boolean },
) {
  const [view, setView] = useState<View>({ state: "loading" });
  const [request, setRequest] = useState({ download: false, attempt: 0 });
  const loadedRequest = useRef<string | undefined>(undefined);
  useEffect(() => {
    const requestKey = JSON.stringify([
      url,
      courseId,
      request,
      download,
      readText,
    ]);
    if (loadedRequest.current === requestKey) return;
    let disposed = false;
    setView({ state: "loading" });
    const load = async () => {
      try {
        const material = await openMaterial(courseId, url, {
          download: download || request.download,
          readText,
          limit: request.download ? undefined : limit,
        });
        if (disposed) return;
        if (material.state !== "failed") loadedRequest.current = requestKey;
        if (material.state === "ready") {
          return setView({
            state: "ready",
            path: material.file.path,
            name: material.file.name || url,
            viewer: material.file.viewer,
            text: material.file.text ?? undefined,
            page: material.page,
          });
        }
        setView(material);
      } catch {
        // A command error is not the item's: Core refused the request.
        if (!disposed)
          setView({ state: "failed", reason: "invalid", page: null });
      }
    };
    void load();
    return () => {
      disposed = true;
    };
  }, [url, courseId, request, download, readText]);
  return {
    url,
    courseId,
    limit,
    view,
    setView,
    retry: () =>
      setRequest((value) => ({ ...value, attempt: value.attempt + 1 })),
    fetchNow: () =>
      setRequest((value) => ({ download: true, attempt: value.attempt + 1 })),
  };
}
