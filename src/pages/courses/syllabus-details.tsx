import { ExternalLink } from "../../components/ExternalLink";
import { t } from "../../i18n";
import { useEffect, useRef, useState } from "react";
import type { SyllabusCourse } from "../../core/syllabus";
import {
  syllabusIdentity,
  validSyllabusContent,
  type SyllabusContent,
} from "../../core/syllabus-content";

export function SyllabusDetails({
  url,
  fetchCourse,
}: {
  url: string;
  fetchCourse?: (url: string) => Promise<SyllabusCourse>;
}) {
  // Official offering-ID locators redirect to the canonical dated syllabus.
  const identity = syllabusIdentity(url);
  const [content, setContent] = useState<SyllabusContent>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const generation = useRef(0);
  const fetchRef = useRef(fetchCourse);
  fetchRef.current = fetchCourse;
  async function refresh(target: string, request: number) {
    if (!fetchRef.current) return;
    setBusy(true);
    setError("");
    try {
      const course = await fetchRef.current(target);
      // The same course in either language (English sources carry ?hl=en).
      const page = (value: string) => syllabusIdentity(value)?.replace(/\?hl=en$/, "");
      const targetId = new URL(target).pathname.split("/").at(-1);
      const returnedId = new URL(course.url).pathname.split("/").at(-1);
      if (
        targetId !== returnedId ||
        !validSyllabusContent(course.content) ||
        page(course.content.sourceUrl) !== page(course.url)
      )
        throw new Error("Invalid syllabus");
      if (request !== generation.current) return;
      setContent(course.content);
    } catch {
      if (request === generation.current)
        setError(t("The syllabus could not be loaded."));
    } finally {
      if (request === generation.current) setBusy(false);
    }
  }
  useEffect(() => {
    const request = ++generation.current;
    setContent(undefined);
    setError("");
    setBusy(false);
    const timer =
      identity && fetchRef.current
        ? setTimeout(() => void refresh(identity, request), 350)
        : undefined;
    return () => {
      clearTimeout(timer);
      generation.current += 1;
    };
  }, [identity, Boolean(fetchCourse)]);
  return (
    <section className="syllabusDetails" aria-label={t("Official syllabus")}>
      {(error || !identity || (!content && busy)) && (
        <p className="syllabusStatus" role="status">
          {error || (!identity ? t("No syllabus is linked to this course.") : t("Loading the syllabus…"))}
        </p>
      )}
      {content && (
        <>
          {!!content.metadata.length && (
            <details>
              <summary>{t("Course information")}</summary>
              <dl className="syllabusMetadata">
                {content.metadata.map((row, i) => (
                  <div key={i}>
                    <dt>{row.label}</dt>
                    <dd>{row.value}</dd>
                  </div>
                ))}
              </dl>
            </details>
          )}
          {content.sections
            .filter(
              (section) =>
                !(
                  section.key === "lecturePlan" && content.lecturePlan.length
                ) && section.key !== "competencies",
            )
            .map((section, i) => (
              <details key={`${section.key}-${i}`} open={i === 0}>
                <summary>{section.heading}</summary>
                <div className="syllabusProse">
                  {section.blocks.map((block, j) =>
                    block.kind === "tableRow" ? (
                      <p key={j}>{block.cells?.join(" · ") || block.text}</p>
                    ) : block.kind === "subheading" ? (
                      <h3 key={j}>{block.text}</h3>
                    ) : (
                      <p key={j}>
                        {block.kind === "listItem" ? "• " : ""}
                        {block.text}
                      </p>
                    ),
                  )}
                  {section.links.map((link, j) => (
                    <p key={`link-${j}`}>
                      <ExternalLink href={link.url}>
                        {link.label || link.url} ↗
                      </ExternalLink>
                    </p>
                  ))}
                </div>
              </details>
            ))}
          {!!content.lecturePlan.length && (
            <details>
              <summary>
                Lesson plan · {content.lecturePlan.length} sessions
              </summary>
              <div className="syllabusLessonPlan">
                {content.lecturePlan.map((lesson, i) => (
                  <div key={i}>
                    <b>{lesson.label}</b>
                    <div>
                      <p>{lesson.topic}</p>
                      {lesson.objective && (
                        <p className="syllabusObjective">{lesson.objective}</p>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            </details>
          )}
          {!!content.competencies.filter((c) => c.selected).length && (
            <details>
              <summary>Competencies</summary>
              <ul>
                {content.competencies
                  .filter((c) => c.selected)
                  .map((c, i) => (
                    <li key={i}>{c.label}</li>
                  ))}
              </ul>
            </details>
          )}
          {!content.sections.length && !content.lecturePlan.length && (
            <p>No additional syllabus sections were found in this source.</p>
          )}
        </>
      )}
    </section>
  );
}
