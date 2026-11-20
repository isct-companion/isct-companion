import { createElement, type ComponentType } from "react";
import {
  EMPTY_WORKSPACE,
  type ReminderLeadOptions,
} from "../generated/persistence";
import type { Workspace } from "../lib/model";
import type { SchoolRecords } from "./desktop-bridge";
import {
  scheduleAnswers,
  useScheduleQuery,
  type ScheduleQuery,
  type WorkspaceOverview,
} from "./core-query";
import { freshAnswer, queryClient } from "./query-client";

/** What a page can ask Core for by name: workspace records, and the school
 * records only the shell shows. */
export type ShellRecords = Workspace &
  Pick<SchoolRecords, "lmsCourses" | "freshness" | "pageAvailability"> & {
    readonly reminderLeadOptions: ReminderLeadOptions;
  };
export type RecordField = keyof ShellRecords;

const recordsQuery = (fields: readonly RecordField[]): ScheduleQuery => ({
  kind: "records",
  fields: [...new Set(fields)].sort(),
});

/** Fields a page left out read as empty, never as missing. */
const complete = (answer: Record<string, unknown>) =>
  ({
    ...EMPTY_WORKSPACE,
    lmsCourses: [],
    freshness: {},
    pageAvailability: {},
    ...answer,
  }) as ShellRecords;

/** Core's records for `fields` now, from the query cache when it holds a
 * current answer; for an edit that starts from what is stored. */
export async function fetchRecords(
  fields: readonly RecordField[],
): Promise<ShellRecords> {
  const query = recordsQuery(fields);
  const answer = await queryClient.fetchQuery({
    queryKey: ["schedule", [query]],
    queryFn: ({ signal }) =>
      freshAnswer(
        () => scheduleAnswers<[Record<string, unknown>]>([query]),
        signal,
      ),
  });
  return complete(answer.value[0]);
}

/**
 * The records a page shows, only `fields` (settings, the device, terms and
 * the current term always come along), cached until Core says what they
 * read changed. Courses come as the shell shows them: the colour Core
 * picked, and the title and instructor in the content language unless the
 * person renamed them.
 */
export function useRecords(
  fields: readonly RecordField[],
  enabled = true,
): ShellRecords | undefined {
  const answer = useScheduleQuery<Record<string, unknown>>(
    recordsQuery(fields),
    enabled,
  ).value;
  const overview = useScheduleQuery<WorkspaceOverview>(
    { kind: "workspaceOverview", term: "" },
    enabled && fields.includes("courses"),
  ).value;
  if (!answer) return undefined;
  const records = complete(answer);
  if (!fields.includes("courses")) return records;
  // Core reads a course's title and instructor in the content language.
  return {
    ...records,
    courses: records.courses.map((course) => ({
      ...course,
      color: overview?.colors[course.id] ?? course.color,
      title: course.overrides?.title ?? course.title,
      instructor: course.overrides?.instructor ?? course.instructor,
    })),
  };
}

/** `Body` fed with its own records instead of the shell's: it renders once
 * Core answered for `fields`. */
export function withRecords<P extends { data: Workspace }>(
  fields: readonly RecordField[],
  Body: ComponentType<P>,
): ComponentType<Omit<P, "data">> {
  function WithRecords(props: Omit<P, "data">) {
    const data = useRecords(fields);
    return data
      ? createElement(Body, { ...props, data } as unknown as P)
      : null;
  }
  WithRecords.displayName = `WithRecords(${Body.displayName ?? Body.name})`;
  return WithRecords;
}
