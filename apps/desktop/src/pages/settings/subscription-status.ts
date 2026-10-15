import { intlLocale } from "../../i18n/locale";
import type { Subscription, Workspace } from "../../lib/model";
import { useScheduleQuery } from "../../platform/core-query";
import { isCivilDate, parseIsoTimestamp } from "../../lib/civil-date";

export const subscriptionRow = (data: Workspace, kind: Subscription["kind"]) =>
  data.subscriptions.find((row) => row.kind === kind && !row.deletedAt);

export const languageTag = intlLocale;

/** "3 Oct" / "10月3日" for an ISO timestamp or civil date, in Japan time. */
export function formatDay(
  value: string,
  locale: Workspace["settings"]["locale"],
  options: Intl.DateTimeFormatOptions = {},
) {
  const date = isCivilDate(value)
    ? new Date(`${value}T12:00:00+09:00`)
    : parseIsoTimestamp(value);
  if (!date) return value;
  return new Intl.DateTimeFormat(languageTag(locale), {
    month: "short",
    day: "numeric",
    timeZone: "Asia/Tokyo",
    ...options,
  }).format(date);
}

export type SubscriptionState =
  | { state: "off" }
  | { state: "no-year" }
  | { state: "error"; message: string }
  | { state: "current"; fetchedAt: string }
  | { state: "pending" };

export type SubscriptionView = {
  year?: number | null;
  enabled: boolean;
  state: SubscriptionState;
  url: string;
  dataUrl?: string | null;
};
export function useSubscriptionViews() {
  return useScheduleQuery<Record<Subscription["kind"], SubscriptionView>>({
    kind: "subscriptionViews",
  });
}
