import type { Subscription, Workspace } from "../../core/model";
import { subscriptionYear } from "../../platform/subscription-polling";

export const subscriptionRow = (data: Workspace, kind: Subscription["kind"]) =>
  data.subscriptions.find((row) => row.kind === kind && !row.deletedAt);

export const languageTag = (locale: Workspace["settings"]["locale"]) =>
  locale === "ja" ? "ja-JP" : "en-GB";

/** "3 Oct" / "10月3日" for an ISO timestamp or civil date, in Japan time. */
export function formatDay(
  value: string,
  locale: Workspace["settings"]["locale"],
  options: Intl.DateTimeFormatOptions = {},
) {
  const date = /^\d{4}-\d{2}-\d{2}$/.test(value)
    ? new Date(`${value}T12:00:00+09:00`)
    : new Date(value);
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

/** What one official dataset looks like for the year it currently follows. */
export function subscriptionState(
  data: Workspace,
  kind: Subscription["kind"],
): SubscriptionState {
  const row = subscriptionRow(data, kind);
  const year = subscriptionYear(data, kind);
  if (row?.enabled === false) return { state: "off" };
  if (!year) return { state: "no-year" };
  if (row?.lastError) return { state: "error", message: row.lastError };
  if (row?.fetchedAt && row.year === year)
    return { state: "current", fetchedAt: row.fetchedAt };
  return { state: "pending" };
}
