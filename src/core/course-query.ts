/**
 * Course finder query: browser-address-bar style search over the local catalog.
 *
 * A query is a list of condition keys (canonical spellings such as "curr", "火3-4",
 * "推荐", "!集中"). Conditions of the same kind are alternatives (or); different kinds
 * must all hold (and); a leading "!" excludes. Free words search title, instructor and
 * code. There is no other syntax, so any input is valid.
 */
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
  schedule: { kind: string; day?: number; periods?: [number, number]; room?: Localized }[];
  unitCode?: string;
  syllabusUrl: string;
  /** False while only the search row is stored: no schedule, credits or format yet. */
  detailed?: boolean;
};

export type FinderContext = {
  currentQuarter: number;
  /** Recommendation marks of the person's department, by course code. */
  recommended: Map<string, { group: string; mark?: "required" | "recommended" }>;
  departments: { id: string; name: Localized; unitCode: string }[];
  /** Weekly slots already taken in the quarter being planned. */
  busy: { day: number; from: number; to: number }[];
};

type TimeValue = { day?: number; from?: number; to?: number };
export type ConditionType =
  | "quarter" | "time" | "code" | "level" | "mark" | "group" | "dept"
  | "lang" | "format" | "kind" | "credits" | "free" | "text";
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
const DAY_EN: Record<string, number> = { mon: 1, tue: 2, wed: 3, thu: 4, fri: 5, monday: 1, tuesday: 2, wednesday: 3, thursday: 4, friday: 5, sat: 6, saturday: 6 };
const DAY_JA: Record<string, number> = { 月: 1, 火: 2, 水: 3, 木: 4, 金: 5, 土: 6 };
export const SLOTS: [number, number][] = [[1, 2], [3, 4], [5, 6], [7, 8], [9, 10]];
const ORDER: (ConditionType | "exclude")[] = ["quarter", "dept", "level", "code", "time", "format", "lang", "kind", "credits", "mark", "group", "free", "text", "exclude"];
/** Conditions the syllabus cannot answer; they are evaluated by the app. */
export const LOCAL_TYPES = new Set<ConditionType>(["mark", "group", "free"]);

export const nfkc = (value: string) => value.normalize("NFKC");
const lower = (value: string) => nfkc(value).toLowerCase();

function slotLabel(day: number | undefined, from?: number, to?: number) {
  const d = day ? DAYS[day - 1] : "";
  if (from == null) return `${d}曜`;
  const p = `${from}${to != null && to !== from ? `-${to}` : ""}`;
  return d ? `${d}${p}` : `${p}限`;
}

