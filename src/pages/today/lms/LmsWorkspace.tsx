import "./lms.css";
import {
  lazy,
  memo,
  Suspense,
  useCallback,
  useLayoutEffect,
  useEffect,
  useMemo,
  useRef,
  useState,
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
  const content = useMemo(() => safeContent(html, language), [html, language]);
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
  current.current = { ...current.current, expanded, tabs, selected };
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
  useEffect(() => {
    const assignment = assignments.find((a) => a.id === entry.assignmentId);
    if (assignment) {
      const source = data.materials.find(
        (m) => assignment.lmsCmid != null && m.cmid === assignment.lmsCmid,
      );
      const section = sections.find(
        (s) => s.id === source?.sectionId || s.number === source?.sectionNumber,
      );
      if (section)
        setExpanded((previous) =>
          previous.filter((id) => id !== `section:${section.id}`),
        );
      openAssignment(assignment);
    }
    // No date→section association exists in the current importer. Resume the
    // real reading position rather than guessing the lesson from the week.
    const frame = requestAnimationFrame(() => {
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
  const assignmentContent = (a: Assignment) => {
    const source = data.materials.find(
      (m) => a.lmsCmid != null && m.cmid === a.lmsCmid,
    );
    return (
      <article className="lmsInlineActivity" data-assignment={a.id} key={a.id}>
        <h4>{a.title}</h4>
        <p className="lmsQuiet">
          {t(a.submission ?? "unknown")}
          {a.dueAt &&
            ` · ${new Date(a.dueAt).toLocaleString(data.settings.locale, { timeZone: "Asia/Tokyo" })}`}
        </p>
        {source?.bodyHtml ? (
          <RichContent files={files}
            html={source.bodyHtml}
            language={language}
            openLink={openLink}
          />
        ) : (
          <p>{a.details || t("No assignment description synced yet.")}</p>
        )}
        {a.sourceUrl && (
          <button
            type="button"
            className="lmsSubmit"
            onClick={() =>
              open({
                id: `official:${a.id}`,
                title: a.title,
                kind: "official",
                url: a.sourceUrl,
              })
            }
          >
            {t("Submit in LMS")} <LmsIcon name="external" size={14} />
          </button>
        )}
      </article>
    );
  };
  const forumCards = (m: Material) => {
    const posts = announcementCards.filter(post => post.id.startsWith(`announcement:${m.id}:`));
    return <article className="lmsInlineActivity" key={m.id}><h4>{m.title}</h4>{posts.length ? posts.map(post => (
      <button type="button" className={`lmsAnnouncementCard${selected === post.id ? " selected" : ""}`} key={post.id} onClick={() => open({id:post.id, title:post.title, kind:"announcement", url:post.url})}>
        <span className="lmsCardType">{t("Announcement")} · {formatLmsDate(post.date, data.settings.locale)}</span><strong>{post.title}</strong><span className="lmsCardExcerpt">{contentText(post.html).trim().replace(/\s+/g," ").slice(0,140)}</span><LmsIcon name="chevron" className="lmsCardArrow" />
      </button>
    )) : row(m)}</article>;
  };
  const row = (m: Material) => {
    const url = m.resourceUrl || m.url || "";
    const family = fileFamily(m.resourceUrl || "", m.module);
    const ext = extension(m.resourceUrl || "");
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
          <small>{[ext && ext.toUpperCase(), m.sectionTitle || t(m.module ?? "File")].filter(Boolean).join(" · ")}</small>
        </span>
        <LmsIcon name="chevron" className="lmsRowAction" />
      </button>
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
        {syllabus && <button type="button" className="lmsRefresh" onClick={() => open({ id: "syllabus", title: t("Syllabus"), kind: "syllabus", url: syllabus })}>{t("Open in syllabus")} <LmsIcon name="external" size={14} /></button>}
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
      <div className="lmsWorkspaceBody">
        <aside className="lmsOutline">
          <div className="lmsOutlineTools">
            <input
              aria-label={t("Search course content")}
              placeholder={t("Search this course…")}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
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
          </div>
          <div
            className={`lmsOutlineScroll${query ? " searching" : ""}`}
            ref={outline}
            onScroll={(event) => recordPosition(event.currentTarget)}
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
            <h2>{t("Course content")}</h2>
            {sections.map((section) => {
              const id = `section:${section.id}`;
              const items = materials.filter((m) =>
                m.sectionId != null
                  ? m.sectionId === section.id
                  : (m.sectionNumber ?? m.session) === section.number,
              );
              if (
                !matches(section.title, section.bodyHtml) &&
                !items.some((m) => matches(m.title, m.bodyHtml))
              )
                return null;
              const isOpen = !expanded.includes(id) || Boolean(query);
              return (
                <section
                  className={`lmsSection${readingSection === id ? " reading" : ""}`}
                  data-reading-section={id}
                  key={section.id}
                >
                  <div className="lmsSectionHeading">
                    <h3>
                      <button
                        type="button"
                        aria-expanded={isOpen}
                        aria-controls={`lms-panel-${section.id}`}
                        onClick={() => toggle(id)}
                      >
                        <LmsIcon name="chevron" className="lmsSectionChevron" />
                        <span>{section.title}</span>
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
                              <h4>{m.title}</h4>
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
            })}
            {!sections.length && (
              <p className="lmsQuiet">
                {t(
                  "No course content synced yet. Refresh LMS to load the outline.",
                )}
              </p>
            )}
            <section
              data-reading-section="assignments"
              className={`lmsActivityGroup${readingSection === "assignments" ? " reading" : ""}`}
            >
              <h2>
                {t("Assignments")} <small>{assignments.length}</small>
              </h2>
              {assignments
                .filter(
                  (a) =>
                    !data.materials.some(
                      (m) =>
                        a.lmsCmid != null &&
                        m.cmid === a.lmsCmid &&
                        sections.some((s) =>
                          m.sectionId != null
                            ? m.sectionId === s.id
                            : m.sectionNumber === s.number,
                        ),
                    ),
                )
                .filter((a) => matches(a.title, a.details))
                .map(assignmentContent)}
              {!assignments.length && (
                <p className="lmsQuiet">{t("No assignments")}</p>
              )}
            </section>
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
                  <h2>
                    {t(group)} <small>{items.length}</small>
                  </h2>
                  {items.map(m => m.module === "forum" ? forumCards(m) : row(m))}
                </div>
              ) : null;
            })}
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
                  <SyllabusDetails url={activeTab.url!} fetchCourse={desktopBridge ? url => desktopBridge!.syllabusCourse(url, contentLanguage(data.settings)) : undefined} />
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
