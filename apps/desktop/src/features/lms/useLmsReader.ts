import { type OpenedContent } from "./reading-state";
import { useCallback, useState } from "react";
import type { Assignment, Material, Workspace } from "../../lib/model";
import { safeUrl } from "./content";
import { downloadOnly } from "./file-types";
import { type SchoolPage } from "./files";
import type { Dispatch, RefObject, SetStateAction } from "react";

import type { CourseContent } from "../../platform/core-query";
import type { Section } from "./useOutlineReading";

export function useLmsReader({
  data,
  query,
  outline,
  recordPosition,
  setOpened,
  setViewing,
  revealAssignment,
  openWeb,
  setArchiveItems,
  setOpenFolders,
  opened,
  materials,
  content,
  sections,
  original,
}: {
  data: Pick<Workspace, "materials" | "assignments">;
  query: string;
  outline: RefObject<HTMLDivElement | null>;
  recordPosition: (host: HTMLDivElement) => void;
  setOpened: Dispatch<SetStateAction<OpenedContent | undefined>>;
  setViewing: Dispatch<SetStateAction<boolean>>;
  revealAssignment: (id: string) => void;
  openWeb?: (url: string) => void;
  setArchiveItems: Dispatch<SetStateAction<string[]>>;
  setOpenFolders: Dispatch<SetStateAction<string[]>>;
  opened?: OpenedContent;
  materials: Map<string, Material>;
  content?: CourseContent;
  sections: Section[];
  original: boolean;
}) {
  const open = useCallback(
    (content: OpenedContent) => {
      if (outline.current) recordPosition(outline.current);
      setOpened(content);
      setViewing(true);
    },
    [query],
  );
  const [fileMenu, setFileMenu] = useState<{
    url: string;
    title: string;
    anchor: Element;
  }>();
  const closeFileMenu = useCallback(() => setFileMenu(undefined), []);
  /**
   * Files the app cannot preview open as a popover at the clicked item, so the
   * reading pane is only used for content. Returns false for previewable ones.
   */
  const showFileMenu = (
    url: string | undefined,
    title: string,
    anchor?: Element,
  ) => {
    if (!url || !anchor || !downloadOnly(url)) return false;
    setFileMenu({ url, title, anchor });
    return true;
  };
  const openMaterial = useCallback(
    (m: Material, anchor?: Element) => {
      if (showFileMenu(safeUrl(m.resourceUrl), m.title, anchor)) return;
      open({ id: m.id, title: m.title, kind: "material" });
    },
    [open],
  );
  /**
   * A school page Core named: the LMS in the reader's signed-in view, the
   * syllabus in its frame, any other school site on the Web page (the reader
   * lets the item go, so coming back does not open it there again).
   */
  const openPage = ({ site, url }: SchoolPage, title: string) => {
    if (site === "science-lms")
      open({ id: `official:${url}`, title, kind: "official", url });
    else if (site === "syllabus")
      open({ id: `syllabus:${url}`, title, kind: "syllabus", url });
    else {
      closeContent();
      openWeb?.(url);
    }
  };
  /**
   * A page outside the school sites, in the reader's web view: the LMS view,
   * where a link out of an LMS page lands as well.
   */
  const openWebPage = (url: string, title: string) =>
    open({ id: `official:${url}`, title, kind: "official", url });
  const openAssignment = useCallback((a: Assignment) => {
    setViewing(false);
    requestAnimationFrame(() => revealAssignment(a.id));
  }, []);
  /** The course item a link points at, by its page or its file. */
  const materialAt = useCallback(
    (url: string) =>
      data.materials.find((m) => m.url === url || m.resourceUrl === url),
    [data.materials],
  );
  const openLink = useCallback(
    (url: string, title: string, anchor?: Element) => {
      const material = materialAt(url);
      if (material) return openMaterial(material, anchor);
      const assignment = data.assignments.find((a) => a.sourceUrl === url);
      if (assignment) return openAssignment(assignment);
      if (showFileMenu(url, title, anchor)) return;
      open({ id: `link:${url}`, title, kind: "link", url });
    },
    [materialAt, data.assignments, openMaterial, openAssignment, open],
  );
  /**
   * Core answered a ZIP for an item the reader opened: its row becomes a
   * folder and opens, and the reader closes. False for anything without a
   * row (a link), which the reader lists itself.
   */
  const archiveInOutline = (item: { id: string; kind: string }) => {
    if (item.kind === "material") {
      setArchiveItems((list) =>
        list.includes(item.id) ? list : [...list, item.id],
      );
      setOpenFolders((list) =>
        list.includes(item.id) ? list : [...list, item.id],
      );
      closeContent();
      return true;
    }
    return false;
  };
  const closeContent = () => {
    if (outline.current) recordPosition(outline.current);
    setOpened(undefined);
    setViewing(false);
  };

  const material =
    opened?.kind === "material" ? materials.get(opened.id) : undefined;
  const readerHtml = material
    ? content?.materialReaderHtml?.[material.id]
    : undefined;
  const materialBody = readerHtml
    ? original
      ? (readerHtml.original ?? readerHtml.preferred)
      : readerHtml.preferred
    : sections
        .flatMap((section) => section.items)
        .find((item) => item.materialId === material?.id)?.bodyHtml;
  return {
    material,
    materialBody,
    open,
    openMaterial,
    openPage,
    openWebPage,
    materialAt,
    openLink,
    archiveInOutline,
    closeContent,
    fileMenu,
    closeFileMenu,
  };
}
