import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { UiInput } from "../../components/mantine";
import {
  contentLanguage,
  pickLocalized,
  type TermId,
  type Workspace,
} from "../../lib/model";
import { t } from "../../i18n";
import { useScheduleQuery } from "../../platform/core-query";
import { usePageLayer } from "../../app/KeptPage";
import type { LectureSearch } from "../../generated/persistence";

const termLabel = (term: string) => term.replace("-", " · ");

/** Course search for the workstation sidebar: this term first, then older quarters. */
export function CourseFinder({
  data,
  term,
  open,
}: {
  data: Workspace;
  term: TermId;
  open: (courseId: string) => void;
}) {
  const [query, setQuery] = useState("");
  const matches = useScheduleQuery<LectureSearch>(
    { kind: "lectureSearch", term: null, query },
    Boolean(query.trim()),
  ).value;
  const [expanded, setExpanded] = useState(false);
  const [cursor, setCursor] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  // The sidebar clips overflow, so the list is portaled and placed by the input.
  const layer = usePageLayer();
  const [place, setPlace] = useState<React.CSSProperties>();
  useLayoutEffect(() => {
    if (!expanded) return;
    const measure = () => {
      const rect = input.current?.getBoundingClientRect();
      if (!rect) return;
      const width = Math.min(Math.max(rect.width, 340), window.innerWidth - 16);
      setPlace({
        top: rect.bottom + 4,
        left: Math.max(8, Math.min(rect.left, window.innerWidth - width - 8)),
        width,
        maxHeight: Math.min(420, window.innerHeight - rect.bottom - 16),
      });
    };
    measure();
    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);
    return () => {
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
    };
  }, [expanded]);
  const groups = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const byTerm = new Map<
      string,
      { id: string; title: string; code: string; color: string }[]
    >();
    for (const course of data.courses) {
      if (course.deletedAt) continue;
      const title = pickLocalized(
        course.names?.title,
        contentLanguage(data.settings),
        course.title,
      );
      if (needle && !matches?.courseIds.includes(course.id)) continue;
      const list = byTerm.get(course.termId) ?? [];
      list.push({
        id: course.id,
        title,
        code: course.code,
        color: course.color,
      });
      byTerm.set(course.termId, list);
    }
    return [...byTerm.entries()]
      .sort(([a], [b]) =>
        a === term ? -1 : b === term ? 1 : b.localeCompare(a),
      )
      .map(([id, courses]) => ({
        id,
        label: id === term ? `This term · ${termLabel(id)}` : termLabel(id),
        courses: courses.sort((a, b) => a.title.localeCompare(b.title)),
      }));
  }, [
    data.courses,
    data.settings.locale,
    data.settings.contentLanguage,
    matches,
    query,
    term,
  ]);
  const flat = groups.flatMap((group) => group.courses);
  const choose = (id: string) => {
    open(id);
    setExpanded(false);
    setQuery("");
    input.current?.blur();
  };
  let index = -1;
  return (
    <div className="courseFinder">
      <UiInput
        ref={input}
        type="search"
        placeholder={t("Find course")}
        aria-label={t("Find course")}
        value={query}
        onFocus={() => setExpanded(true)}
        onBlur={() => setExpanded(false)}
        onChange={(event) => {
          setQuery(event.target.value);
          setCursor(0);
          setExpanded(true);
        }}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            const step = event.key === "ArrowDown" ? 1 : -1;
            setCursor(
              (value) =>
                (value + step + flat.length) % Math.max(flat.length, 1),
            );
          } else if (event.key === "Enter" && flat[cursor]) {
            choose(flat[cursor].id);
          } else if (event.key === "Escape") {
            setExpanded(false);
            input.current?.blur();
          }
        }}
      />
      {expanded &&
        createPortal(
          <div className="courseFinderList" role="listbox" style={place}>
            {groups.map((group) => (
              <section key={group.id}>
                <small>{group.label}</small>
                {group.courses.map((course) => {
                  index += 1;
                  const position = index;
                  return (
                    <button
                      type="button"
                      role="option"
                      aria-selected={position === cursor}
                      className={position === cursor ? "active" : ""}
                      key={course.id}
                      // Keep focus in the input so blur does not close the list first.
                      onMouseDown={(event) => event.preventDefault()}
                      onMouseEnter={() => setCursor(position)}
                      onClick={() => choose(course.id)}
                    >
                      <i style={{ background: course.color }} />
                      <span>{course.title}</span>
                      <code>{course.code}</code>
                    </button>
                  );
                })}
              </section>
            ))}
            {!flat.length && <p>{t("No matching courses")}</p>}
          </div>,
          layer,
        )}
    </div>
  );
}