/** One word (or "tue 3" / "group A") → one condition. Unknown words become text. */
export function recognize(word: string, ctx: FinderContext, next?: string): Condition {
  const w = nfkc(word).trim();
  const lw = w.toLowerCase();
  const text: Condition = { type: "text", key: w, label: w, values: [lw] };
  let m: RegExpMatchArray | null;
  if (/^(curr|current|当前|今期)$/i.test(w))
    return { type: "quarter", key: "curr", label: `${ctx.currentQuarter}Q`, values: [ctx.currentQuarter] };
  if ((m = lw.match(/^q?([1-4])q?$/)) && /q/.test(lw)) return { type: "quarter", key: `${m[1]}Q`, label: `${m[1]}Q`, values: [+m[1]] };
  if ((m = w.match(/^([A-Za-z]{3})\.([A-Za-z])(\d{3})(?:\.[A-Za-z])?$/))) {
    const code = `${m[1]}.${m[2]}${m[3]}`.toUpperCase();
    return { type: "code", key: code, label: code, values: [code] };
  }
  // "CSC", "CSC.T3", "csc.t3" — but not a lowercase word like "rec"
  if ((m = w.match(/^([A-Za-z]{3})(\.([A-Za-z]\d{0,2})?)?$/)) && (w.includes(".") || /^[A-Z]{3}$/.test(w))) {
    const prefix = w.toUpperCase().replace(/\.$/, "");
    return { type: "code", key: prefix, label: `${prefix}…`, values: [prefix.includes(".") ? prefix : `${prefix}.`], prefix: true };
  }
  const time = (day: number | undefined, from?: number, to?: number): Condition => {
    if (from != null && to != null && to < from) [from, to] = [to, from];
    const label = slotLabel(day, from, to ?? from);
    return { type: "time", key: label, label, values: [{ day, from, to: to ?? from }] };
  };
  if ((m = w.match(/^([月火水木金土])(?:曜日?)?(\d{1,2})(?:-(\d{1,2}))?限?$/))) return time(DAY_JA[m[1]], +m[2], m[3] ? +m[3] : undefined);
  if ((m = lw.match(/^(mon|tue|wed|thu|fri|sat)(\d{1,2})(?:-(\d{1,2}))?$/))) return time(DAY_EN[m[1]], +m[2], m[3] ? +m[3] : undefined);
  if (DAY_EN[lw] && next && /^\d{1,2}(-\d{1,2})?$/.test(next)) {
    const [a, b] = next.split("-").map(Number);
    return time(DAY_EN[lw], a, b);
  }
  if ((m = w.match(/^([月火水木金土])曜日?$/))) return time(DAY_JA[m[1]]);
  if (DAY_EN[lw]) return time(DAY_EN[lw]);
  if ((m = w.match(/^(\d{1,2})(?:-(\d{1,2}))?限$/))) return time(undefined, +m[1], m[2] ? +m[2] : undefined);
  if ((m = w.match(/^([1-6])00(?:番台|-level)$/)) || (m = lw.match(/^([1-6])xx$/)))
    return { type: "level", key: `${m[1]}00番台`, label: `${m[1]}00番台`, values: [+m[1] * 100] };
  if (/^(推荐|推薦|おすすめ|rec|recommended|○)$/i.test(w)) return { type: "mark", key: "推荐", label: "Recommended", values: ["any"] };
  if (/^(必修|required|◎)$/i.test(w)) return { type: "mark", key: "必修", label: "Required", values: ["required"] };
  if ((m = w.match(/^([A-Z])群$/)) || (lw === "group" && next && (m = next.match(/^([A-Za-z])$/))))
    return { type: "group", key: `${m[1].toUpperCase()}群`, label: `${m[1].toUpperCase()}群`, values: [m[1].toUpperCase()] };
  if (/^(空き|free|不冲突|no-conflict)$/i.test(w)) return { type: "free", key: "空き", label: "No conflict", values: [true as unknown as string] };
  if (/^(講義|lecture)$/i.test(w)) return { type: "format", key: "講義", label: "講義", values: ["lecture"] };
  if (/^(演習|exercise)$/i.test(w)) return { type: "format", key: "演習", label: "演習", values: ["exercise"] };
  if (/^(実験|実験等|experiment)$/i.test(w)) return { type: "format", key: "実験", label: "実験", values: ["experiment"] };
  if (/^(英語|english)$/i.test(w)) return { type: "lang", key: "英語", label: "English-taught", values: ["en"] };
  if (/^(日本語|japanese)$/i.test(w)) return { type: "lang", key: "日本語", label: "Japanese-taught", values: ["ja"] };
  if (/^(集中|集中講義|intensive)$/i.test(w)) return { type: "kind", key: "集中", label: "Intensive", values: ["intensive"] };
  if (/^(特課研|特課PJ|seminar)$/i.test(w)) return { type: "kind", key: "特課研", label: "Seminar", values: ["seminar"] };
  if ((m = lw.match(/^(\d(?:\.\d)?)(単位|cr|credits?)$/))) return { type: "credits", key: `${m[1]}単位`, label: `${m[1]} cr`, values: [+m[1]] };
  const department = ctx.departments.find((d) => [d.id, d.name.ja, d.name.en].some((n) => n && lower(n) === lw));
  if (department) return { type: "dept", key: department.name.ja ?? department.id, label: department.name.ja ?? department.name.en ?? department.id, values: [department.unitCode] };
  return text;
}

/** A key may start with "!" to exclude. */
export function condition(key: string, ctx: FinderContext): Condition {
  const exclude = key.startsWith("!") && key.length > 1;
  const raw = exclude ? key.slice(1) : key;
  const [first, second] = raw.split(/\s+/);
  const base = second && (DAY_EN[lower(first)] || lower(first) === "group") ? recognize(first, ctx, second) : recognize(raw, ctx);
  return exclude ? { ...base, key: `!${base.key}`, exclude: true } : base;
}

