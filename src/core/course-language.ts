import englishExamples from "../data/syllabus-examples.json";
import japaneseExamples from "../data/syllabus-examples-ja.json";
import type { Course } from "./model";
import { syllabusDetailCache } from "../cache/syllabus-catalog";
import { syllabusLanguageUrl } from "../cache/syllabus-languages";
import { isSyllabusCourse } from "./syllabus";
const exampleVariants = [...englishExamples, ...japaneseExamples];
const variant = (url: string) =>
  syllabusDetailCache.read(url)?.value ??
  exampleVariants.find((item) => item.url === url);

/** Presentation only: never rewrite workspace fields or infer translations for LMS/manual courses. */
export function getLocalizedCourse(course: Course, locale: string): Course {
  if (!isSyllabusCourse(course) || !course.url) return course;
  try {
    const translated = variant(syllabusLanguageUrl(course.url, locale));
    if (!translated) return course;
    const original =
      course.syllabusSource ??
      variant(
        syllabusLanguageUrl(
          course.url,
          new URL(course.url).searchParams.get("hl") ?? "ja",
        ),
      );
    if (!original) return course;
    return {
      ...course,
      title:
        course.overrides?.title ??
        (course.title === original.title ? translated.title : course.title),
      instructor:
        course.overrides?.instructor ??
        (course.instructor === original.instructor
          ? translated.instructor
          : course.instructor),
      language:
        course.language === original.language
          ? (translated.language ?? course.language)
          : course.language,
    };
  } catch {
    return course;
  }
}
