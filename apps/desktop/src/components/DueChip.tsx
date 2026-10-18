import type { TaskDeadlineView } from "../generated/persistence";
import { deadlineText } from "../lib/deadline-text";
import { t } from "../i18n";

/** The right-hand state of an assignment row, shared by Today and Lecture
 * Desk: a submitted assignment says so instead of repeating its deadline; an
 * open one counts down; a missed one shows when it closed. */
export function DueChip({
  due,
  submitted,
  locale,
}: {
  due: TaskDeadlineView;
  submitted: boolean;
  locale: string;
}) {
  const text = deadlineText(due, locale);
  if (submitted)
    return (
      <time className="dueChip done" title={text.label}>
        {t("Submitted")}
      </time>
    );
  return due.overdue ? (
    <time className="dueChip overdue">{text.label}</time>
  ) : (
    <time className={`dueChip ${due.countdownTone}`} title={text.label}>
      {text.countdownLabel}
    </time>
  );
}