/** Keys → factors: same type = alternatives, types = all required; duplicates dropped. */
export function factorsOf(keys: string[], ctx: FinderContext) {
  const factors: Factor[] = [];
  let duplicates = 0;
  for (const key of keys) {
    const c = condition(key, ctx);
    if (factors.some((f) => f.alts.some((a) => a.key === c.key))) { duplicates++; continue; }
    const type = c.exclude ? "exclude" : c.type;
    // text words and exclusions must all hold, so each stands alone
    let factor = type === "text" || type === "exclude" ? undefined : factors.find((f) => f.type === type);
    if (!factor) { factor = { type, alts: [] }; factors.push(factor); }
    factor.alts.push(c);
  }
  factors.sort((a, b) => ORDER.indexOf(a.type) - ORDER.indexOf(b.type));
  return { factors, duplicates };
}
export const normalizedKeys = (keys: string[], ctx: FinderContext) =>
  factorsOf(keys, ctx).factors.flatMap((f) => f.alts.map((a) => a.key));

const weekly = (c: FinderCourse) => c.schedule.filter((s) => s.kind === "weekly" && s.day && s.periods);
const overlap = (a: [number, number], from: number, to: number) => a[0] <= to && from <= a[1];
export const totalCredits = (c: FinderCourse) => c.credits.lecture + c.credits.exercise + c.credits.lab;
export function clashes(c: FinderCourse, busy: FinderContext["busy"]) {
  return weekly(c).some((s) => busy.some((b) => b.day === s.day && overlap(s.periods!, b.from, b.to)));
}
/** Placed courses whose weekly periods overlap this result's (the ones Add replaces). */
export function clashingCourses<T extends { code: string; meetings: { days: number[]; periods: number[] }[] }>(
  c: FinderCourse,
  placed: readonly T[],
): T[] {
  const slots = weekly(c);
  return placed.filter(
    (p) =>
      p.code !== c.code &&
      p.meetings.some(
        (m) =>
          m.periods.length > 0 &&
          slots.some((s) => m.days.includes(s.day!) && overlap(s.periods!, Math.min(...m.periods), Math.max(...m.periods))),
      ),
  );
}

function hit(c: FinderCourse, cond: Condition, ctx: FinderContext): boolean {
  return cond.values.some((v) => {
    switch (cond.type) {
      case "quarter": return c.quarters.includes(`${v}Q` as Quarter);
      case "time": {
        const t = v as TimeValue;
        return weekly(c).some((s) => (t.day == null || s.day === t.day) && (t.from == null || overlap(s.periods!, t.from, t.to ?? t.from)));
      }
      case "code": return cond.prefix ? c.code.startsWith(String(v)) : c.code === v;
      case "level": return (Number(c.code[5]) || 0) * 100 === v;
      case "mark": { const r = ctx.recommended.get(c.code); return v === "required" ? r?.mark === "required" : Boolean(r?.mark); }
      case "group": return ctx.recommended.get(c.code)?.group === v;
      case "dept": return c.unitCode === v;
      case "lang": return c.instructionLanguage === v || c.instructionLanguage === "ja-en";
      case "format": return c.format.kinds.includes(String(v));
      case "kind": return c.schedule.some((s) => s.kind === v);
      case "credits": return totalCredits(c) === v;
      case "free": return !clashes(c, ctx.busy);
      case "text": return searchable(c).includes(String(v));
    }
    return false;
  });
}
const searchCache = new WeakMap<FinderCourse, string>();
function searchable(c: FinderCourse) {
  let value = searchCache.get(c);
  if (!value) {
    value = lower([c.code, c.title.ja, c.title.en, ...c.instructors.flatMap((i) => [i.ja, i.en])].filter(Boolean).join(" "));
    searchCache.set(c, value);
  }
  return value;
}

/* ---------- what the syllabus itself can answer ---------- */
/** Conditions sent to the official syllabus search; the rest filter its rows locally. */
export const REMOTE_TYPES = new Set<ConditionType>(["dept", "code", "level", "text"]);
export const MAX_REMOTE_QUERIES = 4;
export type RemoteQuery = { q: string; scope?: string; code?: string; groups?: string[] };
export type RemotePlan =
  | { queries: RemoteQuery[] }
  | { unsupported: "empty" | "too-many" };

const isRemote = (f: Factor) => f.type !== "exclude" && REMOTE_TYPES.has(f.type);
/** Factors the app evaluates itself (the syllabus already applied the others). */
export const localFactors = (factors: Factor[]) => factors.filter((f) => !isRemote(f));

