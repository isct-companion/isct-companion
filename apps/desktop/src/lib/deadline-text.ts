import type { CountdownUnit, TaskDeadlineView } from "../generated/persistence";
import { t, type MessageKey } from "../i18n";

const UNIT_KEYS: Record<CountdownUnit, MessageKey> = {
  days: "format.durationDays",
  hours: "format.durationHours",
  minutes: "format.countdownMinutes",
  seconds: "format.countdownSeconds",
};

export function countdownText(deadline: TaskDeadlineView, locale: string) {
  const number = new Intl.NumberFormat(locale);
  const duration = deadline.units
    .map(({ unit, count }) =>
      t(UNIT_KEYS[unit], { count: number.format(count) }),
    )
    .join(" ");
  return deadline.state === "overdue"
    ? t("deadlineText.durationAgo", { duration })
    : duration;
}
