import { syllabusDetailCache } from "./syllabus-catalog";
import {
  cacheSyllabusContent,
  syllabusIdentity,
} from "../core/syllabus-content";
import type { SyllabusCourse } from "../core/syllabus";
export type SyllabusLocale = "en" | "ja";
export const syllabusLocale = (locale: string): SyllabusLocale =>
  locale === "ja" ? "ja" : "en";
export function syllabusLanguageUrl(url: string, locale: string) {
  const identity = syllabusIdentity(url);
  if (!identity) throw new Error("Invalid syllabus URL");
  const parsed = new URL(identity);
  parsed.searchParams.set("hl", syllabusLocale(locale));
  return parsed.toString();
}
const listeners = new Set<() => void>();
export const subscribeSyllabusLanguages = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};
let active = 0;
const queue: (() => void)[] = [];
async function limited<T>(load: () => Promise<T>): Promise<T> {
  if (active >= 2) await new Promise<void>((resolve) => queue.push(resolve));
  else active++;
  try {
    return await load();
  } finally {
    const next = queue.shift();
    if (next) next();
    else active--;
  }
}
async function fetchLanguage(
  url: string,
  loader: (url: string) => Promise<SyllabusCourse>,
  force = false,
) {
  let loaded = false;
  return syllabusDetailCache
    .fetch(
      url,
      () =>
        limited(async () => {
          loaded = true;
          const course = await loader(url);
          const parsed = new URL(url);
          const segments = parsed.pathname.split("/");
          if (
            syllabusIdentity(course.url) !== url ||
            course.externalId !== segments.at(-1) ||
            course.year !== Number(segments[2]) ||
            (course.content &&
              syllabusIdentity(course.content.sourceUrl) !== url)
          )
            throw Error("Course identity mismatch");
          if (course.content) cacheSyllabusContent(course.content);
          // Notify after wrapper commits the validated value, below.
          return course;
        }),
      force,
    )
    .then((course) => {
      if (loaded) for (const listener of listeners) listener();
      return course;
    });
}
/** Only hydrate opened/selected courses. Preferred language first, partner quietly in background. */
export async function hydrateSyllabusLanguages(
  url: string,
  locale: string,
  loader: (url: string) => Promise<SyllabusCourse>,
  force = false,
): Promise<SyllabusCourse> {
  const preferred = syllabusLanguageUrl(url, locale),
    other = syllabusLanguageUrl(
      url,
      syllabusLocale(locale) === "en" ? "ja" : "en",
    );
  try {
    const course = await fetchLanguage(preferred, loader, force);
    void fetchLanguage(other, loader).catch(() => undefined);
    return course;
  } catch (error) {
    const saved = syllabusDetailCache.read(preferred)?.value;
    if (saved) {
      void fetchLanguage(other, loader).catch(() => undefined);
      return saved;
    }
    try {
      return await fetchLanguage(other, loader);
    } catch {
      throw error;
    }
  }
}
