import { t } from "../../i18n";
import { useTranslation } from "react-i18next";
import {
  hydrateSyllabusLanguages,
  syllabusLanguageUrl,
} from "../../cache/syllabus-languages";
import { useEffect, useRef, useState } from "react";
import type { SyllabusCourse } from "../../core/syllabus";
import {
  cacheSyllabusContent,
  readSyllabusContent,
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
  const { i18n } = useTranslation();
  const identity = syllabusIdentity(url)
    ? syllabusLanguageUrl(url, i18n.language)
    : undefined;
  const [content, setContent] = useState<SyllabusContent | undefined>(() =>
    identity ? readSyllabusContent(identity) : undefined,
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const generation = useRef(0);
  const fetchRef = useRef(fetchCourse);
  fetchRef.current = fetchCourse;
  async function refresh(target: string, request: number, force = false) {
    if (!fetchRef.current) return;
    setBusy(true);
    setError("");
    try {
      const course = await hydrateSyllabusLanguages(
        target,
        i18n.language,
        fetchRef.current,
        force,
      );
      if (
        !validSyllabusContent(course.content) ||
        syllabusIdentity(course.content.sourceUrl) !==
          syllabusIdentity(course.url)
      )
        throw new Error("Invalid syllabus");
      if (request !== generation.current) return;
      const persisted = cacheSyllabusContent(course.content);
      setContent(course.content);
      if (!persisted)
        setError("Showing fetched syllabus. Local cache is unavailable.");
    } catch {
      if (request === generation.current)
        setError(
          readSyllabusContent(target)
            ? "Could not refresh the syllabus. Saved content is kept."
            : "Could not fetch the syllabus. Try Refresh.",
        );
    } finally {
      if (request === generation.current) setBusy(false);
    }
  }
  useEffect(() => {
    const request = ++generation.current;
    const cached = identity ? readSyllabusContent(identity) : undefined;
    setContent(cached);
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
      <div className="syllabusDetailsHeader">
        <div>
          <h2>{t("Official syllabus")}</h2>
          <p>{t("Reference content. Your course edits stay unchanged.")}</p>
        </div>
        {identity && (
          <div className="syllabusDetailsActions">
            <a href={identity} target="_blank" rel="noreferrer">
              {t("Open source ↗")}
            </a>
            <UiButton
              className="button small"
              type="button"
              disabled={!fetchCourse || busy}
              onClick={() => void refresh(identity, ++generation.current, true)}
            >
              {busy ? "Refreshing…" : "Refresh"}
            </UiButton>
          </div>
        )}
      </div>
      <p className="syllabusStatus" role="status">
        {error ||
          (content
            ? `${t("Updated")} ${new Date(content.fetchedAt).toLocaleString(i18n.language)}` +
              ` · ${content.sourceLanguage.toUpperCase()}${content.sourceUpdated ? ` · Source updated ${content.sourceUpdated}` : ""}`
            : !identity
              ? "Add an official syllabus course URL above to load its details."
              : busy
                ? "Fetching public course details…"
                : "Syllabus not fetched. Connect through the macOS app to load it.")}
      </p>
      {content &&
        content.sourceLanguage !== (i18n.language === "ja" ? "ja" : "en") && (
          <p className="syllabusStatus">
            {t("Fetched syllabus language differs from your preference")}
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
                      <a href={link.url} target="_blank" rel="noreferrer">
                        {link.label || link.url} ↗
                      </a>
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
import { UiButton } from "../../components/mantine";