/**
 * Pills → syllabus searches. The site takes one department and one code per
 * request, so alternatives fan out (at most four requests). It refuses an
 * empty keyword; "." matches every course code. Without a department, code
 * or word there is nothing it can search for.
 */
export function remotePlan(factors: Factor[], scopeOf: (unitCode: string) => string | undefined): RemotePlan {
  const alts = (type: ConditionType) => factors.filter((f) => f.type === type).flatMap((f) => f.alts);
  const words = alts("text").map((c) => c.key);
  const scopes = alts("dept").map((c) => scopeOf(String(c.values[0]))).filter((v): v is string => Boolean(v));
  const codes = alts("code").map((c) => c.key);
  const groups = alts("level").map((c) => String(c.values[0]));
  if (!words.length && !scopes.length && !codes.length) return { unsupported: "empty" };
  const scopeList: (string | undefined)[] = scopes.length ? scopes : [undefined];
  const codeList: (string | undefined)[] = codes.length ? codes : [undefined];
  if (scopeList.length * codeList.length > MAX_REMOTE_QUERIES) return { unsupported: "too-many" };
  const q = words.join(" ") || ".";
  return {
    queries: scopeList.flatMap((scope) =>
      codeList.map((code) => ({ q, ...(scope ? { scope } : {}), ...(code ? { code } : {}), ...(groups.length ? { groups } : {}) })),
    ),
  };
}

export type FinderRow = { course: FinderCourse; matched: string[] };
/** Courses that pass every factor; `matched` names which alternatives hit when a factor has several. */
export function run(courses: FinderCourse[], factors: Factor[], ctx: FinderContext, liveText = ""): FinderRow[] {
  const multi = factors.filter((f) => f.alts.length > 1 && f.type !== "exclude");
  const live = lower(liveText.trim());
  const seen = new Set<string>();
  const rows: FinderRow[] = [];
  for (const c of courses) {
    if (seen.has(c.id)) continue;
    if (live && !searchable(c).includes(live)) continue;
    const ok = factors.every((f) => (f.type === "exclude" ? !hit(c, f.alts[0], ctx) : f.alts.some((a) => hit(c, a, ctx))));
    if (!ok) continue;
    seen.add(c.id);
    rows.push({ course: c, matched: multi.flatMap((f) => f.alts.filter((a) => hit(c, a, ctx)).map((a) => a.label)) });
  }
  const weight = (c: FinderCourse) => { const r = ctx.recommended.get(c.code); return r?.mark === "required" ? 2 : r?.mark ? 1 : 0; };
  return rows.sort((a, b) => weight(b.course) - weight(a.course) || a.course.code.localeCompare(b.course.code));
}

/* ---------- address-bar suggestions ---------- */
type VocabEntry = { key: string; aliases: string[] };
export function vocabulary(ctx: FinderContext): VocabEntry[] {
  const en = ["mon", "tue", "wed", "thu", "fri", "sat"];
  const long = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday"];
  const groups = [...new Set([...ctx.recommended.values()].map((r) => r.group))].sort();
  return [
    { key: "curr", aliases: ["curr", "current", "当前", "今期"] },
    ...[1, 2, 3, 4].map((q) => ({ key: `${q}Q`, aliases: [`${q}q`, `q${q}`] })),
    ...DAYS.flatMap((d, i) => [
      { key: `${d}曜`, aliases: [d, `${d}曜`, en[i], long[i]] },
      ...SLOTS.map(([a, b]) => ({ key: `${d}${a}-${b}`, aliases: [`${d}${a}-${b}`, `${d}${a}`, `${d}${b}`, `${en[i]}${a}`, `${en[i]} ${a}`, `${en[i]}${a}-${b}`] })),
    ]),
    // Recommendations come from optional subscription data; offer them only when present.
    ...(ctx.recommended.size
      ? [
          { key: "推荐", aliases: ["推荐", "推薦", "おすすめ", "rec", "recommended"] },
          { key: "必修", aliases: ["必修", "required"] },
        ]
      : []),
    ...groups.map((g) => ({ key: `${g}群`, aliases: [`${g}群`, `group ${g.toLowerCase()}`] })),
    { key: "空き", aliases: ["空き", "free", "no conflict", "不冲突"] },
    ...ctx.departments.map((d) => ({ key: d.name.ja ?? d.id, aliases: [d.id, d.name.ja, d.name.en].filter(Boolean) as string[] })),
    { key: "200番台", aliases: ["200番台", "200", "2xx"] },
    { key: "300番台", aliases: ["300番台", "300", "3xx"] },
    { key: "講義", aliases: ["講義", "lecture"] },
    { key: "演習", aliases: ["演習", "exercise"] },
    { key: "実験", aliases: ["実験", "experiment"] },
    { key: "英語", aliases: ["英語", "english"] },
    { key: "日本語", aliases: ["日本語", "japanese"] },
    { key: "集中", aliases: ["集中", "集中講義", "intensive"] },
    { key: "特課研", aliases: ["特課研", "seminar"] },
  ];
}
export const COMMON_KEYS = ["curr", "推荐", "空き", "講義", "演習", "英語"];

