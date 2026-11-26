import { useEffect, useState } from "react";
import { useIntersectionObserver } from "usehooks-ts";
import type { TaskDeadlineView } from "../generated/persistence";
import { countdownText } from "../lib/deadline-text";
import { formatJst } from "../lib/civil-date";
import { t } from "../i18n";
import { scheduleQuery } from "../platform/core-query";
import { wallNow } from "../platform/use-clock";
import { Tag, type TagRole } from "./ui";

/** Core supplies each display and its next refresh instant. */
export function DueChip({
  due,
  submitted = false,
  locale,
}: {
  due: TaskDeadlineView;
  submitted?: boolean;
  locale: string;
}) {
  const { ref, isIntersecting } = useIntersectionObserver();
  const [refresh, setRefresh] = useState<{
    initial: TaskDeadlineView;
    view: TaskDeadlineView;
  }>();
  const view = refresh?.initial === due ? refresh.view : due;
  useEffect(() => {
    if (submitted || !isIntersecting || !view.nextChangeAt) return;
    let cancelled = false;
    const timer = setTimeout(
      () => {
        void scheduleQuery<TaskDeadlineView>({
          kind: "deadlineDisplay",
          at: view.at,
          now: new Date(wallNow()).toISOString(),
        }).then(
          (next) => {
            if (!cancelled) setRefresh({ initial: due, view: next });
          },
          () => undefined,
        );
      },
      Math.max(1, Date.parse(view.nextChangeAt) - wallNow()),
    );
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [due, isIntersecting, submitted, view]);

  const tone = submitted
    ? "done"
    : view.state === "overdue"
      ? "overdue"
      : view.countdownTone;
  return (
    <Tag
      ref={ref}
      component="time"
      tone={dueRole(tone)}
      className={`dueChip ${tone}`}
      title={t("course.dueDate", {
        date: formatJst(view.at, locale, {
          month: "short",
          day: "numeric",
          hour: "2-digit",
          minute: "2-digit",
        }),
      })}
    >
      {submitted ? t("common.submitted") : countdownText(view, locale)}
    </Tag>
  );
}

const dueRole = (tone: string): TagRole => {
  if (tone === "done") return "ok";
  if (tone === "overdue" || tone === "urgent") return "danger";
  return tone === "soon" ? "warn" : "neutral";
};
