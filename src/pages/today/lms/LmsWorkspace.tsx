import "./lms.css";
import {
  createContext,
  lazy,
  memo,
  Suspense,
  useCallback,
  useLayoutEffect,
  useEffect,
  useMemo,
  useRef,
  useContext,
  useState,
  type WheelEvent,
} from "react";
import { contentLanguage } from "../../../core/model";
import type {
  Assignment,
  Course,
  Material,
  Workspace,
} from "../../../core/model";
import type {
  LmsCourseRecord,
  LmsSection,
} from "../../../platform/desktop-bridge";
import { t } from "../../../i18n";
import { desktopBridge } from "../../../app/native";
import { announcements, formatLmsDate, contentText, imageOnly, safeContent, safeUrl } from "./content";
import { OfficialLmsView } from "./OfficialLmsView";
import { FileTile, LmsIcon } from "./LmsIcon";
import { dueCountdown } from "../../../core/academic";
import { springScroll, type SpringScroll } from "./spring-scroll";
import { courseLessons, frontierLesson, gravityTarget, lessonSummary, markedLesson, markSchedule, railLabels } from "./lessons";
import { addDays, resolveDayAgenda, tokyoDate } from "../../../core/schedule";
import { jstMinutes } from "../../../components/timetable/timetable";
/** The rail's entry for course activities outside any section. */
const ACTIVITIES = -1;
/** The lesson the course is at, for marking rows of schedule tables. */
const CurrentLesson = createContext<number | undefined>(undefined);
import { fileFamily, extension } from "./file-types";
import { cachedThumbnail, dropThumbnail, loadThumbnail, onSignedIn } from "./thumbnails";
import {
  type CourseFolder,
  folderComponent,
  downloadLimit,
  previewKind,
  listNotes,
  saveNote,
  moveNote,
  type LocalFile,
} from "./files";
const FilePopover = lazy(() => import("./FilePreview").then((m) => ({ default: m.FilePopover })));
const InlineFolder = lazy(() => import("./FilePreview").then((m) => ({ default: m.InlineFolder })));
const FilePreview = lazy(() =>
  import("./FilePreview").then((m) => ({ default: m.FilePreview })),
);
const NoteEditor = lazy(() =>
  import("./NoteEditor").then((m) => ({ default: m.NoteEditor })),
);

export type CourseEntry = {
  courseId: string;
  request: number;
  date?: string;
  startTime?: string;
  endTime?: string;
  assignmentId?: string;
};
const SyllabusDetails = lazy(() => import("../../courses/syllabus-details").then(m => ({ default: m.SyllabusDetails })));
const isSyllabus = (m: Material) => /^(シラバス|syllabus)$/i.test(m.title.trim()) || Boolean(m.resourceUrl?.startsWith("https://syllabus.s.isct.ac.jp/"));
type ViewTab = {
  id: string;
  title: string;
  kind: "material" | "official" | "link" | "note" | "syllabus" | "announcement";
  section?: string;
  url?: string;
};
type ReadingState = {
  version: 2;
  expanded: string[];
  scroll: number;
  tabs: ViewTab[];
  selected?: string;
  anchor?: { id: string; ratio: number };
  /** The lesson shown alone in the outline, by section id. */
  lesson?: number;
};
const reading = new Map<string, ReadingState>();
/** Where course files live and how large an automatic download may be. */
type FileAccess = { context: CourseFolder; limit: number };
/** Consecutive image-only labels share one grid instead of a block each. */
function groupImageLabels(items: Material[]): (Material | Material[])[] {
  const grouped: (Material | Material[])[] = [];
  for (const item of items) {
    const tile = item.module === "label" && imageOnly(item.bodyHtml);
    const last = grouped.at(-1);
    if (tile && Array.isArray(last)) last.push(item);
    else grouped.push(tile ? [item] : item);
  }
  return grouped;
}
const groupFor = (module?: string) =>
  module === "forum"
    ? "Discussions"
    : module === "url"
      ? "Links"
      : [undefined, "resource", "folder", "label", "page"].includes(module)
        ? "Course content"
        : "Other activities";

/**
 * A folder's files without a request: the listing kept from its last read,
 * else the folder links the crawl stored in its body.
 */
const parsedFolders = new Map<string, { name: string; url: string }[] | undefined>();
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
  const page = new DOMParser().parseFromString(html.slice(0, 200000), "text/html");
  const files = [...page.querySelectorAll<HTMLAnchorElement>('a[href*="/pluginfile.php/"][href*="/mod_folder/"]')].flatMap((link) => {
    const url = safeUrl(link.getAttribute("href") ?? "");
    return url && new URL(url).hostname === "lms.s.isct.ac.jp"
      ? [{ name: link.textContent?.trim() || decodeURIComponent(url.split("/").pop()?.split("?")[0] ?? ""), url }]
      : [];
  });
  return files.length ? files : undefined;
}

/** Moodle repeats an assignment's open/due dates in its body; the card already shows them. */
const strippedBodies = new Map<string, string>();
function withoutActivityDates(html: string) {
  if (!/activity-dates|activity-information/.test(html)) return html;
  const known = strippedBodies.get(html);
  if (known != null) return known;
  if (strippedBodies.size > 200) strippedBodies.clear();
  const page = new DOMParser().parseFromString(html, "text/html");
  page.querySelectorAll('[data-region="activity-dates"], .activity-dates, [data-region="activity-information"]').forEach((node) => node.remove());
  strippedBodies.set(html, page.body.innerHTML);
  return page.body.innerHTML;
}

