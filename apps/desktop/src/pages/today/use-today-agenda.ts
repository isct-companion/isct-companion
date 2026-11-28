import { useEffect } from "react";
import type { TodayAgenda } from "../../generated/persistence";
import { scheduleAnswers } from "../../platform/core-query";
import { wallNow } from "../../platform/use-clock";
import { useViewQuery } from "../../platform/view-query";

export function useTodayAgenda(date: string | null, active: boolean) {
  const state = useViewQuery(
    ["schedule", "todayAgenda", date],
    async () => {
      const answer = await scheduleAnswers<[TodayAgenda]>([
        { kind: "todayAgenda", date, now: new Date(wallNow()).toISOString() },
      ]);
      return { ...answer, value: answer.value[0] };
    },
    active,
  );
  const { error, pending, retry, value } = state;
  const nextBoundary = value?.nextBoundary;
  useEffect(() => {
    if (!active || pending || error || !retry || !nextBoundary) return;
    const timer = setTimeout(
      retry,
      Math.max(0, Date.parse(nextBoundary) - wallNow()),
    );
    return () => clearTimeout(timer);
  }, [active, error, nextBoundary, pending, retry]);
  return state;
}
