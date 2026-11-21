import type { Subscription, Workspace } from "../../lib/model";
import { useScheduleQuery } from "../../platform/core-query";

export const subscriptionRow = (data: Workspace, kind: Subscription["kind"]) =>
  data.subscriptions.find((row) => row.kind === kind && !row.deletedAt);

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
