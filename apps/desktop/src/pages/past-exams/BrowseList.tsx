import { useCallback, useEffect, useRef, useState } from "react";
import {
  BookOpen,
  ChevronDown,
  ChevronRight,
  FileText,
  LoaderCircle,
  Search,
} from "lucide-react";
import { Accordion } from "radix-ui";
import { useDebounceValue } from "usehooks-ts";
import {
  Button,
  EmptyState,
  ErrorBanner,
  Input,
  Skeleton,
} from "../../components/ui";
import { notify } from "../../components/notifications/notify";
import { useScheduleQuery } from "../../platform/core-query";
import { t } from "../../i18n";
import type {
  ExamAction,
  ExamChoices,
  ExamCourseList,
  ExamDocument,
  ExamSet,
  ExamSubject,
  ExamSubjectPage,
} from "../../generated/persistence";
import { RequestDialog } from "./ExamDialogs";
import {
  Row,
  Workspace,
  loadChoices,
  occasionLabel,
  roleName,
  subjectLabel,
  teacherLine,
  useReload,
  type Call,
  type OpenUpload,
  type ViewProps,
} from "./ExamList";

/** A document's role on its pill, with the number Core gives it when its
 * sitting holds several of that role. */
const roleLabel = (document: ExamDocument) =>
  [
    roleName(document.role, document.roleText) ?? t("examShare.file"),
    document.number,
  ]
    .filter(Boolean)
    .join(" ");
/** One course or search hit on the left, and how Core opens its subject:
 * a course by its id, so Core can put its own class's teachers first. */
type Entry = {
  key: string;
  title: string;
  meta: string;
  action?: Extract<ExamAction, { kind: "subject" | "courseSubject" }>;
  /** This quarter's exams (as the app keeps them) include this course. */
  hasExam?: boolean;
};
/** A panel's content: the subject, the subjects a course maps to, or a failure. */
type PanelState = {
  page?: ExamSubjectPage;
  subjects?: ExamSubject[];
  failed?: boolean;
};

/** Left: this quarter's courses or a search, each subject an expansion
 * panel of its sittings. Right: always the reader. */
