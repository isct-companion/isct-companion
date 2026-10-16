import { getDesktopBridge } from "./desktop-bridge";
import { useViewQuery } from "./view-query";
import type {
  AcademicProfile,
  CalendarException,
  Course,
  ImportantDate,
  TermId,
} from "../lib/model";

import type * as Protocol from "../generated/persistence";
import type { WorkspaceQuery } from "../generated/persistence";
// Fields with serde defaults may be omitted by shells; all other request fields
// are checked against the generated Core protocol.
type Defaulted =
  | "includePersonal"
  | "options"
  | "beforeDays"
  | "afterDays"
  | "limit"
  | "scope"
  | "locale";
type OptionalDefaults<T> = T extends { kind: string }
  ? Omit<T, Defaulted> & Partial<Pick<T, Extract<keyof T, Defaulted>>>
  : never;
export type ScheduleQuery = OptionalDefaults<WorkspaceQuery>;
export type QueryState<T> = {
  value?: T;
  error?: string;
  pending: boolean;
  retry?: () => void;
};

/** These fields use serde's skip_serializing_if: Option::is_none. */
type OmittedNulls<T> = {
  [K in keyof T as null extends T[K] ? never : K]: T[K];
} & { [K in keyof T as null extends T[K] ? K : never]?: Exclude<T[K], null> };
export type ScheduleOccurrence = OmittedNulls<Protocol.Occurrence>;
export type QuarterRange = OmittedNulls<Protocol.QuarterRange>;
export type DatedAgenda = {
  date: string;
  occurrences: ScheduleOccurrence[];
  calendarState?: {
    label?: string | Record<string, string>;
    type?: string;
    replacementDay?: number;
  } | null;
  importantDates: ImportantDate[];
  noClasses: boolean;
  calendarNotice?: string;
};
type ScheduleOverviewFields = {
  term: TermId;
  courses: Course[];
  allTerms: TermId[];
  range: QuarterRange | null;
};
export type WorkspaceOverview = Omit<
  Protocol.WorkspaceOverview,
  | keyof ScheduleOverviewFields
  | "currentTerm"
  | "selectedTerm"
  | "todayTerm"
  | "coursesTerm"
> &
  ScheduleOverviewFields & {
    currentTerm: TermId;
    selectedTerm: TermId;
    todayTerm: TermId;
    coursesTerm: TermId;
  };
export type ScheduleView = Omit<
  Protocol.ScheduleView,
  keyof ScheduleOverviewFields | "agendas"
> &
  ScheduleOverviewFields & { agendas: DatedAgenda[] };
export type PlanningWeek = Omit<Protocol.PlanningWeek, "courses" | "days"> & {
  courses: Course[];
  days: { day: number; blocks: ScheduleOccurrence[] }[];
};
export type AcademicYearIndex = Protocol.AcademicYearIndex;
export type PlanCivilStart = Protocol.PlanCivilStart;
export type CourseTimeline = Protocol.CourseTimeline;
export type TaskView = Protocol.TaskView;
export type CourseDetail = Protocol.CourseDetail;
export type CalendarOrigin = "official" | "own" | "changed" | "estimated";
export type CalendarDayView = {
  date: string;
  type: "holiday" | "no-class" | "replacement" | "break";
  replacementDay?: number;
  label: string;
  origin: Exclude<CalendarOrigin, "estimated">;
  official?: {
    type: "holiday" | "no-class" | "replacement" | "break";
    replacementDay?: number;
    label: string;
  };
  exception?: CalendarException;
};
export type AcademicYearCalendar = {
  year: number;
  quarters: {
    quarter: "1Q" | "2Q" | "3Q" | "4Q";
    from: string;
    to: string;
    origin: Exclude<CalendarOrigin, "changed">;
  }[];
  days: CalendarDayView[];
  importantDates: ImportantDate[];
  published: boolean;
};
export type { AcademicProfile };

export async function scheduleQueries<T extends unknown[]>(
  queries: readonly ScheduleQuery[],
): Promise<T> {
  const bridge = getDesktopBridge();
  if (!bridge) throw new Error("Schedule data requires the native Core.");
  const response = await bridge.scheduleQuery(queries);
  if (
    !Array.isArray(response.values) ||
    response.values.length !== queries.length ||
    response.values.some(
      (result, index) =>
        result.kind !==
        ((
          {
            validateAcademicProfile: "academicProfile",
            academicTermForDate: "academicTerm",
          } as Record<string, string>
        )[queries[index].kind] ?? queries[index].kind),
    )
  )
    throw new Error("Core returned an invalid schedule snapshot.");
  return response.values.map((result) => result.value) as T;
}

export async function scheduleQuery<T>(query: ScheduleQuery): Promise<T> {
  return (await scheduleQueries<[T]>([query]))[0];
}

/** Cache query results, never domain decisions. A new revision or query owns
 * a new request; late responses cannot replace the currently rendered view. */
export function useScheduleQueries<T extends unknown[]>(
  queries: readonly ScheduleQuery[],
  enabled = true,
): QueryState<T> {
  return useViewQuery(
    "schedule",
    queries,
    () => scheduleQueries<T>(queries),
    enabled,
  );
}

export function useScheduleQuery<T>(
  query: ScheduleQuery,
  enabled = true,
): QueryState<T> {
  const state = useScheduleQueries<[T]>([query], enabled);
  return { ...state, value: state.value?.[0] };
}