const ImageThumbnail = memo(function ImageThumbnail({ url, files }: { url: string; files: FileAccess }) {
  const element = useRef<HTMLSpanElement>(null);
  const [src, setSrc] = useState<string | undefined>(() =>
    cachedThumbnail(files.context, url),
  );
  const [loaded, setLoaded] = useState(false);
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  useEffect(
    () =>
      onSignedIn(() => {
        setFailed(false);
        setAttempt((value) => value + 1);
      }),
    [],
  );
  useEffect(() => {
    if (src || failed) return;
    let disposed = false;
    const load = () => void loadThumbnail(files.context, url, files.limit).then(value => { if (!disposed && value) setSrc(value); }).catch(() => undefined);
    const observer = typeof IntersectionObserver === "undefined" ? undefined : new IntersectionObserver(entries => {
      if (entries.some(entry => entry.isIntersecting)) { observer!.disconnect(); load(); }
    }, { rootMargin: "200px" });
    if (observer && element.current) observer.observe(element.current); else load();
    return () => { disposed = true; observer?.disconnect(); };
  }, [url, src, failed, attempt, files]);
  return <span ref={element} className={`lmsFileTile lmsThumbnail${loaded ? " loaded" : ""}`} data-family="image" aria-hidden="true">
    <LmsIcon name="image" size={18} />
    {src && <img src={src} alt="" decoding="async" onLoad={() => setLoaded(true)} onError={() => { dropThumbnail(url); setSrc(undefined); setLoaded(false); setFailed(true); }} />}
  </span>;
});

const RichContent = memo(function RichContent({
  html,
  language,
  openLink,
  files,
}: {
  files: FileAccess;
  html: string;
  language?: "ja" | "en";
  openLink: (url: string, title: string, anchor?: Element) => void;
}) {
  const current = useContext(CurrentLesson);
  const content = useMemo(() => markSchedule(safeContent(html, language), current), [html, language, current]);
  const renderedHtml = useMemo(() => ({ __html: content }), [content]);
  const element = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let disposed = false;
    const load = async (placeholder: HTMLElement) => {
      const url = placeholder.dataset.imageSrc!;
      placeholder.dataset.state = "loading";
      try {
        const src = await loadThumbnail(files.context, url, files.limit);
        if (disposed) return;
        if (!src) { placeholder.dataset.state = "unavailable"; return; }
        const image = document.createElement("img");
        image.alt = placeholder.dataset.imageAlt || "";
        image.decoding = "async"; image.className = "lmsInlineThumbnail";
        image.onload = () => { placeholder.dataset.state = "ready"; };
        image.onerror = () => { dropThumbnail(url); image.remove(); placeholder.dataset.state = "unavailable"; };
        image.src = src;
        placeholder.replaceChildren(image);
      } catch (error) { if (!disposed) { placeholder.dataset.state = "unavailable"; placeholder.title = String(error); } }
    };
    const placeholders = element.current?.querySelectorAll<HTMLElement>("[data-image-src]") || [];
    const observer = typeof IntersectionObserver === "undefined" ? null : new IntersectionObserver(entries => {
      for (const entry of entries) if (entry.isIntersecting) { observer!.unobserve(entry.target); void load(entry.target as HTMLElement); }
    }, {rootMargin:"80px"});
    for (const placeholder of placeholders) { if (observer) observer.observe(placeholder); else void load(placeholder); }
    const stop = onSignedIn(() => {
      for (const placeholder of placeholders) if (placeholder.dataset.state === "unavailable") void load(placeholder);
    });
    return () => { disposed = true; observer?.disconnect(); stop(); };
  }, [content, files]);
  return (
    <div
      ref={element}
      className="lmsRichContent"
      dangerouslySetInnerHTML={renderedHtml}
      onClick={(event) => {
        const link = (event.target as Element).closest("a");
        if (!link) return;
        event.preventDefault();
        const url = safeUrl(link.getAttribute("href"));
        const name = link.querySelector<HTMLElement>("[data-image-alt]")?.dataset.imageAlt || link.querySelector(".lmsAttachmentName")?.textContent;
        if (url) openLink(url, name || link.textContent || t("Link"), link);
      }}
    />
  );
});