export function BrowseList({
  active,
  takeCourse,
  openUpload,
  call,
  open,
  opening,
  reading,
  pane,
  generation,
  setViewUrl,
}: ViewProps & {
  active: boolean;
  /** Names the expanded subject's page on the website. */
  setViewUrl: (url: string | undefined) => void;
  takeCourse: () => string | undefined;
  openUpload: OpenUpload;
}) {
  const [query, setQuery] = useState("");
  const [debounced] = useDebounceValue(query.trim(), 300);
  const [hits, setHits] = useState<{ query: string; items: ExamSubject[] }>();
  // The panel opened last: its page is what "Open on web" and a reload
  // follow. Panels open several at once (`manual` and `openKeys` below).
  const [expanded, setExpanded] = useState("");
  // The person's own open or close of a panel, which outranks the default.
  const [manual, setManual] = useState<Record<string, boolean>>({});
  const [showEmpty, setShowEmpty] = useState(false);
  const [pages, setPages] = useState<Record<string, PanelState>>({});
  /** A course opened from elsewhere that this quarter's list lacks. */
  const [extra, setExtra] = useState<string>();
  const searchSeq = useRef(0);

  useEffect(() => {
    if (!debounced) return setHits(undefined);
    const seq = ++searchSeq.current;
    void call({ kind: "search", query: debounced }).then((answer) => {
      if (seq !== searchSeq.current) return;
      setHits({
        query: debounced,
        items: answer?.kind === "subjects" ? answer.value : [],
      });
    });
  }, [debounced, call]);

  const searching = !!query.trim();
  const timetable = useScheduleQuery<ExamCourseList>(
    { kind: "examCourses" },
    active && !searching,
  );

  const load = useCallback(
    async (key: string, action: Entry["action"]) => {
      if (!action) return;
      // A refresh keeps what is shown until the new answer arrives.
      setPages((all) => ({ ...all, [key]: all[key]?.page ? all[key] : {} }));
      const answer = await call(action);
      setPages((all) => ({
        ...all,
        [key]:
          answer?.kind === "subject"
            ? { page: answer.value }
            : answer?.kind === "subjects"
              ? { subjects: answer.value }
              : all[key]?.page
                ? all[key]
                : { failed: true },
      }));
    },
    [call],
  );
  /** Reads an open subject again in the order it was opened with. */
  const reloadAction = (key: string, entry?: Entry): Entry["action"] => {
    const page = pages[key]?.page;
    return page
      ? { kind: "subject", code: page.subject.code, classId: page.classId }
      : entry?.action;
  };

  // A course opened from its course page expands at once.
  useEffect(() => {
    const id = takeCourse();
    if (!id) return;
    const key = `course:${id}`;
    setQuery("");
    setExtra(id);
    setExpanded(key);
    setManual((all) => ({ ...all, [key]: true }));
    void load(key, { kind: "courseSubject", courseId: id });
  }, [takeCourse, load]);

  const courses = timetable.value?.courses;
  const courseEntries: Entry[] | undefined = courses?.map((course) => ({
    key: `course:${course.courseId}`,
    title: course.title,
    meta:
      [course.subjectCodes.join(" / "), course.instructor]
        .filter(Boolean)
        .join(" · ") || t("examShare.noSubjectCode"),
    action: course.subjectCodes.length
      ? { kind: "courseSubject", courseId: course.courseId }
      : undefined,
    hasExam: course.hasExam,
  }));
  const extraPage = extra ? pages[`course:${extra}`]?.page : undefined;
  if (
    courseEntries &&
    extra &&
    !courseEntries.some((entry) => entry.key === `course:${extra}`)
  )
    courseEntries.unshift({
      key: `course:${extra}`,
      title: extraPage ? subjectLabel(extraPage.subject) : t("common.loading"),
      meta: extraPage?.subject.code ?? "",
      action: { kind: "courseSubject", courseId: extra },
      hasExam: true,
    });
  const entries: Entry[] | undefined = searching
    ? hits?.query === debounced && debounced === query.trim()
      ? hits.items.map((subject) => ({
          key: `search:${subject.code}`,
          title: subjectLabel(subject),
          meta: subject.code,
          action: { kind: "subject", code: subject.code, classId: null },
        }))
      : undefined
    : courseEntries;

  // The quarter's courses with an exam are read up front, to tell which
  // have past exams on ScienceTokyoExam.
  useEffect(() => {
    if (searching || !courseEntries) return;
    for (const entry of courseEntries)
      if (entry.hasExam && entry.action && !pages[entry.key])
        void load(entry.key, entry.action);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searching, courses]);
  const hasMaterials = (key: string) => {
    const page = pages[key]?.page;
    return !page || page.sets.length > 0;
  };
  // A course with no exam this quarter is hidden behind one line until the
  // person asks for it; a search lists everything it finds.
  const hidden = searching
    ? []
    : (entries ?? []).filter((entry) => !entry.hasExam);
  const listed = (entries ?? []).filter(
    (entry) => showEmpty || !hidden.includes(entry),
  );
  // A course with an exam opens by itself, unless its subject turns out to
  // have no past exams; the person's own open or close outranks both.
  const openKeys = listed
    .filter(
      (entry) =>
        manual[entry.key] ??
        (!searching &&
          !!entry.hasExam &&
          !!entry.action &&
          hasMaterials(entry.key)),
    )
    .map((entry) => entry.key);

  const shownUrl = pages[expanded]?.page?.webUrl;
  useEffect(() => setViewUrl(shownUrl), [shownUrl, setViewUrl]);
  // Open panels read again; the others load when next opened.
  useReload(generation, () => {
    setPages((all) =>
      Object.fromEntries(
        openKeys.filter((key) => all[key]).map((key) => [key, all[key]]),
      ),
    );
    for (const key of openKeys)
      void load(
        key,
        reloadAction(
          key,
          entries?.find((e) => e.key === key),
        ),
      );
  });

  return (
    <Workspace>
      <div className="examPane">
        <Input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.currentTarget.value)}
          placeholder={t("examShare.search")}
          aria-label={t("examShare.search")}
          rightSection={<Search size={14} strokeWidth={1.75} aria-hidden />}
        />
        <div className="scroll">
          {!searching && timetable.error ? (
            <ErrorBanner onRetry={timetable.retry}>
              {timetable.error}
            </ErrorBanner>
          ) : !entries ? (
            <Skeleton count={5} />
          ) : entries.length ? (
            <>
              {!searching && (
                <h2 className="examListHeading">
                  {t("examShare.currentCourses")}
                </h2>
              )}
              <Accordion.Root
                type="multiple"
                value={openKeys}
                onValueChange={(keys) => {
                  const opened = keys.find((key) => !openKeys.includes(key));
                  setManual((all) => {
                    const next = { ...all };
                    for (const entry of entries)
                      if (
                        keys.includes(entry.key) !==
                        openKeys.includes(entry.key)
                      )
                        next[entry.key] = keys.includes(entry.key);
                    return next;
                  });
                  if (!opened) return;
                  setExpanded(opened);
                  const entry = entries.find((e) => e.key === opened);
                  if (entry && !pages[opened]?.page)
                    void load(opened, entry.action);
                }}
                className="examPanels"
              >
                {listed.map((entry) => (
                  <Accordion.Item
                    key={entry.key}
                    value={entry.key}
                    disabled={!entry.action}
                    className="examPanel"
                    data-empty={
                      pages[entry.key]?.page && !hasMaterials(entry.key)
                        ? ""
                        : undefined
                    }
                  >
                    <Accordion.Header asChild>
                      <h3>
                        <Accordion.Trigger className="examPanelTrigger">
                          <span>
                            <strong>{entry.title}</strong>
                            <small>{entry.meta}</small>
                          </span>
                          <ChevronDown
                            size={16}
                            strokeWidth={1.75}
                            aria-hidden
                          />
                        </Accordion.Trigger>
                      </h3>
                    </Accordion.Header>
                    <Accordion.Content className="examPanelContent">
                      <SubjectSets
                        state={pages[entry.key]}
                        call={call}
                        open={open}
                        opening={opening}
                        reading={reading}
                        retry={() =>
                          void load(entry.key, reloadAction(entry.key, entry))
                        }
                        choose={(code) =>
                          void load(entry.key, {
                            kind: "subject",
                            code,
                            classId: null,
                          })
                        }
                        openUpload={openUpload}
                      />
                    </Accordion.Content>
                  </Accordion.Item>
                ))}
              </Accordion.Root>
              {hidden.length > 0 && (
                <button
                  type="button"
                  className="examOthers examWithout"
                  aria-expanded={showEmpty}
                  onClick={() => setShowEmpty(!showEmpty)}
                >
                  <ChevronRight size={14} strokeWidth={1.75} aria-hidden />
                  {t("examShare.withoutExams", { count: hidden.length })}
                </button>
              )}
            </>
          ) : (
            <EmptyState compact icon={Search}>
              {searching
                ? t("examShare.noSubjects")
                : t("examShare.noCurrentCourses")}
            </EmptyState>
          )}
        </div>
      </div>
      <div className="examPane examDetail">
        {pane || (
          <EmptyState icon={expanded ? FileText : BookOpen}>
            {t(expanded ? "examShare.pickDocument" : "examShare.pickSubject")}
          </EmptyState>
        )}
      </div>
    </Workspace>
  );
}

