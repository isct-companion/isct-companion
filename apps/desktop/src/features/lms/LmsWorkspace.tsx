import { ExamCards } from "../../components/exams/ExamCards";
import { dueRole } from "../../components/DueChip";
import { Tag } from "../../components/ui";
import { GuideTip } from "../../components/guide";
import { deadlineText } from "../../lib/deadline-text";
import { formatJst } from "../../lib/civil-date";
import { useResizeObserver } from "usehooks-ts";
import { ReaderPane } from "./ReaderPane";
import { ResizableWorkspace } from "../../components/ResizableWorkspace";
import {
  loadReading,
  persistReading,
  type ReadingState,
  type OpenedContent,
} from "./reading-state";
import { useCourseNotes } from "./useCourseNotes";
import { ImageThumbnail, RichContent, type FileAccess } from "./RichContent";
import "./lms.css";
import {
  lazy,
  Suspense,
  useCallback,
  useLayoutEffect,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { contentLanguage } from "../../lib/model";
import type { Assignment, Course, Material, Workspace } from "../../lib/model";
import type { LessonContent, LessonItem } from "../../generated/persistence";
import { t } from "../../i18n";
import { detectPlatform } from "../../platform/capabilities";
import { useClock } from "../../platform/use-clock";
import type { PageProgress } from "../../platform/sync";
import { ageText } from "../../app/sync-text";
import { AssignmentWorkspaceFooter } from "./AssignmentWorkspaceCard";
import { assignmentBrief } from "./assignment-workspace";
import { announcements, formatLmsDate, contentText, safeUrl } from "./content";
import { OfficialLmsView } from "./OfficialLmsView";
import { SyllabusFrame } from "../../pages/courses/syllabus-frame";
import { FileTile, LmsIcon } from "./LmsIcon";
import {
  gravityTarget,
  springScroll,
  type SpringScroll,
} from "./spring-scroll";
import {
  useScheduleQueries,
  useScheduleQuery,
  type AssignmentDetail,
  type CourseContent,
  type CourseDetail,
  type TaskView,
} from "../../platform/core-query";
/** The rail's entry for course activities outside any section. */
const ACTIVITIES = -1;
/** The lesson the course is at, for marking rows of schedule tables. */

import { fileFamily, extension } from "./file-types";
import { withRecords } from "../../platform/records";
import {
  folderComponent,
  downloadLimit,
  previewKind,
  moveNote,
  type LocalFile,
} from "./files";
const FilePopover = lazy(() =>
  import("./FilePreview").then((m) => ({ default: m.FilePopover })),
);
const InlineFolder = lazy(() =>
  import("./FilePreview").then((m) => ({ default: m.InlineFolder })),
);
const FilePreview = lazy(() =>
  import("./FilePreview").then((m) => ({ default: m.FilePreview })),
);
const NoteEditor = lazy(() =>
  import("./NoteEditor").then((m) => ({ default: m.NoteEditor })),
);

export type CourseEntry = {
  term?: string;
  courseId: string;
  request: number;
  date?: string;
  startTime?: string;
  endTime?: string;
  assignmentId?: string;
};
/**
 * A section as Core lays it out (`courseContent`), keyed by its LMS section
 * id (its number in a course without an LMS record) and named by its title.
 */
type Section = Omit<LessonContent, "title"> & { id: number; title: string };

/** Consecutive image-only labels share one grid instead of a block each. */
function groupImageLabels(items: LessonItem[]): (LessonItem | LessonItem[])[] {
  const grouped: (LessonItem | LessonItem[])[] = [];
  for (const item of items) {
    const last = grouped.at(-1);
    if (item.imageOnly && Array.isArray(last)) last.push(item);
    else grouped.push(item.imageOnly ? [item] : item);
  }
  return grouped;
}

/**
 * A folder's files without a request: the listing kept from its last read,
 * else the folder links the crawl stored in its body.
 */
const parsedFolders = new Map<
  string,
  { name: string; url: string }[] | undefined
>();
function knownFolderFiles(m: Material) {
  if (m.folderFiles?.length) return m.folderFiles;
  if (!m.bodyHtml?.includes("mod_folder")) return undefined;
  if (parsedFolders.has(m.bodyHtml)) return parsedFolders.get(m.bodyHtml);
  if (parsedFolders.size > 200) parsedFolders.clear();
  const files = folderLinks(m.bodyHtml);
  parsedFolders.set(m.bodyHtml, files);
  return files;
}
function folderLinks(html: string) {
  const page = new DOMParser().parseFromString(
    html.slice(0, 200000),
    "text/html",
  );
  const files = [
    ...page.querySelectorAll<HTMLAnchorElement>(
      'a[href*="/pluginfile.php/"][href*="/mod_folder/"]',
    ),
  ].flatMap((link) => {
    const url = safeUrl(link.getAttribute("href") ?? "");
    return url && new URL(url).hostname === "lms.s.isct.ac.jp"
      ? [
          {
            name:
              link.textContent?.trim() ||
              decodeURIComponent(url.split("/").pop()?.split("?")[0] ?? ""),
            url,
          },
        ]
      : [];
  });
  return files.length ? files : undefined;
}

export function LmsWorkspaceBody({
  course,
  data,
  entry,
  active,
  schoolWebAvailable,
  close,
  refresh,
  progress,
  updatedAt,
  markLesson,
  navigationProvided = false,
  headerStart,
}: {
  course: Course;
  data: Workspace;
  entry: CourseEntry;
  active: boolean;
  schoolWebAvailable: boolean;
  navigationProvided?: boolean;
  headerStart?: ReactNode;
  close: () => void;
  /** Asks Core to refresh this course now (`Refresh { course }`). */
  refresh?: () => void;
  /** Core's runs for this course; null while nothing reads it. */
  progress?: PageProgress;
  /** When this course's content was last read completely. */
  updatedAt?: string | null;
  /** Records that the class starting at `occurrence` covers this lesson. */
  markLesson?: (sectionId: number, occurrence: string, label: string) => void;
}) {
  const key = `${data.deviceId}:${course.id}`;
  const [saved] = useState(() => loadReading(key));
  const [expanded, setExpanded] = useState<string[]>(saved?.expanded ?? []);
  const [opened, setOpened] = useState<OpenedContent | undefined>(
    saved?.opened,
  );
  const selected = opened?.id;
  const [readingSection, setReadingSection] = useState(saved?.anchor?.id);
  const [query, setQuery] = useState("");
  const [focusLesson, setFocusLesson] = useState<number | undefined>(
    saved?.lesson,
  );
  const [openFolders, setOpenFolders] = useState<string[]>([]);
  const [openDetails, setOpenDetails] = useState<string[]>([]);
  const flip = (list: string[], id: string) =>
    list.includes(id) ? list.filter((item) => item !== id) : [...list, id];
  const [original, setOriginal] = useState(false);
  // Outline and viewer take turns when the workspace pane is narrow. A saved
  // reader restored into a narrow pane starts on the viewer, not hidden.
  const [viewing, setViewing] = useState(Boolean(saved?.opened));
  const outline = useRef<HTMLDivElement>(null);
  const workspaceRef = useRef<HTMLDivElement>(null!);
  const workspaceSize = useResizeObserver({ ref: workspaceRef });
  const narrow =
    (workspaceSize.width ?? 0) > 0 && (workspaceSize.width ?? 0) < 640;
  const viewerActive = active && (!narrow || viewing);
  const folder = useMemo(
    () => ({
      term: folderComponent(course.termId),
      course: folderComponent(course.code || course.id),
      section: "section-0",
    }),
    [course.termId, course.code, course.id],
  );
  const limit = downloadLimit(data.settings);
  const files = useMemo<FileAccess>(
    () => ({ context: folder, limit }),
    [folder, limit],
  );
  /** Assignment workspaces are a desktop feature (folders, VS Code). */
  const workspaces = detectPlatform() === "native-desktop";
  const current = useRef<ReadingState>({
    version: 3,
    expanded,
    opened,
    scroll: saved?.scroll ?? 0,
    anchor: saved?.anchor,
  });
  current.current = {
    ...current.current,
    expanded,
    opened,
    lesson: focusLesson,
  };
  const language =
    data.settings.lmsBilingualFilter && !original
      ? contentLanguage(data.settings)
      : undefined;
  // The course's records by id: what a row shows of its material (file
  // type, folder files, announcements) and the document it opens.
  const materials = useMemo(
    () =>
      new Map(
        data.materials
          .filter((m) => m.courseId === course.id)
          .map((m) => [m.id, m] as const),
      ),
    [data.materials, course.id],
  );
  const courseDetail = useScheduleQuery<CourseDetail>({
    kind: "courseDetail",
    courseId: course.id,
  }).value;
  const syllabus = courseDetail?.syllabusUrl;
  const assignments = useMemo(
    () =>
      data.assignments.filter(
        (a) => a.courseId === course.id && !a.deletedAt && !a.hidden,
      ),
    [data.assignments, course.id],
  );
  const assignmentsById = useMemo(
    () => new Map(assignments.map((a) => [a.id, a] as const)),
    [assignments],
  );
  const assignmentsIn = (ids: string[]) =>
    ids.flatMap((id) => assignmentsById.get(id) ?? []);
  // The LMS text and files of each assignment, as Core reads them.
  const detailState = useScheduleQueries<(AssignmentDetail | null)[]>(
    assignments.map((a) => ({ kind: "assignmentDetail", assignmentId: a.id })),
    assignments.length > 0,
  );
  const details = useMemo(
    () =>
      new Map(
        (detailState.value ?? []).flatMap((detail) =>
          detail ? [[detail.assignmentId, detail] as const] : [],
        ),
      ),
    [detailState.value],
  );
  const timelineNow = useClock(active);
  const contentQuery = useScheduleQuery<CourseContent>({
    kind: "courseContent",
    courseId: course.id,
    now: timelineNow,
  });
  const content = contentQuery.value;
  // Arriving waits for Core's first answer, which says where the course is.
  const contentRead = content !== undefined || contentQuery.error != null;
  // Core lays the course out: its sections in order with their text, items
  // and assignments; the lessons among them with their rail labels, where
  // the course is (the person's mark, else the LMS) and the class a mark is
  // for; and schedule tables marked against the current lesson. One lesson
  // is shown at a time, the rest fold.
  const sections: Section[] = useMemo(
    () =>
      (content?.lessons ?? []).map((lesson) => ({
        ...lesson,
        id: lesson.sectionId ?? lesson.section,
        title: lesson.title || lesson.label,
      })),
    [content],
  );
  const lessons = useMemo(
    () => sections.filter((section) => section.number != null),
    [sections],
  );
  const announcementCards = useMemo(
    () =>
      sections.flatMap((section) =>
        section.items.flatMap((item) => {
          const m = materials.get(item.materialId);
          return item.kind === "forum" && m
            ? announcements(m.bodyHtml).map((post) => ({
                ...post,
                id: `announcement:${m.id}:${post.id}`,
                section: section.title,
              }))
            : [];
        }),
      ),
    [sections, materials],
  );
  const announcement = announcementCards.find((post) => post.id === selected);
  const taskView = useScheduleQuery<TaskView>({
    kind: "taskView",
    now: timelineNow,
  }).value;
  const pendingAssignmentIds = useMemo(
    () => new Set(taskView?.byTerm?.[course.termId]?.pendingIds ?? []),
    [course.termId, taskView],
  );
  const submittedIds = useMemo(
    () => new Set(taskView?.submittedIds),
    [taskView],
  );
  const closedIds = useMemo(() => new Set(taskView?.closedIds), [taskView]);
  const classAt = content?.classAt ?? undefined;
  const at = lessons.find((l) => l.section === content?.currentSection);
  const atIndex = lessons.findIndex((l) => l.id === at?.id);
  // The whole course is laid out at once; lessons behave as pages of it.
  // Sections without a number (一般, 連絡先 …) sit in the flow and are
  // reached from the rail, like Activities outside any section.
  const pinned = useMemo(
    () => sections.filter((s) => !lessons.some((l) => l.id === s.id)),
    [sections, lessons],
  );
  const paged = lessons.length > 0 && !query.trim();
  // The page being read: set by arriving, the rail, a pull, or scrolling.
  const [page, setPage] = useState<number | undefined>(focusLesson);
  const shownId =
    page === ACTIVITIES || sections.some((s) => s.id === page) ? page : at?.id;
  const lessonId = lessons.some((l) => l.id === shownId) ? shownId : undefined;
  const markThisClass = (sectionId: number) => {
    if (!markLesson || !classAt) return;
    // Named by its rail label, not by the teacher's title: a bare number in
    // the UI language, another series (演習 3) as Core labels it.
    const lesson = lessons.find((l) => l.id === sectionId);
    if (!lesson) return;
    markLesson(
      sectionId,
      classAt,
      /^\d+$/.test(lesson.label)
        ? t("Lesson {{number}}", { number: lesson.label })
        : lesson.label,
    );
  };
  const pendingIn = (section: Section) =>
    section.assignmentIds.some((id) => pendingAssignmentIds.has(id));
  const pageElement = (id: number | undefined) =>
    id == null
      ? null
      : (outline.current?.querySelector<HTMLElement>(
          id === ACTIVITIES
            ? '[data-page="activities"]'
            : `[data-page="${id}"]`,
        ) ?? null);
  const goLesson = (id: number) => {
    const host = outline.current;
    const target = pageElement(id);
    setPage(id);
    setFocusLesson(id);
    if (!host || !target) return;
    // The page follows the scroll again once this scroll has arrived.
    const state = pull.current;
    state.turning = target.offsetTop;
    state.motion?.cancel();
    // Only where the spring arrives counts as placed: a jump the person
    // interrupts rests where they stopped it, and gravity pulls as usual.
    state.placed = Math.min(
      target.offsetTop,
      Math.max(0, host.scrollHeight - host.clientHeight),
    );
    state.motion = springScroll(host, target.offsetTop, {
      onDone: () => {
        state.turning = null;
        state.motion = null;
      },
    });
  };
  // Sections attract like gravity wells. Scrolling itself stays native, with
  // its own speed and momentum, so a fast flick can fly over several
  // sections. Where it comes to rest decides: inside a section (the middle
  // of the outline) nothing pulls; with a section boundary near an edge of
  // the outline, the nearer section draws the outline to it, the new one by
  // bringing its start to the top, the old one by bringing its end back to
  // the bottom.
  const pull = useRef({
    turning: null as number | null,
    rest: 0,
    motion: null as SpringScroll | null,
    placed: null as number | null,
  });
  // Where the app itself puts the outline (arriving, revealing an assignment,
  // restoring the reading position) is where the person asked to be, so
  // gravity pulls only an outline the person scrolled. Every programmatic
  // placement of the outline goes through here.
  const placeOutline = (apply: (host: HTMLDivElement) => void) => {
    const host = outline.current;
    if (!host) return;
    const state = pull.current;
    state.motion?.cancel();
    clearTimeout(state.rest);
    apply(host);
    state.placed = host.scrollTop;
  };
  // An assignment already in full view stays where it is, so asking for it
  // again never moves the outline.
  const revealAssignment = (id: string) =>
    placeOutline((host) => {
      const card = host.querySelector<HTMLElement>(
        `[data-assignment="${CSS.escape(id)}"]`,
      );
      if (!card) return;
      const view = host.getBoundingClientRect();
      const box = card.getBoundingClientRect();
      if (box.height > 0 && box.top >= view.top && box.bottom <= view.bottom)
        return;
      card.scrollIntoView?.({
        block: box.height > view.height ? "start" : "center",
      });
    });
  const pageRef = useRef(shownId);
  pageRef.current = shownId;
  const settleInto = (host: HTMLDivElement) => {
    const state = pull.current;
    if (!paged || state.turning != null) return;
    if (state.placed != null && Math.abs(host.scrollTop - state.placed) < 1)
      return;
    state.placed = null;
    const boundaries = [...host.querySelectorAll<HTMLElement>("[data-page]")]
      .slice(1)
      .map((element) => element.offsetTop);
    const target = gravityTarget(
      host.scrollTop,
      host.clientHeight,
      boundaries,
      host.scrollHeight,
    );
    if (target == null) return;
    state.motion?.cancel();
    state.motion = springScroll(host, target, {
      onDone: () => (state.motion = null),
    });
  };
  // Scrolling has come to rest once its events stop (momentum included).
  const restAfterScroll = (host: HTMLDivElement) => {
    const state = pull.current;
    // The app's own spring scrolls do not count as coming to rest.
    if (state.motion) return;
    clearTimeout(state.rest);
    state.rest = window.setTimeout(() => settleInto(host), 90);
  };
  // Scrolling keeps the page in step with what is read:
  // the last page whose top has reached the upper third of the outline.
  const followScroll = (host: HTMLDivElement) => {
    const state = pull.current;
    if (state.turning != null) {
      if (Math.abs(host.scrollTop - state.turning) > 2) return;
      state.turning = null;
    }
    if (!paged) return;
    const line = host.scrollTop + host.clientHeight / 3;
    let found: number | undefined;
    for (const element of host.querySelectorAll<HTMLElement>("[data-page]")) {
      if (element.offsetTop > line) break;
      const key = element.dataset.page!;
      found = key === "activities" ? ACTIVITIES : Number(key);
    }
    if (found != null && found !== pageRef.current) setPage(found);
  };
  // One accent bar slides along the rail to the page being read.
  const rail = useRef<HTMLElement>(null);
  const railIndicator = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    const nav = rail.current;
    const bar = railIndicator.current;
    if (!nav || !bar) return;
    const place = () => {
      const current = nav.querySelector<HTMLElement>("button.current");
      bar.style.opacity = current ? "1" : "0";
      if (!current) return;
      bar.style.transform = `translateX(${current.offsetLeft + 2}px)`;
      bar.style.width = `${Math.max(0, current.offsetWidth - 4)}px`;
      // Keep the current entry in view on a rail that scrolls sideways.
      if (
        current.offsetLeft < nav.scrollLeft ||
        current.offsetLeft + current.offsetWidth >
          nav.scrollLeft + nav.clientWidth
      )
        nav.scrollTo?.({
          left: current.offsetLeft - nav.clientWidth / 2,
          behavior: "smooth",
        });
    };
    place();
    const resize =
      typeof ResizeObserver === "function"
        ? new ResizeObserver(place)
        : undefined;
    resize?.observe(nav);
    return () => resize?.disconnect();
  }, [shownId, paged, lessons, pinned.length]);
  // Assignments outside any section get one rail entry of their own.
  const otherAssignments = assignmentsIn(content?.otherAssignmentIds ?? []);
  const hasActivities = otherAssignments.length > 0;
  const searchable = useMemo(
    () =>
      new Map([
        ...sections.flatMap((s) => [
          [s.summaryHtml, contentText(s.summaryHtml)] as const,
          ...s.items.map(
            (item) =>
              [item.bodyHtml ?? "", contentText(item.bodyHtml ?? "")] as const,
          ),
        ]),
        ...assignments.map((a) => [a.details, contentText(a.details)] as const),
      ]),
    [sections, assignments],
  );
  const matches = (title: string, html?: string) =>
    !query.trim() ||
    `${title} ${html ? searchable.get(html) || contentText(html) : ""}`
      .toLocaleLowerCase()
      .includes(query.trim().toLocaleLowerCase());
  const open = useCallback(
    (content: OpenedContent) => {
      if (outline.current) recordPosition(outline.current);
      setOpened(content);
      setViewing(true);
    },
    [query],
  );
  // Notes are listed again only when the sections change, not with every
  // answer Core gives as the clock moves.
  const noteKey = sections.map((s) => `${s.id}:${s.title}`).join("\n");
  const noteSections = useMemo(
    () => sections.map(({ id, title }) => ({ id, title })),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [noteKey],
  );
  const {
    notes,
    noteError,
    setNoteError,
    deletedNote,
    setDeletedNote,
    creatingNote,
    refreshNotes,
    newNote,
  } = useCourseNotes(folder, noteSections, active, open);
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
    if (
      !url ||
      !anchor ||
      previewKind(url) ||
      !new URL(url).pathname.includes("/pluginfile.php/")
    )
      return false;
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
  const openAssignment = useCallback((a: Assignment) => {
    setViewing(false);
    requestAnimationFrame(() => revealAssignment(a.id));
  }, []);
  const openLink = useCallback(
    (url: string, title: string, anchor?: Element) => {
      const material = data.materials.find(
        (m) => m.url === url || m.resourceUrl === url,
      );
      if (material) return openMaterial(material, anchor);
      const assignment = data.assignments.find((a) => a.sourceUrl === url);
      if (assignment) return openAssignment(assignment);
      if (showFileMenu(url, title, anchor)) return;
      open({ id: `link:${url}`, title, kind: "link", url });
    },
    [data.materials, data.assignments, openMaterial, openAssignment, open],
  );
  const positionedRequest = useRef<number | undefined>(undefined);
  useEffect(() => {
    // Revealing Activity resumes the retained DOM; only a new entrance repositions it.
    if (positionedRequest.current === entry.request || !contentRead) return;
    const assignment = assignments.find((a) => a.id === entry.assignmentId);
    if (assignment) {
      const section = sections.find((s) =>
        s.assignmentIds.includes(assignment.id),
      );
      if (section) {
        const key = `section:${section.id}`;
        setExpanded((previous) =>
          previous.includes(key)
            ? previous.filter((id) => id !== key)
            : previous,
        );
        if (lessons.some((l) => l.id === section.id)) {
          setFocusLesson(section.id);
          setPage(section.id);
        }
      }
      // Arriving at one assignment shows its requirements at once.
      setOpenDetails((list) =>
        list.includes(assignment.id) ? list : [...list, assignment.id],
      );
      setViewing(false);
    }
    // No date→section association exists in the current importer. Resume the
    // real reading position rather than guessing the lesson from the week.
    const frame = requestAnimationFrame(() => {
      positionedRequest.current = entry.request;
      if (assignment) return revealAssignment(assignment.id);
      placeOutline((host) => {
        const anchor = current.current.anchor;
        const element =
          anchor &&
          [
            ...host.querySelectorAll<HTMLElement>("[data-reading-section]"),
          ].find((el) => el.dataset.readingSection === anchor.id);
        if (element && anchor) {
          const panel = host.getBoundingClientRect();
          const rect = element.getBoundingClientRect();
          host.scrollTop +=
            rect.top -
            panel.top +
            rect.height * anchor.ratio -
            panel.height / 2;
        } else if (lessons.length && at) {
          // Arriving at a lesson course opens the page the course is at.
          const page = host.querySelector<HTMLElement>(
            `[data-page="${at.id}"]`,
          );
          host.scrollTop = page?.offsetTop ?? 0;
          setPage(at.id);
        } else host.scrollTop = current.current.scroll;
      });
    });
    return () => cancelAnimationFrame(frame);
    // Explicit entrance only; refreshing content must never jump the reader.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [entry.request, contentRead]);
  const saveReading = () => persistReading(key, data.deviceId, current.current);
  useEffect(() => {
    window.addEventListener("pagehide", saveReading);
    return () => {
      window.removeEventListener("pagehide", saveReading);
      saveReading();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  const recordPosition = (host: HTMLDivElement) => {
    current.current.scroll = host.scrollTop;
    if (query) return;
    const viewport = host.getBoundingClientRect();
    if (viewport.height <= 0) return;
    const center = viewport.top + viewport.height / 2;
    let closest:
      { el: HTMLElement; rect: DOMRect; distance: number } | undefined;
    // Scan once without allocating and sorting a second array on every scroll.
    for (const el of host.querySelectorAll<HTMLElement>(
      "[data-reading-section]",
    )) {
      const rect = el.getBoundingClientRect();
      if (rect.height <= 0) continue;
      const distance =
        center < rect.top
          ? rect.top - center
          : center > rect.bottom
            ? center - rect.bottom
            : 0;
      if (!closest || distance < closest.distance)
        closest = { el, rect, distance };
      if (distance === 0) break;
    }
    if (!closest || !closest.rect.height) return;
    const id = closest.el.dataset.readingSection!;
    current.current.anchor = {
      id,
      ratio: Math.max(
        0,
        Math.min(1, (center - closest.rect.top) / closest.rect.height),
      ),
    };
    setReadingSection(id);
  };
  useLayoutEffect(() => {
    const host = outline.current;
    const anchor = current.current.anchor;
    if (!host || !anchor) return;
    const element = [
      ...host.querySelectorAll<HTMLElement>("[data-reading-section]"),
    ].find((el) => el.dataset.readingSection === anchor.id);
    if (!element) return;
    const panel = host.getBoundingClientRect();
    const rect = element.getBoundingClientRect();
    if (panel.height > 0 && rect.height > 0)
      placeOutline(() => {
        host.scrollTop +=
          rect.top - panel.top + rect.height * anchor.ratio - panel.height / 2;
      });
  }, [Boolean(opened)]);
  const material =
    opened?.kind === "material" ? materials.get(opened.id) : undefined;
  const toggle = (id: string) =>
    setExpanded((previous) =>
      previous.includes(id)
        ? previous.filter((item) => item !== id)
        : [...previous, id],
    );
  const closeContent = () => {
    if (outline.current) recordPosition(outline.current);
    setOpened(undefined);
    setViewing(false);
  };
  /** Assignment card: state and deadline first, the teacher's text on request. */
  const assignmentContent = (a: Assignment) => {
    const detail = details.get(a.id);
    const done = submittedIds.has(a.id);
    const deadline = taskView?.deadlines?.[a.id];
    const due = deadline ? new Date(deadline.at) : undefined;
    const past = deadline?.overdue ?? false;
    const state =
      a.submission === "graded"
        ? "graded"
        : done
          ? "done"
          : past
            ? "missed"
            : "open";
    const countdown =
      deadline && !done && !past
        ? {
            label: deadlineText(deadline, data.settings.locale).countdownLabel,
            tone: deadline.countdownTone,
          }
        : undefined;
    const body = detail?.bodyHtml ?? "";
    const hasBody = Boolean(contentText(body).trim() || a.details?.trim());
    const showBody = openDetails.includes(a.id) || Boolean(query.trim());
    const submit = (preparedUrl?: string) =>
      open({
        id: `official:${a.id}`,
        title: a.title,
        kind: "official",
        url: preparedUrl ?? a.sourceUrl!,
      });
    return (
      <article
        className={`lmsAssignmentCard ${state}`}
        data-assignment={a.id}
        key={a.id}
      >
        <header>
          <span className="lmsAssignmentMark" aria-hidden="true" />
          <div className="lmsAssignmentTitle">
            <h4>
              {hasBody && !query.trim() ? (
                <button
                  type="button"
                  aria-expanded={showBody}
                  onClick={() => setOpenDetails((list) => flip(list, a.id))}
                >
                  {a.title}
                  <LmsIcon
                    name="chevron"
                    size={12}
                    className="lmsAssignmentChevron"
                  />
                </button>
              ) : (
                a.title
              )}
            </h4>
            {due && (
              <time dateTime={deadline?.at}>
                {t("Due")}{" "}
                {formatJst(due, data.settings.locale, {
                  month: "short",
                  day: "numeric",
                  weekday: "short",
                  hour: "2-digit",
                  minute: "2-digit",
                })}
              </time>
            )}
          </div>
          {countdown ? (
            <Tag
              tone={dueRole(countdown.tone)}
              className={`dueChip ${countdown.tone}`}
            >
              {countdown.label}
            </Tag>
          ) : (
            <Tag
              tone={
                state === "graded" || state === "done"
                  ? "ok"
                  : state === "missed"
                    ? "danger"
                    : "neutral"
              }
              className={`lmsAssignmentState ${state}`}
            >
              {t(
                state === "graded"
                  ? "Graded"
                  : state === "done"
                    ? "Submitted"
                    : state === "missed"
                      ? "Not submitted"
                      : "No deadline",
              )}
              {state === "graded" && a.grade && (
                <span className="lmsAssignmentGrade">{a.grade}</span>
              )}
            </Tag>
          )}
        </header>
        {showBody && hasBody && (
          <div className="lmsAssignmentBody">
            {body ? (
              <RichContent
                files={files}
                html={body}
                language={language}
                openLink={openLink}
              />
            ) : (
              <p>{a.details}</p>
            )}
          </div>
        )}
        {workspaces ? (
          <AssignmentWorkspaceFooter
            context={folder}
            assignment={a}
            attachments={detail?.attachments ?? []}
            closed={closedIds.has(a.id)}
            brief={(attachments) =>
              assignmentBrief({
                title: a.title,
                course: course.title,
                due:
                  due &&
                  formatJst(due, data.settings.locale, {
                    dateStyle: "medium",
                    timeStyle: "short",
                  }) + " JST",
                url: a.sourceUrl,
                html: body || a.details || "",
                attachments,
              })
            }
            submit={a.sourceUrl ? submit : undefined}
          />
        ) : (
          <footer>
            {a.sourceUrl && (
              <button
                type="button"
                className={done ? "lmsTextLink" : "lmsSubmit"}
                onClick={() => submit()}
              >
                {t(done ? "View in LMS" : "Submit")}{" "}
                <LmsIcon name="external" size={13} />
              </button>
            )}
          </footer>
        )}
      </article>
    );
  };
  const forumCards = (m: Material) => {
    const posts = announcementCards.filter((post) =>
      post.id.startsWith(`announcement:${m.id}:`),
    );
    return (
      <article className="lmsInlineActivity" key={m.id}>
        <h4>{m.title}</h4>
        {posts.length ? (
          posts.map((post) => (
            <button
              type="button"
              className={`lmsAnnouncementCard${selected === post.id ? " selected" : ""}`}
              key={post.id}
              onClick={() =>
                open({
                  id: post.id,
                  title: post.title,
                  kind: "announcement",
                  url: post.url,
                })
              }
            >
              <span className="lmsCardType">
                {formatLmsDate(post.date, data.settings.locale)}
              </span>
              <strong>{post.title}</strong>
              <span className="lmsCardExcerpt">
                {contentText(post.html)
                  .trim()
                  .replace(/\s+/g, " ")
                  .slice(0, 140)}
              </span>
              <LmsIcon name="chevron" className="lmsCardArrow" />
            </button>
          ))
        ) : (
          <p className="lmsQuiet">
            {t("No announcements yet.")}{" "}
            {safeUrl(m.url) && (
              <button
                type="button"
                className="lmsTextLink"
                onClick={() =>
                  open({
                    id: `official:${m.url}`,
                    title: m.title,
                    kind: "official",
                    url: m.url,
                  })
                }
              >
                {t("Open in LMS")}
              </button>
            )}
          </p>
        )}
      </article>
    );
  };
  const row = (m: Material) => {
    const url = m.resourceUrl || m.url || "";
    const family = fileFamily(m.resourceUrl || "", m.module);
    const ext = extension(m.resourceUrl || "");
    // A folder opens in place: its files list under its row.
    if (m.module === "folder" && safeUrl(m.url)) {
      const isOpen = openFolders.includes(m.id);
      return (
        <div className={`lmsFolder${isOpen ? " open" : ""}`} key={m.id}>
          <button
            type="button"
            className="lmsItemRow"
            aria-expanded={isOpen}
            onClick={() => setOpenFolders((list) => flip(list, m.id))}
          >
            <FileTile family="folder" />
            <span>
              <b>{m.title}</b>
              <small>{t("Folder")}</small>
            </span>
            <LmsIcon name="chevron" className="lmsRowAction lmsFolderChevron" />
          </button>
          {isOpen && (
            <Suspense
              fallback={
                <p className="lmsQuiet lmsFolderNote">{t("Loading…")}</p>
              }
            >
              <InlineFolder
                url={safeUrl(m.url)!}
                context={folder}
                limit={limit}
                known={knownFolderFiles(m)}
                openFile={(fileUrl, name, anchor) =>
                  openLink(fileUrl, name, anchor)
                }
                openInApp={() =>
                  open({
                    id: `official:${m.url}`,
                    title: m.title,
                    kind: "official",
                    url: m.url,
                  })
                }
              />
            </Suspense>
          )}
        </div>
      );
    }
    return (
      <button
        type="button"
        className={`lmsItemRow${selected === m.id ? " selected" : ""}`}
        key={m.id}
        onClick={(event) => openMaterial(m, event.currentTarget)}
      >
        {family === "image" && url.includes("/pluginfile.php/") ? (
          <ImageThumbnail url={url} files={files} />
        ) : (
          <FileTile family={family} />
        )}
        <span>
          <b>{m.title}</b>
          <small>{ext ? ext.toUpperCase() : t(m.module ?? "File")}</small>
        </span>
        <LmsIcon name="chevron" className="lmsRowAction" />
      </button>
    );
  };
  /** One outline section; in a lesson course, lessons are its pages. */
  const sectionView = (section: Section) => {
    const id = `section:${section.id}`;
    const items = section.items;
    if (
      !matches(section.title, section.summaryHtml) &&
      !items.some((item) => matches(item.title, item.bodyHtml ?? undefined))
    )
      return null;
    const lesson = section.number != null ? section : undefined;
    const isOpen = paged || !expanded.includes(id) || Boolean(query);
    return (
      <section
        className={`lmsSection${readingSection === id ? " reading" : ""}${paged && lesson ? " lmsLessonPage" : ""}`}
        data-reading-section={id}
        data-lesson={lesson ? section.id : undefined}
        data-page={paged ? section.id : undefined}
        key={section.id}
      >
        <div className="lmsSectionHeading">
          <h3>
            <button
              type="button"
              aria-expanded={isOpen}
              aria-controls={`lms-panel-${section.id}`}
              onClick={() => (paged ? undefined : toggle(id))}
            >
              <LmsIcon name="chevron" className="lmsSectionChevron" />
              {paged && lesson ? (
                <span className="lmsLessonHead" title={section.title}>
                  <span className="lmsLessonNo">
                    {lesson.label.padStart(2, "0")}
                  </span>
                  <span>{section.summary || section.title}</span>
                </span>
              ) : (
                <span>{section.title}</span>
              )}
              {items.length > 0 && <small>{items.length}</small>}
            </button>
          </h3>
          <button
            type="button"
            className="lmsAddNote"
            disabled={creatingNote}
            aria-label={`${t("New note")} · ${section.title}`}
            title={t("New note")}
            onClick={() => void newNote(section)}
          >
            <LmsIcon name="plus" />
          </button>
        </div>
        {!isOpen && section.summaryHtml && (
          <p className="lmsSectionPreview">{t("Section collapsed")}</p>
        )}
        {isOpen && (
          <div id={`lms-panel-${section.id}`} className="lmsSectionBody">
            {section.summaryHtml && (
              <RichContent
                files={files}
                html={section.summaryHtml}
                language={language}
                openLink={openLink}
              />
            )}
            {groupImageLabels(
              items.filter(
                (item) =>
                  !query ||
                  matches(item.title, item.bodyHtml ?? undefined) ||
                  matches(section.title, section.summaryHtml),
              ),
            ).map((item) => {
              if (Array.isArray(item))
                return (
                  <div className="lmsImageGrid" key={item[0].materialId}>
                    {item.map((label) => (
                      <RichContent
                        key={label.materialId}
                        files={files}
                        html={label.bodyHtml ?? ""}
                        language={language}
                        openLink={openLink}
                      />
                    ))}
                  </div>
                );
              const m = materials.get(item.materialId);
              if (item.kind === "forum") return m ? forumCards(m) : null;
              if (
                item.bodyHtml &&
                (item.kind === "page" || item.kind === "label")
              )
                return (
                  <article className="lmsInlineActivity" key={item.materialId}>
                    {/* Moodle names a label after its own text, so only pages get a heading. */}
                    {item.kind === "page" && <h4>{item.title}</h4>}
                    <RichContent
                      files={files}
                      html={item.bodyHtml}
                      language={language}
                      openLink={openLink}
                    />
                  </article>
                );
              return m ? row(m) : null;
            })}
            {assignmentsIn(section.assignmentIds).map(assignmentContent)}
            {(() => {
              const sectionNotes = notes.filter(
                (note) => note.section === `section-${section.id}`,
              );
              const row = (note: LocalFile) => (
                <button
                  type="button"
                  className="lmsItemRow"
                  key={note.id}
                  onClick={() =>
                    open({
                      id: note.id,
                      title:
                        note.name.trim() || `${section.title} · ${t("Note")}`,
                      kind: "note",
                      section: note.section,
                    })
                  }
                >
                  <FileTile family="note" />
                  <span>
                    <b>
                      {note.name.trim() || `${section.title} · ${t("Note")}`}
                    </b>
                    <small>
                      {t("Local note · stored only on this device")}
                    </small>
                  </span>
                  <LmsIcon name="chevron" className="lmsRowAction" />
                </button>
              );
              return (
                <>
                  {sectionNotes[0] && row(sectionNotes[0])}
                  {sectionNotes.length > 1 && (
                    <details className="lmsPreviousNotes">
                      <summary>
                        {t("Previous notes")} · {sectionNotes.length - 1}
                      </summary>
                      {sectionNotes.slice(1).map(row)}
                    </details>
                  )}
                </>
              );
            })()}
            {!section.summaryHtml && !items.length && (
              <p className="lmsQuiet">{t("No content in this section")}</p>
            )}
          </div>
        )}
      </section>
    );
  };
  return (
    <div
      className={`lmsWorkspace${narrow ? " narrowReader" : ""}${opened ? " hasViewer" : ""}${viewing ? " viewing" : ""}`}
    >
      <header className="lmsWorkspaceHeader">
        {headerStart}
        {!navigationProvided && (
          <button type="button" className="lmsBack" onClick={close}>
            <LmsIcon name="back" /> {t("Today")}
          </button>
        )}
        <div className="lmsCourseIdentity">
          <h1>{course.title}</h1>
          <small>{course.code}</small>
        </div>
        {entry.date && (
          <span className="lmsEntryTime">
            {entry.date}{" "}
            {entry.startTime && `· ${entry.startTime}–${entry.endTime}`}
          </span>
        )}
        {syllabus && (
          <button
            type="button"
            className="lmsRefresh"
            aria-pressed={selected === "syllabus"}
            onClick={() =>
              open({
                id: "syllabus",
                title: t("Syllabus"),
                kind: "syllabus",
                url: syllabus,
              })
            }
          >
            <LmsIcon name="doc" size={14} /> {t("Syllabus")}
          </button>
        )}
        {(progress != null || refresh) && (
          <div className="lmsPullControls">
            {progress != null && (
              // Local data stays on screen; the whole course switches when
              // Core commits it.
              <span className="syncPageStatus" role="status">
                <i className="schoolStatusSpinner" aria-hidden="true" />
                {updatedAt
                  ? `${t("Showing data from {{age}}", {
                      age: ageText(
                        updatedAt,
                        timelineNow,
                        data.settings.locale,
                      ),
                    })} · `
                  : null}
                {progress.total !== null
                  ? t("updating this course {{done}} / {{total}}", {
                      done: progress.done,
                      total: progress.total,
                    })
                  : t("updating this course")}
              </span>
            )}
            {refresh && (
              <button
                type="button"
                className="lmsRefresh"
                disabled={progress != null || opened?.kind === "official"}
                onClick={refresh}
              >
                {t("Pull from LMS")}
              </button>
            )}
          </div>
        )}
      </header>
      <div className="lmsWorkspaceTools">
        {lessons.length > 0 && !query.trim() && (
          <GuideTip
            id="lesson-mark"
            text="Double-click a lesson number to mark it as the lesson of today's class."
          >
            <nav className="lmsLessonRail" aria-label={t("Lessons")} ref={rail}>
              <span
                className="lmsRailIndicator"
                ref={railIndicator}
                aria-hidden="true"
              />
              {pinned.map((section) => (
                <button
                  type="button"
                  key={section.id}
                  className={`lmsRailTab${paged && shownId === section.id ? " current" : ""}`}
                  aria-current={
                    paged && shownId === section.id ? "step" : undefined
                  }
                  title={section.title}
                  onClick={() => goLesson(section.id)}
                >
                  {section.title}
                </button>
              ))}
              {hasActivities && (
                <button
                  type="button"
                  className={`lmsRailTab${paged && shownId === ACTIVITIES ? " current" : ""}`}
                  aria-current={
                    paged && shownId === ACTIVITIES ? "step" : undefined
                  }
                  onClick={() => goLesson(ACTIVITIES)}
                >
                  {t("Activities")}
                </button>
              )}
              {(pinned.length > 0 || hasActivities) && (
                <span className="lmsRailDivider" aria-hidden="true" />
              )}
              {lessons.map((l, index) => (
                <button
                  type="button"
                  key={l.id}
                  className={`${atIndex >= 0 && index <= atIndex ? "held" : ""}${l.id === lessonId && paged ? " current" : ""}${pendingIn(l) ? " due" : ""}`}
                  aria-current={l.id === lessonId && paged ? "step" : undefined}
                  title={l.title}
                  onClick={() => goLesson(l.id)}
                  onDoubleClick={() => markThisClass(l.id)}
                >
                  {l.label}
                </button>
              ))}
            </nav>
          </GuideTip>
        )}
      </div>
      <div className="lmsWorkspaceBody" ref={workspaceRef}>
        <ResizableWorkspace
          id={`reading-${course.id}`}
          enabled={!narrow && Boolean(opened)}
          firstSize="44%"
          firstMin={220}
          secondMin={260}
        >
          <aside className="lmsOutline">
            <div className="lmsOutlineTools">
              <input
                aria-label={t("Search course content")}
                placeholder={t("Search this course…")}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
              {lessons.length ? null : (
                <button
                  type="button"
                  onClick={() =>
                    setExpanded(
                      expanded.length
                        ? []
                        : sections.map((s) => `section:${s.id}`),
                    )
                  }
                >
                  {expanded.length ? t("Expand all") : t("Collapse all")}
                </button>
              )}
            </div>

            <div
              className={`lmsOutlineScroll${query ? " searching" : ""}`}
              ref={outline}
              onScroll={(event) => {
                recordPosition(event.currentTarget);
                followScroll(event.currentTarget);
                restAfterScroll(event.currentTarget);
              }}
            >
              {fileMenu && (
                <Suspense fallback={null}>
                  <FilePopover
                    key={fileMenu.url}
                    url={fileMenu.url}
                    title={fileMenu.title}
                    context={folder}
                    limit={limit}
                    anchor={fileMenu.anchor}
                    onClose={closeFileMenu}
                    openInApp={() => {
                      closeFileMenu();
                      open({
                        id: `official:${fileMenu.url}`,
                        title: fileMenu.title,
                        kind: "official",
                        url: fileMenu.url,
                      });
                    }}
                  />
                </Suspense>
              )}
              {noteError && <p role="alert">{noteError}</p>}
              {deletedNote && (
                <div className="lmsNoteUndo" role="status">
                  <span>{t("Note deleted")}</span>
                  <button
                    type="button"
                    onClick={() => {
                      const note = deletedNote;
                      void moveNote(
                        { ...folder, section: note.section },
                        note.id,
                        true,
                      )
                        .then(() => {
                          setDeletedNote(null);
                          refreshNotes();
                          open({ ...note, kind: "note" });
                        })
                        .catch((error) => setNoteError(String(error)));
                    }}
                  >
                    {t("Undo")}
                  </button>
                </div>
              )}
              {(courseDetail?.exams?.length ?? 0) > 0 && (
                <section className="lmsCourseExams" aria-label={t("Exams")}>
                  <h2>{t("Exams")}</h2>
                  <ExamCards
                    exams={courseDetail?.exams ?? []}
                    courses={data.courses.filter((row) => row.id === course.id)}
                  />
                </section>
              )}
              {!paged && <h2>{t("Course content")}</h2>}
              {sections.map((section) => sectionView(section))}
              {!sections.length && (
                <p className="lmsQuiet">
                  {t(
                    "No course content synced yet. Refresh LMS to load the outline.",
                  )}
                </p>
              )}
              <div data-page={paged ? "activities" : undefined}>
                {/* Only assignments outside any section; the rest sit in their lesson. */}
                {(() => {
                  const loose = otherAssignments.filter((a) =>
                    matches(a.title, a.details),
                  );
                  return loose.length ? (
                    <section
                      data-reading-section="assignments"
                      className={`lmsActivityGroup${readingSection === "assignments" ? " reading" : ""}`}
                    >
                      <h2>{t("Assignments")}</h2>
                      {loose.map(assignmentContent)}
                    </section>
                  ) : null;
                })()}
              </div>
            </div>
            {data.settings.lmsBilingualFilter && (
              <button
                type="button"
                className="lmsOriginal"
                onClick={() => setOriginal((value) => !value)}
              >
                {original ? t("Use preferred language") : t("Show original")}
              </button>
            )}
          </aside>
          {opened && (
            <ReaderPane title={opened.title} close={closeContent}>
              {/* Hidden readers release their documents; the official view keeps its webview. */}
              {!viewerActive &&
              opened.kind !== "official" ? null : opened.kind ===
                "announcement" ? (
                <div className="lmsDocument">
                  <div className="lmsDocumentHeading">
                    <small>
                      {formatLmsDate(
                        announcement?.date || "",
                        data.settings.locale,
                      )}
                    </small>
                    <h2>{opened.title}</h2>
                  </div>
                  {announcement && (
                    <RichContent
                      files={files}
                      html={announcement.html}
                      language={language}
                      openLink={openLink}
                    />
                  )}
                </div>
              ) : opened.kind === "syllabus" ? (
                <div className="lmsDocument lmsSyllabus">
                  <SyllabusFrame url={opened.url} />
                </div>
              ) : opened.kind === "note" ? (
                <Suspense fallback={<p>{t("Loading editor…")}</p>}>
                  <NoteEditor
                    key={opened.id}
                    id={opened.id}
                    context={{ ...folder, section: opened.section! }}
                    onSaved={refreshNotes}
                    onDeleted={() => {
                      setDeletedNote({
                        id: opened.id,
                        section: opened.section!,
                        title: opened.title,
                      });
                      closeContent();
                      refreshNotes();
                    }}
                  />
                </Suspense>
              ) : opened.kind === "official" ? (
                <>
                  <OfficialLmsView
                    url={opened.url!}
                    active={viewerActive}
                    available={schoolWebAvailable}
                  />
                </>
              ) : (
                <div className="lmsDocument lmsFileDocument">
                  <div className="lmsDocumentHeading">
                    <small>{material?.sectionTitle || course.code}</small>
                    <h2>{opened.title}</h2>
                  </div>
                  {material?.bodyHtml && (
                    <RichContent
                      files={files}
                      html={material.bodyHtml}
                      language={language}
                      openLink={openLink}
                    />
                  )}
                  {(() => {
                    const url = safeUrl(
                      material?.resourceUrl ||
                        (material?.url &&
                        ["resource", "url", "folder"].includes(
                          material.module ?? "",
                        )
                          ? material.url
                          : undefined) ||
                        (opened.kind === "link" ? opened.url : undefined),
                    );
                    if (!url) {
                      // A forum read without posts is empty, not unsynced; any
                      // unresolved activity can still open on the official page.
                      const official = safeUrl(material?.url);
                      return material && !material.bodyHtml ? (
                        <>
                          <p className="lmsQuiet">
                            {t(
                              material.module === "forum"
                                ? "No announcements yet."
                                : "No content in this section",
                            )}
                          </p>
                          {official &&
                            new URL(official).hostname ===
                              "lms.s.isct.ac.jp" && (
                              <button
                                type="button"
                                className="lmsRefresh"
                                onClick={() =>
                                  open({
                                    id: `official:${official}`,
                                    title: opened.title,
                                    kind: "official",
                                    url: official,
                                  })
                                }
                              >
                                {t("Open in LMS")}
                              </button>
                            )}
                        </>
                      ) : null;
                    }
                    return (
                      <Suspense fallback={<p>{t("Loading preview…")}</p>}>
                        <FilePreview
                          key={url}
                          url={url}
                          title={opened.title}
                          context={folder}
                          limit={limit}
                          openInApp={
                            new URL(url).hostname === "lms.s.isct.ac.jp"
                              ? () =>
                                  open({
                                    id: `official:${url}`,
                                    title: opened.title,
                                    kind: "official",
                                    url,
                                  })
                              : undefined
                          }
                          openFile={(fileUrl, name) =>
                            open({
                              id: `link:${fileUrl}`,
                              title: name,
                              kind: "link",
                              url: fileUrl,
                            })
                          }
                        />
                      </Suspense>
                    );
                  })()}
                </div>
              )}
            </ReaderPane>
          )}
        </ResizableWorkspace>
      </div>
    </div>
  );
}

/** LmsWorkspace with the records it shows (`platform/records`). */
export const LmsWorkspace = withRecords(
  [
    "assignments",
    "courses",
    "materials",
    "memberships",
    "planItems",
    "academicProfile",
    "lmsCourses",
  ],
  LmsWorkspaceBody,
);