export function LmsWorkspace({
  course,
  data,
  record,
  entry,
  active,
  schoolWebAvailable,
  close,
  refresh,
  progress,
  markLesson,
  navigationProvided = false,
}: {
  course: Course;
  data: Workspace;
  record?: LmsCourseRecord;
  entry: CourseEntry;
  active: boolean;
  schoolWebAvailable: boolean;
  navigationProvided?: boolean;
  close: () => void;
  refresh?: () => void;
  progress?: number | null;
  /** Records that the class starting at `occurrence` covers this lesson. */
  markLesson?: (sectionId: number, occurrence: string, label: string) => void;
}) {
  const key = `${data.deviceId}:${course.id}`;
  const [saved] = useState(
    () =>
      reading.get(key) ??
      (() => {
        try {
          const value = JSON.parse(
            sessionStorage.getItem(`lms-reading:${key}`) || "null",
          );
          return value &&
            value.version === 2 &&
            Array.isArray(value.expanded) &&
            Array.isArray(value.tabs)
            ? (value as ReadingState)
            : undefined;
        } catch {
          return undefined;
        }
      })(),
  );
  const [expanded, setExpanded] = useState<string[]>(saved?.expanded ?? []);
  const [tabs, setTabs] = useState<ViewTab[]>(saved?.tabs ?? []);
  const [selected, setSelected] = useState(saved?.selected);
  const [readingSection, setReadingSection] = useState(saved?.anchor?.id);
  const [query, setQuery] = useState("");
  const [focusLesson, setFocusLesson] = useState<number | undefined>(saved?.lesson);
  const [openFolders, setOpenFolders] = useState<string[]>([]);
  const [openDetails, setOpenDetails] = useState<string[]>([]);
  const flip = (list: string[], id: string) =>
    list.includes(id) ? list.filter((item) => item !== id) : [...list, id];
  const [original, setOriginal] = useState(false);
  const [mobileViewer, setMobileViewer] = useState(false);
  const outline = useRef<HTMLDivElement>(null);
  const [narrow, setNarrow] = useState(
    () => window.matchMedia?.("(max-width: 850px)").matches ?? false,
  );
  useEffect(() => {
    const media = window.matchMedia?.("(max-width: 850px)");
    if (!media) return;
    const changed = () => setNarrow(media.matches);
    media.addEventListener("change", changed);
    return () => media.removeEventListener("change", changed);
  }, []);
  const viewerActive = active && (!narrow || mobileViewer);
  const [notes, setNotes] = useState<LocalFile[]>([]);
  const [noteError, setNoteError] = useState("");
  const [deletedNote, setDeletedNote] = useState<{id:string;section:string;title:string} | null>(null);
  const noteCreation = useRef(false);
  const [creatingNote, setCreatingNote] = useState(false);
  const folder = useMemo(
    () => ({
      term: folderComponent(course.termId),
      course: folderComponent(
        `${course.code || course.id}${data.deviceId === "lms-ux-preview" ? "-preview" : ""}`,
      ),
      section: "section-0",
    }),
    [course.termId, course.code, course.id, data.deviceId],
  );
  const limit = downloadLimit(data.settings);
  const files = useMemo<FileAccess>(() => ({ context: folder, limit }), [folder, limit]);
  const current = useRef<ReadingState>({
    version: 2,
    expanded,
    tabs,
    selected,
    scroll: saved?.scroll ?? 0,
    anchor: saved?.anchor,
  });
  current.current = { ...current.current, expanded, tabs, selected, lesson: focusLesson };
  const language =
    data.settings.lmsBilingualFilter && !original
      ? contentLanguage(data.settings)
      : undefined;
  const materials = useMemo(
    () =>
      data.materials.filter(
        (m) =>
          m.courseId === course.id &&
          !isSyllabus(m) &&
          !m.deletedAt &&
          m.visible !== false &&
          !["assign", "quiz"].includes(m.module ?? ""),
      ),
    [data.materials, course.id],
  );
  const offeringSyllabus = data.offerings.find(o => o.id === course.offeringId)?.syllabusUrl;
  const syllabus = offeringSyllabus || (course.url?.startsWith("https://syllabus.s.isct.ac.jp/") ? course.url
    : course.offeringId ? `https://syllabus.s.isct.ac.jp/courses/-/-/-/${course.offeringId}`
    : data.materials.find(m => m.courseId === course.id && isSyllabus(m))?.resourceUrl);
  const announcementCards = useMemo(() => materials.filter(m => m.module === "forum").flatMap(m => announcements(m.bodyHtml).map(post => ({ ...post, id: `announcement:${m.id}:${post.id}`, section: m.sectionTitle }))), [materials]);
  const announcement = announcementCards.find(post => post.id === selected);
  const assignments = useMemo(
    () =>
      data.assignments.filter(
        (a) => a.courseId === course.id && !a.deletedAt && !a.hidden,
      ),
    [data.assignments, course.id],
  );
  const sections: LmsSection[] = useMemo(
    () =>
      record?.sections?.length
        ? record.sections
            .filter((s) => s.visible !== false)
            .map((s) => ({ ...s, id: Number(s.id), number: Number(s.number) }))
        : [...new Set(materials.map((m) => m.sectionNumber ?? m.session))].map(
            (number) => ({
              id: number,
              number,
              title:
                materials.find((m) => (m.sectionNumber ?? m.session) === number)
                  ?.sectionTitle || `${t("Section")} ${number}`,
              visible: true,
            }),
          ),
    [record?.sections, materials],
  );
  const itemsIn = useCallback(
    (section: LmsSection) =>
      materials.filter((m) =>
        m.sectionId != null
          ? m.sectionId === section.id
          : (m.sectionNumber ?? m.session) === section.number,
      ),
    [materials],
  );
  const sectionOf = (a: Assignment) => {
    const source = data.materials.find(
      (m) => a.lmsCmid != null && m.cmid === a.lmsCmid,
    );
    return sections.find((s) =>
      source?.sectionId != null
        ? s.id === source.sectionId
        : s.number === source?.sectionNumber,
    );
  };
  // Numbered sections are lessons; one is shown at a time, the rest fold.
  const lessons = useMemo(
    () =>
      courseLessons(sections, (id) =>
        itemsIn(sections.find((s) => s.id === id)!).some(
          (m) => m.module !== "label",
        ),
      ),
    [sections, itemsIn],
  );
  const frontier = frontierLesson(lessons);
  const labels = railLabels(lessons);
  // This course's class meetings around now: the latest one that has begun,
  // and the one a mark is recorded against (the class in progress or held
  // earlier today, else the next one).
  const { classAt, lastStarted } = useMemo(() => {
    const now = new Date();
    const today = tokyoDate(now);
    const minutes = jstMinutes(now);
    const clock = `${String(Math.floor(minutes / 60)).padStart(2, "0")}:${String(minutes % 60).padStart(2, "0")}`;
    let started: string | undefined;
    let next: string | undefined;
    for (let offset = -14; offset <= 14 && !next; offset++) {
      const date = addDays(today, offset);
      for (const block of resolveDayAgenda(data, course.termId, date)) {
        if (block.kind !== "course" || block.id !== course.id) continue;
        const start = `${date}T${block.startTime}:00+09:00`;
        if (offset < 0 || (offset === 0 && block.startTime <= clock)) started = start;
        else if (!next) next = start;
      }
    }
    return {
      lastStarted: started,
      classAt: started?.startsWith(today) ? started : (next ?? started),
    };
  }, [data, course.id, course.termId]);
  // Where the course is: the person's mark when there is one, else the LMS.
  const at = markedLesson(lessons, course.overrides?.lessonLog, lastStarted) ?? frontier;
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
  const shownId = page === ACTIVITIES || sections.some((s) => s.id === page) ? page : at?.id;
  const lessonId = lessons.some((l) => l.id === shownId) ? shownId : undefined;
  const summaries = useMemo(
    () => new Map(sections.map((s) => [s.id, lessonSummary(s.bodyHtml)])),
    [sections],
  );
  const markThisClass = (sectionId: number) => {
    if (!markLesson || !classAt) return;
    // Named in the UI language by its rail number, not by the teacher's title.
    const index = lessons.findIndex((l) => l.id === sectionId);
    markLesson(sectionId, classAt, t("Lesson {{number}}", { number: labels[index] }));
  };
  const pendingIn = (sectionId: number) =>
    assignments.filter(
      (a) =>
        sectionOf(a)?.id === sectionId &&
        !["submitted", "graded"].includes(a.submission ?? "") &&
        a.dueAt != null &&
        new Date(a.dueAt).getTime() > Date.now(),
    );
  const pageElement = (id: number | undefined) =>
    id == null
      ? null
      : outline.current?.querySelector<HTMLElement>(
          id === ACTIVITIES ? '[data-page="activities"]' : `[data-page="${id}"]`,
        ) ?? null;
  const goLesson = (id: number, smooth = true) => {
    const host = outline.current;
    const target = pageElement(id);
    setPage(id);
    setFocusLesson(id);
    if (!host || !target) return;
    // The page follows the scroll again once this scroll has arrived.
    const state = pull.current;
    state.turning = target.offsetTop;
    state.motion?.cancel();
    if (!smooth) {
      host.scrollTop = target.offsetTop;
      return;
    }
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
  });
  const pageRef = useRef(shownId);
  pageRef.current = shownId;
  const settleInto = (host: HTMLDivElement) => {
    if (!paged || pull.current.turning != null) return;
    const boundaries = [...host.querySelectorAll<HTMLElement>("[data-page]")]
      .slice(1)
      .map((element) => element.offsetTop);
    const target = gravityTarget(host.scrollTop, host.clientHeight, boundaries, host.scrollHeight);
    if (target == null) return;
    const state = pull.current;
    state.motion?.cancel();
    state.motion = springScroll(host, target, { onDone: () => (state.motion = null) });
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
      if (current.offsetLeft < nav.scrollLeft || current.offsetLeft + current.offsetWidth > nav.scrollLeft + nav.clientWidth)
        nav.scrollTo?.({ left: current.offsetLeft - nav.clientWidth / 2, behavior: "smooth" });
    };
    place();
    const resize = typeof ResizeObserver === "function" ? new ResizeObserver(place) : undefined;
    resize?.observe(nav);
    return () => resize?.disconnect();
  }, [shownId, paged, lessons, pinned.length]);
  // Activities outside any section get one rail entry of their own.
  const hasActivities =
    assignments.some((a) => !sectionOf(a)) ||
    materials.some(
      (m) =>
        !sections.some((s) =>
          m.sectionId != null
            ? m.sectionId === s.id
            : (m.sectionNumber ?? m.session) === s.number,
        ),
    );
  const searchable = useMemo(
    () =>
      new Map([
        ...sections.map((s) => [s.bodyHtml, contentText(s.bodyHtml)] as const),
        ...materials.map((m) => [m.bodyHtml, contentText(m.bodyHtml)] as const),
        ...assignments.map((a) => [a.details, contentText(a.details)] as const),
      ]),
    [sections, materials, assignments],
  );
  const refreshNotes = () =>
    void listNotes(
      folder,
      sections.map((s) => `section-${s.id}`),
    )
      .then(value => setNotes(value.sort((a,b) => a.id.localeCompare(b.id))))
      .catch((error) => setNoteError(String(error)));
  useEffect(() => {
    if (!active) return;
    refreshNotes();
    window.addEventListener("course-storage-changed", refreshNotes);
    return () =>
      window.removeEventListener("course-storage-changed", refreshNotes);
  }, [active, folder, sections]);
  const newNote = async (section: LmsSection) => {
    if (noteCreation.current) return;
    noteCreation.current = true; setCreatingNote(true); setNoteError("");
    const context = { ...folder, section: `section-${section.id}` };
    try {
      const existing = (await listNotes(folder, [context.section])).sort((a,b) => a.id.localeCompare(b.id))[0];
      let id = existing?.id;
      if (!existing) id = (await saveNote(context, "", `# ${section.title}

`)).id;
      refreshNotes();
      open({ id: id!, title: existing?.name.trim() || `${section.title} · ${t("Note")}`, kind: "note", section: context.section });
    } catch (error) { setNoteError(String(error)); }
    finally { noteCreation.current = false; setCreatingNote(false); }
  };

  const matches = (title: string, html?: string) => !query.trim() ||
    `${title} ${html ? searchable.get(html) || contentText(html) : ""}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase());
  const open = useCallback(
    (tab: ViewTab) => {
      if (outline.current) recordPosition(outline.current);
      setTabs((previous) =>
        previous.some((item) => item.id === tab.id)
          ? previous
          : [...previous, tab].slice(-5),
      );
      setSelected(tab.id);
      setMobileViewer(true);
    },
    [query],
  );
  const [fileMenu, setFileMenu] = useState<{ url: string; title: string; anchor: Element }>();
  const closeFileMenu = useCallback(() => setFileMenu(undefined), []);
  /**
   * Files the app cannot preview open as a popover at the clicked item, so the
   * reading pane is only used for content. Returns false for previewable ones.
   */
  const showFileMenu = (url: string | undefined, title: string, anchor?: Element) => {
    if (!url || !anchor || previewKind(url) || !new URL(url).pathname.includes("/pluginfile.php/")) return false;
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
    setMobileViewer(false);
    requestAnimationFrame(() =>
      outline.current
        ?.querySelector(`[data-assignment="${CSS.escape(a.id)}"]`)
        ?.scrollIntoView?.({ block: "center" }),
    );
  }, []);
  const openLink = useCallback(
    (url: string, title: string, anchor?: Element) => {
      const material = data.materials.find((m) => m.url === url || m.resourceUrl === url);
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
    if (positionedRequest.current === entry.request) return;
    const assignment = assignments.find((a) => a.id === entry.assignmentId);
    if (assignment) {
      const source = data.materials.find(
        (m) => assignment.lmsCmid != null && m.cmid === assignment.lmsCmid,
      );
      const section = sections.find(
        (s) => s.id === source?.sectionId || s.number === source?.sectionNumber,
      );
      if (section) {
        setExpanded((previous) =>
          previous.filter((id) => id !== `section:${section.id}`),
        );
        if (lessons.some((l) => l.id === section.id)) {
          setFocusLesson(section.id);
          setPage(section.id);
        }
      }
      // Arriving at one assignment shows its requirements at once.
      setOpenDetails((list) => (list.includes(assignment.id) ? list : [...list, assignment.id]));
      openAssignment(assignment);
    }
    // No date→section association exists in the current importer. Resume the
    // real reading position rather than guessing the lesson from the week.
    const frame = requestAnimationFrame(() => {
      positionedRequest.current = entry.request;
      if (outline.current) {
        const anchor = current.current.anchor;
        const element =
          anchor &&
          [
            ...outline.current.querySelectorAll<HTMLElement>(
              "[data-reading-section]",
            ),
          ].find((el) => el.dataset.readingSection === anchor.id);
        if (!assignment && element && anchor) {
          const panel = outline.current.getBoundingClientRect();
          const rect = element.getBoundingClientRect();
          outline.current.scrollTop +=
            rect.top -
            panel.top +
            rect.height * anchor.ratio -
            panel.height / 2;
        } else if (!assignment && lessons.length && at) {
          // Arriving at a lesson course opens the page the course is at.
          const page = outline.current.querySelector<HTMLElement>(`[data-page="${at.id}"]`);
          outline.current.scrollTop = page?.offsetTop ?? 0;
          setPage(at.id);
        } else
          outline.current.scrollTop = assignment ? 0 : current.current.scroll;
      }
      if (assignment)
        outline.current
          ?.querySelector(`[data-assignment="${CSS.escape(assignment.id)}"]`)
          ?.scrollIntoView?.({ block: "nearest" });
    });
    return () => cancelAnimationFrame(frame);
    // Explicit entrance only; refreshing content must never jump the reader.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [entry.request]);
  const saveReading = () => {
    reading.delete(key);
    reading.set(key, current.current);
    if (reading.size > 8) reading.delete(reading.keys().next().value!);
    try {
      sessionStorage.setItem(
        `lms-reading:${key}`,
        JSON.stringify(current.current),
      );
    } catch {
      /* Reading remains available in memory. */
    }
    try {
      const indexKey = `lms-reading-index:${data.deviceId}`;
      const stored: unknown = JSON.parse(
        sessionStorage.getItem(indexKey) || "[]",
      );
      const index = [
        ...(Array.isArray(stored)
          ? stored.filter((k) => typeof k === "string" && k !== key)
          : []),
        key,
      ];
      index
        .slice(0, -8)
        .forEach((k) => sessionStorage.removeItem(`lms-reading:${k}`));
      sessionStorage.setItem(indexKey, JSON.stringify(index.slice(-8)));
    } catch {
      /* Persistence is optional. */
    }
  };
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
  const activeTab = tabs.find((tab) => tab.id === selected);
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
    if (panel.height > 0 && rect.height > 0) {
      host.scrollTop +=
        rect.top - panel.top + rect.height * anchor.ratio - panel.height / 2;
    }
  }, [Boolean(activeTab)]);
  const material =
    activeTab?.kind === "material"
      ? materials.find((m) => m.id === activeTab.id)
      : undefined;
  const toggle = (id: string) =>
    setExpanded((previous) =>
      previous.includes(id)
        ? previous.filter((item) => item !== id)
        : [...previous, id],
    );
  const closeTab = (id: string) => {
    if (outline.current) recordPosition(outline.current);
    const next = tabs.filter((tab) => tab.id !== id);
    setTabs(next);
    if (selected === id) { setSelected(undefined); setMobileViewer(false); }
    if (!next.length) setMobileViewer(false);
  };
  /** Assignment card: state and deadline first, the teacher's text on request. */
  const assignmentContent = (a: Assignment) => {
    const source = data.materials.find(
      (m) => a.lmsCmid != null && m.cmid === a.lmsCmid,
    );
    const done = ["submitted", "graded"].includes(a.submission ?? "");
    const due = a.dueAt ? new Date(a.dueAt) : undefined;
    const past = due ? due.getTime() <= Date.now() : false;
    const state = a.submission === "graded" ? "graded" : done ? "done" : past ? "missed" : "open";
    const countdown = a.dueAt && !done && !past ? dueCountdown(a.dueAt) : undefined;
    const body = source?.bodyHtml ? withoutActivityDates(source.bodyHtml) : "";
    const hasBody = Boolean(contentText(body).trim() || a.details?.trim());
    const showBody = openDetails.includes(a.id) || Boolean(query.trim());
    return (
      <article className={`lmsAssignmentCard ${state}`} data-assignment={a.id} key={a.id}>
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
                  <LmsIcon name="chevron" size={12} className="lmsAssignmentChevron" />
                </button>
              ) : (
                a.title
              )}
            </h4>
            {due && (
              <time dateTime={a.dueAt!}>
                {t("Due")}{" "}
                {new Intl.DateTimeFormat(data.settings.locale, {
                  month: "short",
                  day: "numeric",
                  weekday: "short",
                  hour: "2-digit",
                  minute: "2-digit",
                  hourCycle: "h23",
                  timeZone: "Asia/Tokyo",
                }).format(due)}
              </time>
            )}
          </div>
          {countdown ? (
            <span className={`dueChip ${countdown.tone}`}>{countdown.label}</span>
          ) : (
            <span className={`lmsAssignmentState ${state}`}>
              {t(state === "graded" ? "Graded" : state === "done" ? "Submitted" : state === "missed" ? "Not submitted" : "No deadline")}
            </span>
          )}
        </header>
        {showBody && hasBody && (
          <div className="lmsAssignmentBody">
            {body ? (
              <RichContent files={files} html={body} language={language} openLink={openLink} />
            ) : (
              <p>{a.details}</p>
            )}
          </div>
        )}
        <footer>
          {a.sourceUrl && (
            <button
              type="button"
              className={done ? "lmsTextLink" : "lmsSubmit"}
              onClick={() =>
                open({
                  id: `official:${a.id}`,
                  title: a.title,
                  kind: "official",
                  url: a.sourceUrl,
                })
              }
            >
              {t(done ? "View in LMS" : "Submit")} <LmsIcon name="external" size={13} />
            </button>
          )}
        </footer>
      </article>
    );
  };
  const forumCards = (m: Material) => {
    const posts = announcementCards.filter(post => post.id.startsWith(`announcement:${m.id}:`));
    return <article className="lmsInlineActivity" key={m.id}><h4>{m.title}</h4>{posts.length ? posts.map(post => (
      <button type="button" className={`lmsAnnouncementCard${selected === post.id ? " selected" : ""}`} key={post.id} onClick={() => open({id:post.id, title:post.title, kind:"announcement", url:post.url})}>
        <span className="lmsCardType">{formatLmsDate(post.date, data.settings.locale)}</span><strong>{post.title}</strong><span className="lmsCardExcerpt">{contentText(post.html).trim().replace(/\s+/g," ").slice(0,140)}</span><LmsIcon name="chevron" className="lmsCardArrow" />
      </button>
    )) : <p className="lmsQuiet">{t("No announcements yet.")}{" "}
      {safeUrl(m.url) && <button type="button" className="lmsTextLink" onClick={() => open({ id: `official:${m.url}`, title: m.title, kind: "official", url: m.url })}>{t("Open in LMS")}</button>}</p>}</article>;
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
            <Suspense fallback={<p className="lmsQuiet lmsFolderNote">{t("Loading…")}</p>}>
              <InlineFolder
                url={safeUrl(m.url)!}
                context={folder}
                limit={limit}
                known={knownFolderFiles(m)}
                openFile={(fileUrl, name, anchor) => openLink(fileUrl, name, anchor)}
                openInApp={() => open({ id: `official:${m.url}`, title: m.title, kind: "official", url: m.url })}
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
        {family === "image" && url.includes("/pluginfile.php/")
          ? <ImageThumbnail url={url} files={files} />
          : <FileTile family={family} />}
        <span>
          <b>{m.title}</b>
          <small>{ext ? ext.toUpperCase() : t(m.module ?? "File")}</small>
        </span>
        <LmsIcon name="chevron" className="lmsRowAction" />
      </button>
    );
  };
  /** One outline section; in a lesson course, lessons are its pages. */
  const sectionView = (section: LmsSection) => {
              const id = `section:${section.id}`;
              const items = itemsIn(section);
              if (
                !matches(section.title, section.bodyHtml) &&
                !items.some((m) => matches(m.title, m.bodyHtml))
              )
                return null;
              const lesson = lessons.find((l) => l.id === section.id);
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
                            <span className="lmsLessonNo">{labels[lessons.indexOf(lesson)].padStart(2, "0")}</span>
                            <span>{summaries.get(section.id) || section.title}</span>
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
                  {!isOpen && section.bodyHtml && (
                    <p className="lmsSectionPreview">
                      {t("Section collapsed")}
                    </p>
                  )}
                  {isOpen && (
                    <div
                      id={`lms-panel-${section.id}`}
                      className="lmsSectionBody"
                    >
                      {section.bodyHtml && (
                        <RichContent files={files}
                          html={section.bodyHtml}
                          language={language}
                          openLink={openLink}
                        />
                      )}
                      {groupImageLabels(items.filter((m) => !query || matches(m.title, m.bodyHtml) || matches(section.title, section.bodyHtml)))
                        .map((m) =>
                          Array.isArray(m) ? (
                            <div className="lmsImageGrid" key={m[0].id}>
                              {m.map((label) => <RichContent key={label.id} files={files} html={label.bodyHtml!} language={language} openLink={openLink} />)}
                            </div>
                          ) : m.module === "forum" ? forumCards(m)
                          : m.bodyHtml && ["page", "label"].includes(m.module ?? "") ? (
                            <article className="lmsInlineActivity" key={m.id}>
                              {/* Moodle names a label after its own text, so only pages get a heading. */}
                              {m.module === "page" && <h4>{m.title}</h4>}
                              <RichContent files={files} html={m.bodyHtml} language={language} openLink={openLink} />
                            </article>
                          ) : row(m),
                        )}
                      {assignments
                        .filter((a) =>
                          data.materials.some(
                            (m) =>
                              a.lmsCmid != null &&
                              m.cmid === a.lmsCmid &&
                              (m.sectionId != null
                                ? m.sectionId === section.id
                                : m.sectionNumber === section.number),
                          ),
                        )
                        .map(assignmentContent)}
                      {(() => {
                        const sectionNotes = notes.filter(note => note.section === `section-${section.id}`);
                        const row = (note: LocalFile) => <button type="button" className="lmsItemRow" key={note.id} onClick={() => open({id:note.id,title:note.name.trim() || `${section.title} · ${t("Note")}`,kind:"note",section:note.section})}><FileTile family="note" /><span><b>{note.name.trim() || `${section.title} · ${t("Note")}`}</b><small>{t("Local note · stored only on this device")}</small></span><LmsIcon name="chevron" className="lmsRowAction" /></button>;
                        return <>{sectionNotes[0] && row(sectionNotes[0])}{sectionNotes.length > 1 && <details className="lmsPreviousNotes"><summary>{t("Previous notes")} · {sectionNotes.length - 1}</summary>{sectionNotes.slice(1).map(row)}</details>}</>;
                      })()}
                      {!section.bodyHtml && !items.length && (
                        <p className="lmsQuiet">
                          {t("No content in this section")}
                        </p>
                      )}
                    </div>
                  )}
                </section>
              );
  };
  return (
    <div
      className={`lmsWorkspace${activeTab ? " hasViewer" : ""}${mobileViewer ? " viewing" : ""}`}
    >
      <header className="lmsWorkspaceHeader">
        {!navigationProvided && (
          <button type="button" className="lmsBack" onClick={close}>
            <LmsIcon name="back" /> {t("Today")}
          </button>
        )}
        <div className="lmsCourseIdentity">
          <small>{course.code}</small>
          <h1>{course.title}</h1>
        </div>
        {entry.date && (
          <span className="lmsEntryTime">
            {entry.date}{" "}
            {entry.startTime && `· ${entry.startTime}–${entry.endTime}`}
          </span>
        )}
        {syllabus && <button type="button" className="lmsRefresh" aria-pressed={selected === "syllabus"} onClick={() => open({ id: "syllabus", title: t("Syllabus"), kind: "syllabus", url: syllabus })}><LmsIcon name="doc" size={14} /> {t("Syllabus")}</button>}
        {refresh && (
          <button
            type="button"
            className="lmsRefresh"
            disabled={progress != null || activeTab?.kind === "official"}
            onClick={refresh}
          >
            {progress != null ? `${Math.round(progress)}%` : t("Refresh")}
          </button>
        )}
      </header>
      <div className="lmsWorkspaceTools">
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
                    expanded.length ? [] : sections.map((s) => `section:${s.id}`),
                  )
                }
              >
                {expanded.length ? t("Expand all") : t("Collapse all")}
              </button>
            )}
          </div>
          {lessons.length > 0 && !query.trim() && (
            <nav className="lmsLessonRail" aria-label={t("Lessons")} ref={rail}>
              <span className="lmsRailIndicator" ref={railIndicator} aria-hidden="true" />
              {pinned.map((section) => (
                <button
                  type="button"
                  key={section.id}
                  className={`lmsRailTab${paged && shownId === section.id ? " current" : ""}`}
                  aria-current={paged && shownId === section.id ? "step" : undefined}
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
                  aria-current={paged && shownId === ACTIVITIES ? "step" : undefined}
                  onClick={() => goLesson(ACTIVITIES)}
                >
                  {t("Activities")}
                </button>
              )}
              {(pinned.length > 0 || hasActivities) && <span className="lmsRailDivider" aria-hidden="true" />}
              {lessons.map((l, index) => (
                <button
                  type="button"
                  key={l.id}
                  className={`${atIndex >= 0 && index <= atIndex ? "held" : ""}${l.id === lessonId && paged ? " current" : ""}${pendingIn(l.id).length ? " due" : ""}`}
                  aria-current={l.id === lessonId && paged ? "step" : undefined}
                  title={sections.find((x) => x.id === l.id)?.title}
                  onClick={() => goLesson(l.id)}
                  onDoubleClick={() => markThisClass(l.id)}
                >
                  {labels[index]}
                </button>
              ))}
            </nav>
          )}
      </div>
      <div className="lmsWorkspaceBody">
        <CurrentLesson.Provider value={at?.number}><aside className="lmsOutline">
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
                    open({ id: `official:${fileMenu.url}`, title: fileMenu.title, kind: "official", url: fileMenu.url });
                  }}
                />
              </Suspense>
            )}
            {noteError && <p role="alert">{noteError}</p>}
            {deletedNote && <div className="lmsNoteUndo" role="status"><span>{t("Note deleted")}</span><button type="button" onClick={() => { const note=deletedNote; void moveNote({...folder,section:note.section},note.id,true).then(() => {setDeletedNote(null);refreshNotes();open({...note,kind:"note"});}).catch(error => setNoteError(String(error))); }}>{t("Undo")}</button></div>}
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
              const loose = assignments.filter(
                (a) => !sectionOf(a) && matches(a.title, a.details),
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
            {["Discussions", "Links", "Other activities"].map((group) => {
              const items = materials.filter(
                (m) =>
                  groupFor(m.module) === group &&
                  !sections.some((s) =>
                    m.sectionId != null
                      ? m.sectionId === s.id
                      : (m.sectionNumber ?? m.session) === s.number,
                  ) &&
                  matches(m.title, m.bodyHtml),
              );
              return items.length ? (
                <div
                  key={group}
                  data-reading-section={`group:${group}`}
                  className={`lmsActivityGroup${readingSection === `group:${group}` ? " reading" : ""}`}
                >
                  <h2>{t(group)}</h2>
                  {items.map(m => m.module === "forum" ? forumCards(m) : row(m))}
                </div>
              ) : null;
            })}
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
        </aside></CurrentLesson.Provider>
        {activeTab && (
          <section className="lmsViewer" aria-label={t("Opened content")}>
            <button className="lmsViewerClose" type="button" aria-label={`${t("Close")} ${activeTab.title}`} title={t("Close")} onClick={() => closeTab(activeTab.id)}><LmsIcon name="close" /></button>
            <div className="lmsViewerBody">
              {/* Hidden readers release their documents; the official view keeps its webview. */}
              {!viewerActive && activeTab.kind !== "official" ? null : activeTab.kind === "announcement" ? (
                <div className="lmsDocument"><div className="lmsDocumentHeading"><small>{formatLmsDate(announcement?.date || "", data.settings.locale)}</small><h2>{activeTab.title}</h2></div>
                {announcement && <RichContent files={files} html={announcement.html} language={language} openLink={openLink} />}
                </div>
              ) : activeTab.kind === "syllabus" ? (
                <div className="lmsDocument"><Suspense fallback={<p>{t("Loading…")}</p>}>
                  <SyllabusDetails variant="document" currentLesson={at?.number} url={activeTab.url!} fetchCourse={desktopBridge ? url => desktopBridge!.syllabusCourse(url, contentLanguage(data.settings)) : undefined} />
                </Suspense></div>
              ) : activeTab.kind === "note" ? (
                <Suspense fallback={<p>{t("Loading editor…")}</p>}>
                  <NoteEditor
                    key={activeTab.id}
                    id={activeTab.id}
                    context={{ ...folder, section: activeTab.section! }}
                    onSaved={refreshNotes}
                    onDeleted={() => { setDeletedNote({id:activeTab.id,section:activeTab.section!,title:activeTab.title}); closeTab(activeTab.id); refreshNotes(); }}
                  />
                </Suspense>
              ) : activeTab.kind === "official" ? (
                <>
                  <div className="lmsSourceBar">
                    Official LMS · lms.s.isct.ac.jp
                  </div>
                  <OfficialLmsView
                    url={activeTab.url!}
                    active={viewerActive}
                    available={schoolWebAvailable}
                  />
                </>
              ) : (
                <div className="lmsDocument lmsFileDocument">
                  <div className="lmsDocumentHeading">
                    <small>{material?.sectionTitle || course.code}</small>
                    <h2>{activeTab.title}</h2>
                  </div>
                  {material?.bodyHtml && (
                    <RichContent files={files}
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
                        (activeTab.kind === "link" ? activeTab.url : undefined),
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
                            new URL(official).hostname === "lms.s.isct.ac.jp" && (
                              <button
                                type="button"
                                className="lmsRefresh"
                                onClick={() =>
                                  open({
                                    id: `official:${official}`,
                                    title: activeTab.title,
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
                          title={activeTab.title}
                          context={folder}
                          limit={limit}
                          openInApp={new URL(url).hostname === "lms.s.isct.ac.jp"
                            ? () => open({ id: `official:${url}`, title: activeTab.title, kind: "official", url })
                            : undefined}
                          openFile={(fileUrl, name) => open({ id: `link:${fileUrl}`, title: name, kind: "link", url: fileUrl })}
                        />
                      </Suspense>
                    );
                  })()}
                </div>
              )}
            </div>
          </section>
        )}
      </div>
    </div>
  );
}