/** A subject's sittings inside its panel: the person's own class's
 * teachers first, the rest behind one toggle, then a way to share or ask for
 * what is missing. */
function SubjectSets({
  state,
  call,
  open,
  opening,
  reading,
  retry,
  choose,
  openUpload,
}: {
  state?: PanelState;
  call: Call;
  open: (id: string) => void;
  opening?: string;
  reading?: string;
  retry: () => void;
  choose: (code: string) => void;
  openUpload: OpenUpload;
}) {
  /** The request form's choices while it is open. */
  const [requesting, setRequesting] = useState<ExamChoices>();
  const [othersOpen, setOthersOpen] = useState(false);
  if (state?.failed)
    return (
      <ErrorBanner onRetry={retry}>{t("examShare.loadFailed")}</ErrorBanner>
    );
  if (state?.subjects)
    return state.subjects.length ? (
      <ul className="examList">
        {state.subjects.map((subject) => (
          <Row
            key={subject.code}
            title={subjectLabel(subject)}
            meta={subject.code}
            onClick={() => choose(subject.code)}
          />
        ))}
      </ul>
    ) : (
      <p className="examQuiet">{t("examShare.noSubjects")}</p>
    );
  const page = state?.page;
  if (!page) return <Skeleton count={3} />;
  const sets = page.sets;
  // Core knows the person's class only when the subject came from a course.
  const grouped = page.classId !== null;
  const mine = sets.filter((set) => !grouped || set.mine);
  const others = grouped ? sets.filter((set) => !set.mine) : [];
  const rows = (list: ExamSet[]) => (
    <ul className="examSets">
      {list.map((set) => (
        <SetRow
          key={set.documents[0]?.id}
          set={set}
          open={open}
          opening={opening}
          reading={reading}
        />
      ))}
    </ul>
  );
  return (
    <>
      {!sets.length ? (
        <p className="examQuiet">{t("examShare.noDocuments")}</p>
      ) : (
        <>
          {mine.length > 0
            ? rows(mine)
            : grouped && (
                <p className="examQuiet">{t("examShare.noOwnTeacher")}</p>
              )}
          {others.length > 0 && (
            <>
              <button
                type="button"
                className="examOthers"
                aria-expanded={othersOpen}
                onClick={() => setOthersOpen(!othersOpen)}
              >
                <ChevronRight size={14} strokeWidth={1.75} aria-hidden />
                {t("examShare.otherTeachers", { count: others.length })}
              </button>
              {othersOpen && rows(others)}
            </>
          )}
        </>
      )}
      <div className="examPanelActions">
        <Button
          variant="subtle"
          size="compact-sm"
          onClick={() =>
            openUpload({ kind: "subject", code: page.subject.code })
          }
        >
          {t("examShare.uploadHere")}
        </Button>
        <Button
          variant="subtle"
          size="compact-sm"
          onClick={() => void loadChoices(call).then(setRequesting)}
        >
          {t("examShare.newRequest")}
        </Button>
      </div>
      {requesting && (
        <RequestDialog
          subject={page}
          choices={requesting}
          close={() => setRequesting(undefined)}
          submit={async (draft) => {
            if (await call({ kind: "createRequest", draft })) {
              setRequesting(undefined);
              notify.info(t("examShare.requestCreated"));
            }
          }}
        />
      )}
    </>
  );
}

