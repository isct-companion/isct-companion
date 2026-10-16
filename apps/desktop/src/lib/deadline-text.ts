import type { TaskDeadlineView } from "../generated/persistence";
import { formatJst } from "./civil-date";
import { t } from "../i18n";

/** Core supplies time and urgency; this boundary only formats the view's text. */
export function deadlineText(deadline: TaskDeadlineView, locale: string) {
  const ms = deadline.remainingMilliseconds;
  const unit = (count: number, suffix: "d" | "h" | "m") =>
    t(`{{count}}${suffix}`, { count });
  const relative = (value: number) => {
    const hours = Math.max(1, Math.floor(value / 3_600_000));
    return hours < 24
      ? unit(hours, "h")
      : unit(Math.floor((hours + 12) / 24), "d");
  };
  const hours = Math.floor(Math.max(0, ms) / 3_600_000);
  const days = Math.floor(hours / 24);
  const countdownLabel = deadline.overdue
    ? t("{{duration}} ago", { duration: relative(Math.abs(ms)) })
    : hours < 1
      ? unit(Math.max(1, Math.floor(ms / 60_000)), "m")
      : hours < 24
        ? unit(hours, "h")
        : hours < 72 && hours % 24
          ? `${unit(days, "d")} ${unit(hours % 24, "h")}`
          : unit(days, "d");
  const label = deadline.overdue
    ? countdownLabel
    : deadline.tone === "warning"
      ? t("Due in {{duration}}", { duration: relative(ms) })
      : t("Due {{date}}", {
          date: formatJst(deadline.at, locale, {
            month: "short",
            day: "numeric",
            hour: "2-digit",
            minute: "2-digit",
          }),
        });
  return { ...deadline, label, countdownLabel };
}
