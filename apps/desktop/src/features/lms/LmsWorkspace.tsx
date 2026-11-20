import { downloadLimit } from "../../lib/course-file-policy";
import { ExamCards } from "../../components/exams/ExamCards";
import { GuideTip } from "../../components/guide";
import { useResizeObserver } from "usehooks-ts";
import { LmsReader } from "./LmsReader";
import { ResizableWorkspace } from "../../components/ResizableWorkspace";
import { type OpenedContent } from "./reading-state";
import { useCourseNotes } from "./useCourseNotes";
import {
  ImageThumbnail,
  RichContent,
  type FileAccess,
  type LinkedFolders,
} from "./RichContent";
import { FolderRow } from "./FolderRow";
import "./lms.css";
import {
  lazy,
  Suspense,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import type { Assignment, Course, Material, Workspace } from "../../lib/model";
import type { LessonItem } from "../../generated/persistence";
import { t } from "../../i18n";
import { detectPlatform } from "../../platform/capabilities";
import { useClock } from "../../platform/use-clock";
import type { PageProgress } from "../../platform/sync";
import { ageText } from "../../lib/sync-text";
import { announcements, formatLmsDate, contentText, safeUrl } from "./content";
import { FileTile, LmsIcon } from "./LmsIcon";
import {
  useScheduleQueries,
  useScheduleQuery,
  type AssignmentDetail,
  type CourseContent,
  type CourseDetail,
  type TaskView,
} from "../../platform/core-query";
import { extension, fileFamily, listsAsFolder } from "../../lib/file-types";
import { withRecords } from "../../platform/records";
import { moveNote, type LocalFile } from "./files";
import {
  ACTIVITIES,
  useOutlineReadingState,
  useOutlineReading,
  useOutlineEntrance,
  type Section,
} from "./useOutlineReading";
import { useLmsReader } from "./useLmsReader";
import { LmsAssignmentCard } from "./LmsAssignmentCard";

const FilePopover = lazy(() =>
  import("./FilePreview").then((m) => ({ default: m.FilePopover })),
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

export function LmsWorkspaceBody({
  course,
  data,
  entry,
  active,
  schoolWebAvailable,
  refresh,
  progress,
  updatedAt,
  markLesson,
  headerStart,
  openPastExams,
  openWeb,
  openAccounts,
}: {
  course: Course;
  data: Workspace;
  entry: CourseEntry;
  active: boolean;
  schoolWebAvailable: boolean;
  headerStart?: ReactNode;
  openPastExams?: () => void;
  /** Opens a school page on the Web page. */
  openWeb?: (url: string) => void;
  openAccounts?: () => void;
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
  const outlineState = useOutlineReadingState(key);
  const {
    saved,
    expanded,
    setExpanded,
    readingSection,
    query,
    setQuery,
    outline,
  } = outlineState;
  const [opened, setOpened] = useState<OpenedContent | undefined>(
    saved?.opened,
  );
  const selected = opened?.id;
  const [openFolders, setOpenFolders] = useState<string[]>([]);
  /** Items Core answered as a ZIP though their address did not say so. */
  const [archiveItems, setArchiveItems] = useState<string[]>([]);
  const [openDetails, setOpenDetails] = useState<string[]>([]);
  const flip = (list: string[], id: string) =>
    list.includes(id) ? list.filter((item) => item !== id) : [...list, id];
  const [original, setOriginal] = useState(false);
  // Outline and viewer take turns when the workspace pane is narrow. A saved
  // reader restored into a narrow pane starts on the viewer, not hidden.
  const [viewing, setViewing] = useState(Boolean(saved?.opened));
  const workspaceRef = useRef<HTMLDivElement>(null!);
  const workspaceSize = useResizeObserver({ ref: workspaceRef });
  const narrow =
    (workspaceSize.width ?? 0) > 0 && (workspaceSize.width ?? 0) < 640;
  const viewerActive = active && (!narrow || viewing);

  const limit = downloadLimit(data.settings);
  const files = useMemo<FileAccess>(
    () => ({ courseId: course.id }),
    [course.id],
  );
  /** Assignment workspaces are a desktop feature (folders, VS Code). */
  const workspaces = detectPlatform() === "native-desktop";
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
  // Exams first; the timetable's exam-week classes follow as their own group.
  const courseExams = [
    ...(courseDetail?.exams ?? []),
    ...(courseDetail?.examClasses ?? []),
  ];
  // The course's own syllabus reads as it is now: Core's address follows the
  // content language and its title the UI language. What was saved when it
  // opened is only the fallback until the course detail arrives.
  const shown =
    opened?.id === "syllabus"
      ? { ...opened, title: t("common.syllabus"), url: syllabus ?? opened.url }
      : opened;
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
        summaryHtml: original
          ? (lesson.originalSummaryHtml ?? lesson.summaryHtml)
          : lesson.summaryHtml,
        items: original
          ? lesson.items.map((item) => ({
              ...item,
              bodyHtml: item.originalBodyHtml ?? item.bodyHtml,
            }))
          : lesson.items,
      })),
    [content, original],
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
            ? announcements(item.bodyHtml ?? undefined).map((post) => ({
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
  const classAt = content?.classAt ?? undefined;
  /** The lesson the course is at, for marking rows of schedule tables. */
  const at = lessons.find((l) => l.section === content?.currentSection);
  const atIndex = lessons.findIndex((l) => l.id === at?.id);
  const reading = useOutlineReading(
    outlineState,
    sections,
    lessons,
    at,
    opened,
  );
  const {
    pinned,
    paged,
    shownId,
    lessonId,
    rail,
    railIndicator,
    goLesson,
    revealAssignment,
    recordPosition,
    followScroll,
    restAfterScroll,
  } = reading;
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
        ? t("lms.lessonNumber", { number: lesson.label })
        : lesson.label,
    );
  };
  const pendingIn = (section: Section) =>
    section.assignmentIds.some((id) => pendingAssignmentIds.has(id));
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
  const reader = useLmsReader({
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
  });
  const {
    open,
    openMaterial,
    openPage,
    materialAt,
    openLink,
    fileMenu,
    closeFileMenu,
    material,
    materialBody,
  } = reader;
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
  } = useCourseNotes(course.id, noteSections, active, open);
  /**
   * A link in course text that is a ZIP (its own address, or the resource it
   * points at) is that ZIP's folder row, in place of its card.
   */
  const linkedFolders: LinkedFolders = {
    of: (url) => {
      const material = materialAt(url);
      const zip = material?.resourceUrl ?? url;
      return listsAsFolder(zip) ? zip : undefined;
    },
    render: (url, title) => (
      <FolderRow
        title={title}
        url={url}
        archive
        courseId={course.id}
        limit={limit}
        openFile={(fileUrl, name, anchor) => openLink(fileUrl, name, anchor)}
        openPage={(page) => openPage(page, title)}
      />
    ),
  };
  useOutlineEntrance({
    key,
    deviceId: data.deviceId,
    state: outlineState,
    reading,
    contentRead,
    entry,
    assignments,
    sections,
    lessons,
    at,
    opened,
    setOpenDetails,
    setViewing,
  });
  const toggle = (id: string) =>
    setExpanded((previous) =>
      previous.includes(id)
        ? previous.filter((item) => item !== id)
        : [...previous, id],
    );
  const assignmentContent = (a: Assignment) => {
    const detail = details.get(a.id);
    const assignmentState = taskView?.states?.[a.id] ?? detail?.state;
    const availability =
      taskView?.handInAvailability?.[a.id] ?? detail?.handInAvailability;
    const deadline = detail?.deadline ?? taskView?.deadlines?.[a.id];
    const showBody = openDetails.includes(a.id) || Boolean(query.trim());
    return (
      <LmsAssignmentCard
        key={a.id}
        assignment={a}
        detail={detail}
        assignmentState={assignmentState}
        availability={availability}
        deadline={deadline}
        original={original}
        showBody={showBody}
        query={query}
        toggleDetails={() => setOpenDetails((list) => flip(list, a.id))}
        locale={data.settings.locale}
        open={open}
        files={files}
        openLink={openLink}
        linkedFolders={linkedFolders}
        workspaces={workspaces}
      />
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
            {t("lms.noAnnouncementsYet")}{" "}
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
                {t("common.openInLMS")}
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
    // A folder or a ZIP opens in place: its files list under its row.
    const archive = family === "archive" || archiveItems.includes(m.id);
    const listed =
      m.module === "folder"
        ? safeUrl(m.url)
        : archive
          ? safeUrl(m.resourceUrl || m.url)
          : undefined;
    if (listed)
      return (
        <FolderRow
          key={m.id}
          title={m.title}
          url={listed}
          archive={archive}
          courseId={course.id}
          limit={limit}
          openFile={(fileUrl, name, anchor) => openLink(fileUrl, name, anchor)}
          openPage={(page) => openPage(page, m.title)}
          open={openFolders.includes(m.id)}
          onToggle={() => setOpenFolders((list) => flip(list, m.id))}
        />
      );
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
          <small>{ext ? ext.toUpperCase() : (m.module ?? t("lms.file"))}</small>
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
            aria-label={`${t("lms.newNote")} · ${section.title}`}
            title={t("lms.newNote")}
            onClick={() => void newNote(section)}
          >
            <LmsIcon name="plus" />
          </button>
        </div>
        {!isOpen && section.summaryHtml && (
          <p className="lmsSectionPreview">{t("lms.sectionCollapsed")}</p>
        )}
        {isOpen && (
          <div id={`lms-panel-${section.id}`} className="lmsSectionBody">
            {section.summaryHtml && (
              <RichContent
                files={files}
                html={section.summaryHtml}
                openLink={openLink}
                folders={linkedFolders}
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
                        openLink={openLink}
                        folders={linkedFolders}
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
                      openLink={openLink}
                      folders={linkedFolders}
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
                        note.name.trim() ||
                        `${section.title} · ${t("lms.note")}`,
                      kind: "note",
                      section: note.section,
                    })
                  }
                >
                  <FileTile family="note" />
                  <span>
                    <b>
                      {note.name.trim() ||
                        `${section.title} · ${t("lms.note")}`}
                    </b>
                    <small>{t("lms.localNoteStoredOnlyOn")}</small>
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
                        {t("lms.previousNotes")} · {sectionNotes.length - 1}
                      </summary>
                      {sectionNotes.slice(1).map(row)}
                    </details>
                  )}
                </>
              );
            })()}
            {!section.summaryHtml && !items.length && (
              <p className="lmsQuiet">{t("lms.noContentInThisSection")}</p>
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
        <div className="lmsCourseIdentity">
          <h1 title={course.title}>{course.title}</h1>
          <small>{course.code}</small>
        </div>
        {/* One group, so the actions move to their own row together when
            the title would otherwise get less than its minimum width. */}
        <div className="lmsHeaderActions">
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
                  title: t("common.syllabus"),
                  kind: "syllabus",
                  url: syllabus,
                })
              }
            >
              <LmsIcon name="doc" size={14} /> {t("common.syllabus")}
            </button>
          )}
          {openPastExams && (
            <button
              type="button"
              className="lmsRefresh"
              onClick={openPastExams}
            >
              {t("examShare.openCourse")}
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
                    ? `${t("lms.showingDataFromAge", {
                        age: ageText(
                          updatedAt,
                          timelineNow,
                          data.settings.locale,
                        ),
                      })} · `
                    : null}
                  {progress.total !== null
                    ? t("lms.updatingThisCourseDoneTotal", {
                        done: progress.done,
                        total: progress.total,
                      })
                    : t("lms.updatingThisCourse")}
                </span>
              )}
              {refresh && (
                <button
                  type="button"
                  className="lmsRefresh"
                  disabled={progress != null || opened?.kind === "official"}
                  onClick={refresh}
                >
                  {t("lms.pullFromLMS")}
                </button>
              )}
            </div>
          )}
        </div>
      </header>
      <div className="lmsWorkspaceTools">
        {lessons.length > 0 && !query.trim() && (
          <GuideTip id="lesson-mark" text="lms.doubleClickALessonNumber">
            <nav
              className="lmsLessonRail"
              aria-label={t("lms.lessons")}
              ref={rail}
            >
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
                  {t("lms.activities")}
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
                aria-label={t("lms.searchCourseContent")}
                placeholder={t("lms.searchThisCourse")}
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
                  {expanded.length ? t("lms.expandAll") : t("lms.collapseAll")}
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
                    courseId={course.id}
                    limit={limit}
                    anchor={fileMenu.anchor}
                    onClose={closeFileMenu}
                    openPage={(page) => openPage(page, fileMenu.title)}
                  />
                </Suspense>
              )}
              {noteError && <p role="alert">{noteError}</p>}
              {deletedNote && (
                <div className="lmsNoteUndo" role="status">
                  <span>{t("lms.noteDeleted")}</span>
                  <button
                    type="button"
                    onClick={() => {
                      const note = deletedNote;
                      void moveNote(
                        { courseId: course.id, section: note.section },
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
                    {t("common.undo")}
                  </button>
                </div>
              )}
              {courseExams.length > 0 && (
                <section
                  className="lmsCourseExams"
                  aria-label={t("common.exams")}
                >
                  <h2>{t("common.exams")}</h2>
                  <ExamCards
                    exams={courseExams}
                    courses={data.courses.filter((row) => row.id === course.id)}
                  />
                </section>
              )}
              {!paged && <h2>{t("syncStatus.dataset.courseContent")}</h2>}
              {sections.map((section) => sectionView(section))}
              {!sections.length && (
                <p className="lmsQuiet">{t("lms.noCourseContentSyncedYet")}</p>
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
                      <h2>{t("common.assignments")}</h2>
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
                {original
                  ? t("lms.usePreferredLanguage")
                  : t("lms.showOriginal")}
              </button>
            )}
          </aside>
          {opened && shown && (
            <LmsReader
              opened={opened}
              shown={shown}
              viewerActive={viewerActive}
              course={course}
              locale={data.settings.locale}
              announcement={announcement}
              files={files}
              linkedFolders={linkedFolders}
              material={material}
              materialBody={materialBody}
              limit={limit}
              schoolWebAvailable={schoolWebAvailable}
              openAccounts={openAccounts}
              refreshNotes={refreshNotes}
              setDeletedNote={setDeletedNote}
              reader={reader}
            />
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
