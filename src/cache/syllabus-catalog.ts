import { browserCacheStorage, createCache } from "./index";
import type { SyllabusCourse } from "../core/syllabus";
import {
  syllabusIdentity,
  validSyllabusContent,
} from "../core/syllabus-content";
export type SyllabusResponse = {
  items: SyllabusCourse[];
  total: number;
  truncated: boolean;
};
const validCourse = (value: unknown): value is SyllabusCourse => {
  const c = value as SyllabusCourse;
  return (
    !!c &&
    typeof c.externalId === "string" &&
    typeof c.title === "string" &&
    typeof c.code === "string" &&
    typeof c.instructor === "string" &&
    Number.isInteger(c.year) &&
    ["1Q", "2Q", "3Q", "4Q"].includes(c.quarter) &&
    !!syllabusIdentity(c.url) &&
    (c.content == null || validSyllabusContent(c.content))
  );
};
const validResponse = (value: unknown): value is SyllabusResponse => {
  const r = value as SyllabusResponse;
  return (
    !!r &&
    Array.isArray(r.items) &&
    r.items.length <= 500 &&
    r.items.every(validCourse) &&
    Number.isInteger(r.total) &&
    r.total >= 0 &&
    typeof r.truncated === "boolean"
  );
};
export const syllabusSearchCache = createCache<SyllabusResponse>({
  namespace: "syllabus-search",
  version: 1,
  ttlMs: 10 * 60 * 1000,
  maxEntries: 20,
  maxChars: 1500000,
  validate: validResponse,
  storage: browserCacheStorage,
});
export const syllabusDetailCache = createCache<SyllabusCourse>({
  namespace: "syllabus-details",
  version: 1,
  ttlMs: 24 * 60 * 60 * 1000,
  maxEntries: 30,
  maxChars: 2500000,
  validate: validCourse,
  storage: browserCacheStorage,
});
export function savedSyllabusQueries() {
  try {
    const legacy = localStorage.getItem("stws-syllabus-public-v2");
    if (legacy && legacy.length <= 2000000) {
      const values = JSON.parse(legacy);
      if (Array.isArray(values))
        for (const row of values.slice(0, 20).reverse()) {
          if (
            row &&
            typeof row.key === "string" &&
            Number.isFinite(row.at) &&
            validResponse(row.response) &&
            !syllabusSearchCache.read(row.key)
          )
            syllabusSearchCache.set(row.key, row.response, row.at);
        }
      localStorage.removeItem("stws-syllabus-public-v2");
    }
  } catch {
    /* Invalid optional legacy data is ignored. */
  }
  return syllabusSearchCache
    .entries()
    .map((e) => ({ key: e.key, at: e.updatedAt, response: e.value }));
}