export type Suggestion =
  | { kind: "condition"; key: string; condition: Condition; count: number }
  | { kind: "course"; course: FinderCourse }
  | { kind: "text"; key: string; count: number }; // count -1: decided by the syllabus

/** Suggestions for what is being typed, like a browser address bar. */
export function suggest(input: string, keys: string[], courses: FinderCourse[], ctx: FinderContext): Suggestion[] {
  const exclude = input.startsWith("!");
  const typed = nfkc(exclude ? input.slice(1) : input).trim();
  const lt = typed.toLowerCase();
  const out: Suggestion[] = [];
  const applied = new Set(keys);
  const count = (key: string) => run(courses, factorsOf([...keys, key], ctx).factors, ctx).length;
  const addCondition = (key: string) => {
    const full = exclude ? `!${key}` : key;
    if (applied.has(full) || out.some((s) => s.kind === "condition" && s.key === full)) return;
    const found = condition(full, ctx);
    if (!ctx.recommended.size && (found.type === "mark" || found.type === "group")) return;
    // The syllabus answers department, code and word conditions; their count is unknown until it does.
    const remote = !found.exclude && REMOTE_TYPES.has(found.type);
    out.push({ kind: "condition", key: full, condition: found, count: remote ? -1 : count(full) });
  };
  if (!typed) {
    (exclude ? ["集中", "特課研", "英語", "演習"] : COMMON_KEYS)
      .filter((key) => key !== "推荐" || ctx.recommended.size > 0)
      .forEach(addCondition);
    return out;
  }
  const scored: [number, string][] = [];
  for (const entry of vocabulary(ctx)) {
    let best = 0;
    for (const alias of [entry.key, ...entry.aliases]) {
      const la = lower(alias);
      if (la === lt) best = Math.max(best, 3);
      else if (la.startsWith(lt)) best = Math.max(best, 2);
      else if (lt.length >= 2 && la.includes(lt)) best = Math.max(best, 1);
    }
    if (best) scored.push([best, entry.key]);
  }
  scored.sort((a, b) => b[0] - a[0]);
  const literal = condition(typed, ctx);
  // a spelling the vocabulary knows exactly (金3 → 金3-4) wins over the literal reading
  if (literal.type !== "text" && !scored.some(([score, key]) => key === literal.key || score === 3)) scored.unshift([4, literal.key]);
  scored.slice(0, 6).forEach(([, key]) => addCondition(key));
  if (!exclude) {
    const upper = typed.toUpperCase();
    const seen = new Set<string>();
    courses
      .filter((c) => (c.code.startsWith(upper) || searchable(c).includes(lt)) && !seen.has(c.code) && seen.add(c.code))
      .sort((a, b) => Number(b.code.startsWith(upper)) - Number(a.code.startsWith(upper)))
      .slice(0, 6)
      .forEach((course) => out.push({ kind: "course", course }));
  }
  const textKey = exclude ? `!${typed}` : typed;
  if (!applied.has(textKey))
    out.push({ kind: "text", key: textKey, count: textKey.startsWith("!") ? count(textKey) : -1 });
  return out;
}

/** Typed text that is not a condition narrows results like find-in-page. */
export function liveText(input: string, keys: string[], courses: FinderCourse[], ctx: FinderContext) {
  const value = input.trim();
  if (!value || value.startsWith("!") || condition(value, ctx).type !== "text") return "";
  return suggest(value, keys, courses, ctx).some((s) => s.kind === "condition") ? "" : value;
}