/** One sitting. The sitting is a label; each of its documents (question,
 * answer, …) is its own pill button, and only the one being opened spins. */
function SetRow({
  set,
  open,
  opening,
  reading,
}: {
  set: ExamSet;
  open: (id: string) => void;
  opening?: string;
  reading?: string;
}) {
  const current = set.documents.some((d) => d.id === reading);
  return (
    <li className="examSet" aria-current={current || undefined}>
      <div className="examSetMain">
        <FileText size={15} strokeWidth={1.75} aria-hidden />
        <span className="examSetYear">{set.academicYear ?? "—"}</span>
        <span className="examSetText">
          <strong>
            {occasionLabel(set, set.documents[0]?.fileName ?? "")}
          </strong>
          {set.showTeachers && <small>{teacherLine(set)}</small>}
        </span>
      </div>
      <span className="examSetRoles">
        {set.documents.map((document) => (
          <button
            key={document.id}
            type="button"
            className="examRole"
            data-role={document.role ?? undefined}
            aria-pressed={document.id === reading}
            aria-busy={document.id === opening || undefined}
            title={document.note ?? document.fileName}
            onClick={() => open(document.id)}
          >
            {document.id === opening && (
              <LoaderCircle
                size={12}
                strokeWidth={2}
                className="examSpin"
                aria-hidden
              />
            )}
            {roleLabel(document)}
          </button>
        ))}
      </span>
    </li>
  );
}
