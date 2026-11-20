import { useEffect, useEffectEvent, useRef, type ReactNode } from "react";
import { ResizableWorkspace } from "../../components/ResizableWorkspace";
import { lookup, t } from "../../i18n";
import type {
  ExamAction,
  ExamAnswer,
  ExamDocument,
  ExamSet,
  ExamSubject,
  ExamUploadStart,
} from "../../generated/persistence";

export type Call = (action: ExamAction) => Promise<ExamAnswer | undefined>;
export type OpenUpload = (start: ExamUploadStart) => void;
/** Core names subjects in the person's language. */
export const subjectLabel = (subject: ExamSubject) => subject.name;
/** A sitting's or document's occasion: "Quiz 3 B", else the file name. */
export const occasionLabel = (
  item: Pick<ExamSet, "occasion" | "occasionNumber" | "variant">,
  fallback: string,
) =>
  [
    item.occasion && lookup(`examShare.occasion.${item.occasion}`),
    item.occasionNumber > 1 ? item.occasionNumber : null,
    item.variant,
  ]
    .filter(Boolean)
    .join(" ") || fallback;
export const documentOccasion = (document: ExamDocument) =>
  occasionLabel(document, document.fileName);
export const teacherNames = (teachers: { name: string }[]) =>
  teachers.map(({ name }) => name).join(" / ") || t("examShare.unknownTeacher");

/** What every tab's view takes from the page. */
export type ViewProps = {
  call: Call;
  open: (id: string) => void;
  opening?: string;
  reading?: string;
  pane: ReactNode;
  generation: number;
  setViewUrl: (url: string | undefined) => void;
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

/** The list on the left and what it opens on the right; one layout for all three tabs. */
export function Workspace({ children }: { children: [ReactNode, ReactNode] }) {
  return (
    <ResizableWorkspace
      id="past-exams"
      enabled
      firstSize={320}
      firstMin={260}
      firstMax="45%"
      secondMin="40%"
    >
      {children}
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
