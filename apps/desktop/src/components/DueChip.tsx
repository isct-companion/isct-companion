import type { TaskDeadlineView } from "../generated/persistence";
import { deadlineText } from "../lib/deadline-text";
import { t } from "../i18n";
import { Tag, type TagRole } from "./ui";

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
      <Tag component="time" tone="ok" className="dueChip done" title={text.label}>
        {t("Submitted")}
      </Tag>
    );
  return due.overdue ? (
    <Tag component="time" tone="danger" className="dueChip overdue">
      {text.label}
    </Tag>
  ) : (
    <Tag
      component="time"
      tone={dueRole(due.countdownTone)}
      className={`dueChip ${due.countdownTone}`}
      title={text.label}
    >
      {text.countdownLabel}
    </Tag>
  );
}

/** A countdown's tone as a tag role: today's deadline is danger, a close
 * one warn, anything further off neutral. */
export const dueRole = (tone: string): TagRole =>
  tone === "urgent" ? "danger" : tone === "soon" ? "warn" : "neutral";
