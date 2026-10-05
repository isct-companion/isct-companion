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
import { announcements, formatLmsDate, contentText, safeContent, safeUrl } from "./content";
import { OfficialLmsView } from "./OfficialLmsView";
import {
  openMaterial as readMaterial,
  fileSrc,
  type CourseFolder,
  folderComponent,
  listNotes,
  saveNote,
  moveNote,
  previewKind,
  downloadMaterial,
  type LocalFile,
} from "./files";
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
const groupFor = (module?: string) =>
  module === "forum"
    ? "Discussions"
    : module === "url"
      ? "Links"
      : [undefined, "resource", "folder", "label", "page"].includes(module)
        ? "Course content"
        : "Other activities";

const RichContent = memo(function RichContent({
  html,
  language,
  openLink,
  context,
}: {
  context?: CourseFolder;
  html: string;
  language?: "ja" | "en";
  openLink: (url: string, title: string) => void;
}) {
  const content = useMemo(() => safeContent(html, language), [html, language]);
  const element = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let disposed = false;
    const load = async (placeholder: HTMLElement) => {
      const url = placeholder.dataset.imageSrc!;
      try {
        let src = url;
        if (desktopBridge && new URL(url).pathname.includes("/pluginfile.php/")) {
          if (!context) return;
          const file = await readMaterial(context, url, false, false) || await readMaterial(context, url, true, false);
          if (!file || disposed) return;
          src = fileSrc(file);
        }
        if (disposed) return;
        const image = document.createElement("img");
        image.src = src; image.alt = placeholder.dataset.imageAlt || "";
        image.decoding = "async"; image.className = "lmsInlineThumbnail";
        placeholder.replaceChildren(image);
      } catch { /* The full-size file action remains available if preview fails. */ }
    };
    const placeholders = element.current?.querySelectorAll<HTMLElement>("[data-image-src]") || [];
    const observer = typeof IntersectionObserver === "undefined" ? null : new IntersectionObserver(entries => {
      for (const entry of entries) if (entry.isIntersecting) { observer!.unobserve(entry.target); void load(entry.target as HTMLElement); }
    }, {rootMargin:"80px"});
    for (const placeholder of placeholders) { if (observer) observer.observe(placeholder); else void load(placeholder); }
    return () => { disposed = true; observer?.disconnect(); };
  }, [content, context?.term, context?.course, context?.section]);
  return (
    <div
      ref={element}
      className="lmsRichContent"
      dangerouslySetInnerHTML={{ __html: content }}
      onClick={(event) => {
        const link = (event.target as Element).closest("a");
        if (!link) return;
        event.preventDefault();
        const url = safeUrl(link.getAttribute("href"));
        if (url) openLink(url, link.textContent || t("Link"));
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
  const [fileMessage, setFileMessage] = useState("");
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
    data.settings.lmsBilingualFilter &&
    !original &&
    (data.settings.locale === "ja" || data.settings.locale === "en")
      ? data.settings.locale
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
      const id = existing?.id || crypto.randomUUID();
      if (!existing) await saveNote(context, id, `# ${section.title}

`);
      refreshNotes();
      open({ id, title: existing?.name.trim() || `${section.title} · ${t("Note")}`, kind: "note", section: context.section });
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
  const openMaterial = useCallback(
    (m: Material) => {
      const url = safeUrl(m.resourceUrl || (m.url?.includes("/pluginfile.php/") ? m.url : undefined));
      if (
        url &&
        !previewKind(url) &&
        new URL(url).pathname.includes("/pluginfile.php/")
      ) {
        if (!desktopBridge) {
          window.open(url, "_blank", "noopener,noreferrer");
          return;
        }
        setFileMessage(t("Downloading…"));
        void downloadMaterial({ ...folder, section: `section-${m.sectionId ?? 0}` }, url)
          .then(path => setFileMessage(path ? `${t("Saved locally")} · ${path}` : t("Save cancelled")))
          .catch((error) => setFileMessage(String(error)));
        return;
      }
      open({ id: m.id, title: m.title, kind: "material" });
    },
    [open, folder, sections],
  );
  const openAssignment = useCallback((a: Assignment) => {
    if (outline.current) outline.current.style.scrollSnapType = "none";
    setMobileViewer(false);
    requestAnimationFrame(() =>
      outline.current
        ?.querySelector(`[data-assignment="${CSS.escape(a.id)}"]`)
        ?.scrollIntoView?.({ block: "center" }),
    );
  }, []);
  const openLink = useCallback(
    (url: string, title: string) => {
      const material = data.materials.find(
        (m) => m.url === url || m.resourceUrl === url,
      );
      if (material) return openMaterial(material);
      const assignment = data.assignments.find((a) => a.sourceUrl === url);
      if (assignment) return openAssignment(assignment);
      if (new URL(url).pathname.includes("/pluginfile.php/") && !previewKind(url)) {
        if (!desktopBridge) { window.open(url, "_blank", "noopener,noreferrer"); return; }
        const section = sections.find(item => `section:${item.id}` === readingSection);
        setFileMessage(t("Downloading…"));
        void downloadMaterial({...folder, section:`section-${section?.id ?? 0}`}, url)
          .then(path => setFileMessage(path ? `${t("Saved locally")} · ${path}` : t("Save cancelled")))
          .catch(error => setFileMessage(String(error)));
        return;
      }
      open({ id: `link:${url}`, title, kind: "link", url });
    },
    [data.materials, data.assignments, openMaterial, openAssignment, open, folder, sections, readingSection],
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
        outline.current.style.scrollSnapType = "none";
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
      host.style.scrollSnapType = "none";
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
          <RichContent context={{...folder,section:"section-0"}}
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
            {t("Submit in LMS")} ↗
          </button>
        )}
      </article>
    );
  };
  const forumCards = (m: Material) => {
    const posts = announcementCards.filter(post => post.id.startsWith(`announcement:${m.id}:`));
    return <article className="lmsInlineActivity" key={m.id}><h4>{m.title}</h4>{posts.length ? posts.map(post => (
      <button type="button" className={`lmsAnnouncementCard${selected === post.id ? " selected" : ""}`} key={post.id} onClick={() => open({id:post.id, title:post.title, kind:"announcement", url:post.url})}>
        <span className="lmsCardType">{t("Announcement")} · {formatLmsDate(post.date, data.settings.locale)}</span><strong>{post.title}</strong><span className="lmsCardExcerpt">{contentText(post.html).trim().replace(/\s+/g," ").slice(0,140)}</span><span className="lmsCardArrow" aria-hidden="true">↗</span>
      </button>
    )) : row(m)}</article>;
  };
  const row = (m: Material) => (
    <button
      type="button"
      className={`lmsItemRow${selected === m.id ? " selected" : ""}`}
      key={m.id}
      onClick={() => openMaterial(m)}
    >
      <span className="lmsItemGlyph" aria-hidden="true">
        {m.module === "folder"
          ? "▤"
          : m.module === "forum"
            ? "◇"
            : m.module === "url"
              ? "↗"
              : previewKind(m.resourceUrl || m.url || "") === "pdf" ? "PDF" : "↧"}
      </span>
      <span>
        <b>{m.title}</b>
        <small>{m.sectionTitle || t(m.module ?? "File")}</small>
      </span>
      <span aria-hidden="true">›</span>
    </button>
  );
  return (
    <div
      className={`lmsWorkspace${activeTab ? " hasViewer" : ""}${mobileViewer ? " viewing" : ""}`}
    >
      <header className="lmsWorkspaceHeader">
        {!navigationProvided && (
          <button type="button" className="lmsBack" onClick={close}>
            ← {t("Today")}
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
        {syllabus && <button type="button" className="lmsRefresh" onClick={() => open({ id: "syllabus", title: t("Syllabus"), kind: "syllabus", url: syllabus })}>{t("Open in syllabus")} ↗</button>}
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
      {fileMessage && (
        <div className="lmsSourceBar" role="status">
          {fileMessage}
          <button
            type="button"
            aria-label={t("Dismiss")}
            onClick={() => setFileMessage("")}
          >
            ×
          </button>
        </div>
      )}
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
            onWheel={(event) => {
              event.currentTarget.style.scrollSnapType = "";
            }}
            onTouchStart={(event) => {
              event.currentTarget.style.scrollSnapType = "";
            }}
            onKeyDown={(event) => {
              if (
                ["PageDown", "PageUp", "ArrowDown", "ArrowUp", " "].includes(
                  event.key,
                )
              )
                event.currentTarget.style.scrollSnapType = "";
            }}
          >
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
                        <span aria-hidden="true">{isOpen ? "⌄" : "›"}</span>
                        <span>{section.title}</span>
                        <small>
                          {readingSection === id ? "• " : ""}
                          {items.length || ""}
                        </small>
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
                      +
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
                        <RichContent context={{...folder,section:"section-0"}}
                          html={section.bodyHtml}
                          language={language}
                          openLink={openLink}
                        />
                      )}
                      {items
                        .filter(
                          (m) =>
                            !query ||
                            matches(m.title, m.bodyHtml) ||
                            matches(section.title, section.bodyHtml),
                        )
                        .map((m) =>
                          m.module === "forum" ? forumCards(m) : m.bodyHtml &&
                          ["page", "label"].includes(
                            m.module ?? "",
                          ) ? (
                            <article className="lmsInlineActivity" key={m.id}>
                              <h4>{m.title}</h4>
                              <RichContent context={{...folder,section:"section-0"}}
                                html={m.bodyHtml}
                                language={language}
                                openLink={openLink}
                              />
                            </article>
                          ) : (
                            row(m)
                          ),
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
                        const row = (note: LocalFile) => <button type="button" className="lmsItemRow" key={note.id} onClick={() => open({id:note.id,title:note.name.trim() || `${section.title} · ${t("Note")}`,kind:"note",section:note.section})}><span className="lmsItemGlyph">✎</span><span><b>{note.name.trim() || `${section.title} · ${t("Note")}`}</b><small>{t("Local note")}</small></span><span>›</span></button>;
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
            <button className="lmsViewerClose" type="button" aria-label={`${t("Close")} ${activeTab.title}`} title={t("Close")} onClick={() => closeTab(activeTab.id)}>×</button>
            <div className="lmsViewerBody">
              {!viewerActive &&
              activeTab?.kind !== "official" ? null : !activeTab ? (
                <div className="lmsViewerEmpty">
                  <span aria-hidden="true">▧</span>
                  <h2>{t("Choose a file or activity")}</h2>
                  <p>
                    {t(
                      "Course content stays on the left. Opened items appear here.",
                    )}
                  </p>
                </div>
              ) : activeTab.kind === "announcement" ? (
                <div className="lmsDocument"><div className="lmsDocumentHeading"><small>{formatLmsDate(announcement?.date || "", data.settings.locale)}</small><h2>{activeTab.title}</h2></div>
                {announcement && <RichContent context={{...folder,section:"section-0"}} html={announcement.html} language={language} openLink={openLink} />}
                </div>
              ) : activeTab.kind === "syllabus" ? (
                <div className="lmsDocument"><Suspense fallback={<p>{t("Loading…")}</p>}>
                  <SyllabusDetails url={activeTab.url!} fetchCourse={desktopBridge ? url => desktopBridge!.syllabusCourse(url, data.settings.locale === "ja" ? "ja" : "en") : undefined} />
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
                    <RichContent context={{...folder,section:"section-0"}}
                      html={material.bodyHtml}
                      language={language}
                      openLink={openLink}
                    />
                  )}
                  {(() => {
                    const url = safeUrl(
                      material?.resourceUrl ||
                        (material?.url && !material.url.includes("/mod/")
                          ? material.url
                          : undefined) ||
                        (activeTab.kind === "link" ? activeTab.url : undefined),
                    );
                    if (!url)
                      return material && !material.bodyHtml ? (
                        <p className="lmsQuiet">
                          {t(
                            "File details have not been synced. Refresh LMS to resolve this resource.",
                          )}
                        </p>
                      ) : null;
                    return (
                      <Suspense fallback={<p>{t("Loading preview…")}</p>}>
                        <FilePreview
                          key={url}
                          url={url}
                          title={activeTab.title}
                          autoSave={data.settings.autoSaveCourseFiles ?? false}
                          context={{
                            ...folder,
                            section: `section-${material?.sectionId ?? sections.find((s) => s.number === (material?.sectionNumber ?? material?.session))?.id ?? 0}`,
                          }}
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
