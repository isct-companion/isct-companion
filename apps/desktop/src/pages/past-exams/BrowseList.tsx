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
import { lookup, t } from "../../i18n";
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
  occasionLabel,
  subjectLabel,
  teacherNames,
  useReload,
  type Call,
  type OpenUpload,
  type ViewProps,
} from "./ExamList";

/** A document's role on its pill, with the number Core gives it when its
 * sitting holds several of that role. */
const roleLabel = (document: ExamDocument) =>
  [
    (document.role && lookup(`examShare.role.${document.role}`)) ||
      t("examShare.file"),
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
  choices,
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
  choices?: ExamChoices;
  takeCourse: () => string | undefined;
  openUpload: OpenUpload;
}) {
  const [query, setQuery] = useState("");
  const [debounced] = useDebounceValue(query.trim(), 300);
  const [hits, setHits] = useState<{ query: string; items: ExamSubject[] }>();
  const [expanded, setExpanded] = useState("");
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

  const shownUrl = pages[expanded]?.page?.webUrl;
  useEffect(() => setViewUrl(shownUrl), [shownUrl, setViewUrl]);
  // Only the open panel reads again; the others load when next opened.
  useReload(generation, () => {
    const entry = entries?.find((e) => e.key === expanded);
    setPages((all) => (all[expanded] ? { [expanded]: all[expanded] } : {}));
    if (expanded) void load(expanded, reloadAction(expanded, entry));
  });

  return (
    <Workspace>
      <div className="examPane">
        <Input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.currentTarget.value)}
          maxLength={choices?.limits.query ?? 100}
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
                type="single"
                collapsible
                value={expanded}
                onValueChange={(key) => {
                  setExpanded(key);
                  const entry = entries.find((e) => e.key === key);
                  if (entry && !pages[key]?.page) void load(key, entry.action);
                }}
                className="examPanels"
              >
                {entries.map((entry) => (
                  <Accordion.Item
                    key={entry.key}
                    value={entry.key}
                    disabled={!entry.action}
                    className="examPanel"
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
                        choices={choices}
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
  choices,
  call,
  open,
  opening,
  reading,
  retry,
  choose,
  openUpload,
}: {
  state?: PanelState;
  choices?: ExamChoices;
  call: Call;
  open: (id: string) => void;
  opening?: string;
  reading?: string;
  retry: () => void;
  choose: (code: string) => void;
  openUpload: OpenUpload;
}) {
  const [requesting, setRequesting] = useState(false);
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
          withTeachers={!set.mine}
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
          disabled={!choices}
          onClick={() =>
            openUpload({ kind: "subject", code: page.subject.code })
          }
        >
          {t("examShare.uploadHere")}
        </Button>
        <Button
          variant="subtle"
          size="compact-sm"
          disabled={!choices}
          onClick={() => setRequesting(true)}
        >
          {t("examShare.newRequest")}
        </Button>
      </div>
      {choices && (
        <RequestDialog
          opened={requesting}
          subject={page}
          choices={choices}
          close={() => setRequesting(false)}
          submit={async (draft) => {
            if (await call({ kind: "createRequest", draft })) {
              setRequesting(false);
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
  withTeachers,
}: {
  set: ExamSet;
  open: (id: string) => void;
  opening?: string;
  reading?: string;
  withTeachers: boolean;
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
          {withTeachers && <small>{teacherNames(set.teachers)}</small>}
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
