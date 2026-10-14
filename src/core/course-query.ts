/** Presentation contracts for the native Core finder. Query policy lives in isct-core/course_query. */
import type { Localized, Quarter } from "./model";

export type FinderCourse = {
  id: string;
  code: string;
  year: number;
  quarters: Quarter[];
  title: Localized;
  instructors: Localized[];
  credits: { lecture: number; exercise: number; lab: number };
  instructionLanguage: string;
  format: { kinds: string[] };
  schedule: {
    kind: string;
    day?: number;
    periods?: [number, number];
    room?: Localized;
  }[];
  unitCode?: string;
  syllabusUrl: string;
  /** False while only the search row is stored: no schedule, credits or format yet. */
  detailed?: boolean;
};

export type FinderContext = {
  currentQuarter: number;
  /** Recommendation marks of the person's department, by course code. */
  recommended: Map<
    string,
    { group: string; mark?: "required" | "recommended" }
  >;
  departments: { id: string; name: Localized; unitCode: string }[];
  /** Weekly slots already taken in the quarter being planned. */
  busy: { day: number; from: number; to: number }[];
};

type TimeValue = { day?: number; from?: number; to?: number };
export type ConditionType =
  | "quarter"
  | "time"
  | "code"
  | "level"
  | "mark"
  | "group"
  | "dept"
  | "lang"
  | "format"
  | "kind"
  | "credits"
  | "free"
  | "text";
export type Condition = {
  type: ConditionType;
  key: string;
  label: string;
  values: (string | number | TimeValue)[];
  prefix?: boolean;
  exclude?: boolean;
};
export type Factor = { type: ConditionType | "exclude"; alts: Condition[] };

export const DAYS = ["月", "火", "水", "木", "金", "土"] as const;

export const LOCAL_TYPES = new Set<ConditionType>(["mark", "group", "free"]);
export type RemoteQuery = {
  q: string;
  scope?: string;
  code?: string;
  groups?: string[];
};
export type RemotePlan =
  { queries: RemoteQuery[] } | { unsupported: "empty" | "too-many" };

export type FinderRow = { course: FinderCourse; matched: string[] };
export type Suggestion =
  | { kind: "condition"; key: string; condition: Condition; count: number }
  | { kind: "course"; course: FinderCourse }
  | { kind: "text"; key: string; count: number }; // count -1: decided by the syllabus
