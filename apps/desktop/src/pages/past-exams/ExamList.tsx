import {
  createContext,
  useContext,
  useEffect,
  useEffectEvent,
  useRef,
  type ReactNode,
} from "react";
import { ResizableWorkspace } from "../../components/ResizableWorkspace";
import { lookup, t } from "../../i18n";
import type {
  ExamAction,
  ExamAnswer,
  ExamDocument,
  ExamSet,
  ExamSubject,
  ExamTeacher,
  ExamUploadStart,
} from "../../generated/persistence";

export type Call = (action: ExamAction) => Promise<ExamAnswer | undefined>;
export type OpenUpload = (start: ExamUploadStart) => void;
/** The choices a form offers, asked for each time one opens: Core reads
 * them from the service and keeps them for five minutes. A failure reaches
 * the toast like every call, and the next opening asks again. */
export const loadChoices = async (call: Call) => {
  const answer = await call({ kind: "choices" });
  return answer?.kind === "choices" ? answer.value : undefined;
};
/** Core names subjects in the person's language. */
export const subjectLabel = (subject: ExamSubject) => subject.name;
/** An occasion's name: the one given for an `other` occasion, else the
 * code's label; a code this build does not know reads as "other". */
export const occasionName = (occasion: string | null, text = "") =>
  text ||
  (occasion === null
    ? undefined
    : (lookup(`examShare.occasion.${occasion}`) ??
      t("examShare.occasion.other")));
/** A role's name, as `occasionName` for occasions. */
export const roleName = (role: string | null, text = "") =>
  text ||
  (role === null
    ? undefined
    : (lookup(`examShare.role.${role}`) ?? t("examShare.role.other")));
/** Who a sitting or document is from: every teacher, the names, or none
 * known. */
export const teacherLine = (item: {
  teachers: ExamTeacher[];
  allTeachers: boolean;
}) =>
  item.allTeachers
    ? t("examShare.allTeachers")
    : item.teachers.map(({ name }) => name).join(" / ") ||
      t("examShare.unknownTeacher");
/** A sitting's or document's occasion: "Quiz 3 B", else the file name. */
export const occasionLabel = (
  item: Pick<
    ExamSet,
    "occasion" | "occasionText" | "occasionNumber" | "variant"
  >,
  fallback: string,
) =>
  [
    occasionName(item.occasion, item.occasionText),
    item.occasionNumber > 0 ? item.occasionNumber : null,
    item.variant,
  ]
    .filter(Boolean)
    .join(" ") || fallback;
export const documentOccasion = (document: ExamDocument) =>
  occasionLabel(document, document.fileName);

/** What every tab's view takes from the page. */
export type ViewProps = {
  call: Call;
  open: (id: string) => void;
  opening?: string;
  reading?: string;
  pane: ReactNode;
  generation: number;
};

/** Runs `reload` each time the page asks every view to read Core again,
 * but not for the generation the view mounted at. */
export function useReload(generation: number, reload: () => void) {
  const seen = useRef(generation);
  const run = useEffectEvent(reload);
  useEffect(() => {
    if (seen.current === generation) return;
    seen.current = generation;
    run();
  }, [generation]);
}

/** What the page puts around the list column: its title row and tabs at
 * the top, the account and the partner's site at the foot. */
export const ExamChrome = createContext<{
  head: ReactNode;
  foot: ReactNode;
} | null>(null);

/** The list on the left and what it opens on the right; one layout for all
 * three tabs. Each column runs up to the window's top row: the list's with
 * the page title, the reader's with its own tools (ReaderPane). */
export function Workspace({ children }: { children: [ReactNode, ReactNode] }) {
  const chrome = useContext(ExamChrome);
  return (
    <ResizableWorkspace
      id="past-exams"
      enabled
      firstSize={320}
      firstMin={260}
      firstMax="45%"
      secondMin="40%"
    >
      <div className="examColumn">
        {chrome?.head}
        {children[0]}
        {chrome?.foot}
      </div>
      <div className="examColumn examReaderColumn">{children[1]}</div>
    </ResizableWorkspace>
  );
}

/** One list row: a title, a muted line under it, and optional trailing UI. */
export function Row({
  title,
  meta,
  current,
  pending,
  disabled,
  onClick,
  trailing,
}: {
  title: ReactNode;
  meta?: ReactNode;
  current?: boolean;
  pending?: boolean;
  disabled?: boolean;
  onClick?: () => void;
  trailing?: ReactNode;
}) {
  return (
    <li className="examRow" aria-current={current || undefined}>
      <button
        type="button"
        className="examRowMain"
        disabled={disabled}
        aria-busy={pending || undefined}
        onClick={onClick}
      >
        <strong>{title}</strong>
        {meta && <small>{meta}</small>}
      </button>
      {trailing}
    </li>
  );
}
