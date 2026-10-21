import { useVirtualizer } from "@tanstack/react-virtual";
import { Paper } from "@mantine/core";
import { useEffect, useMemo, useRef, useState } from "react";

import { UiButton } from "../../components/mantine";
import {
  DAYS,
  LOCAL_TYPES,
  type Condition,
  type FinderCourse,
  type Suggestion,
} from "../../lib/course-query";
import { pickLocalized, type Lang, type TermId } from "../../lib/model";
import {
  finderQuery,
  useFinderQuery,
  type FinderAction,
  type FinderView,
} from "../../platform/finder-query";
import { runsFor } from "../../platform/sync";
import { useSync } from "../../app/useSync";
import { t } from "../../i18n";

const TAG: Partial<Record<Condition["type"], string>> = {
  quarter: "Quarter",
  time: "Time",
  code: "Code",
  dept: "Department",
  level: "Level",
  format: "Format",
  lang: "Language",
  kind: "Type",
  credits: "Credits",
};

function schedule(course: FinderCourse) {
  return (
    course.schedule
      .map((s) =>
        s.kind === "weekly" && s.day && s.periods
          ? `${DAYS[s.day - 1] ?? s.day}${s.periods[0]}-${s.periods[1]}`
          : ((
              {
                intensive: "集中",
                seminar: "特課研",
                internship: "インターン",
                tba: "未定",
                "on-demand": "オンデマンド",
              } as Record<string, string>
            )[s.kind] ?? s.kind),
      )
      .join(" ") || "—"
  );
}

function Highlight({ text, query }: { text: string; query: string }) {
  const index = query ? text.toLowerCase().indexOf(query.toLowerCase()) : -1;
  if (index < 0) return <>{text}</>;
  return (
    <>
      {text.slice(0, index)}
      <mark>{text.slice(index, index + query.length)}</mark>
      {text.slice(index + query.length)}
    </>
  );
}

const RENDER_BATCH = 100;
const message = (reason: unknown) =>
  reason instanceof Error ? reason.message : String(reason);

/**
 * Browser-address-bar style course search: type, pick a suggestion, conditions become
 * pills. Same kind = or, different kinds = and, "!" excludes. Clicking a timetable slot
 * toggles a time condition (wired by the parent through `keys`).
 */
export function CourseFinder({
  year,
  term,
  enabled,
  activate,
  language,
  keys,
  edit,
  replaceKeys,
  add,
  undo,
  manual,
  summary,
  openCourse,
  activeId,
  readOnly = false,
}: {
  year: number;
  term: TermId;
  enabled: boolean;
  activate: () => void;
  language: Lang;
  keys: string[];
  edit: (action: FinderAction) => Promise<FinderView | undefined>;
  replaceKeys: (keys: string[] | ((current: string[]) => string[])) => void;
  add: (courses: FinderCourse[]) => Promise<unknown>;
  undo: (courses: FinderCourse[]) => Promise<unknown>;
  manual?: () => void;
  /** Shows a result's syllabus details (over the timetable). */
  openCourse?: (course: FinderCourse) => void;
  /** The result whose details are open. */
  activeId?: string;
  readOnly?: boolean;
  /** What is on this quarter's timetable, e.g. "3 on timetable · 6 credits". */
  summary?: string;
}) {
  const [error, setError] = useState("");
  const [input, setInput] = useState("");
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [lastAdded, setLastAdded] = useState<FinderCourse[] | null>(null);
  const [mutating, setMutating] = useState(false);
  const [visibleCount, setVisibleCount] = useState(RENDER_BATCH);
  const resultsRef = useRef<HTMLDivElement>(null);
  const [showUnavailable, setShowUnavailable] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => setShowUnavailable(false), [year, term]);

  // Search is local only. Core's planner downloads the year's catalog
  // (`catalog.official`); while it runs, the query is re-read when it lands.
  const sync = useSync();
  const catalogRuns = runsFor(sync, ["catalog.official"], String(year));
  // The catalog is the 教務 course search: it waits for a 教務 sign-in.
  const signInNeeded = catalogRuns.some(
    (run) => run.held?.kind === "sign-in-required",
  );
  const downloading = catalogRuns.some((run) => !run.failure && !run.held);
  // Catalog rows commit without a workspace revision, so each finished
  // download is its own refresh key; a boolean would reuse the empty answer
  // cached before the download began.
  const [catalogLanded, setCatalogLanded] = useState(0);
  const wasDownloading = useRef(downloading);
  useEffect(() => {
    if (wasDownloading.current && !downloading)
      setCatalogLanded((count) => count + 1);
    wasDownloading.current = downloading;
  }, [downloading]);
  useEffect(() => {
    const focus = (event: KeyboardEvent) => {
      if (
        (event.metaKey || event.ctrlKey) &&
        event.key.toLowerCase() === "k" &&
        inputRef.current?.offsetParent
      ) {
        event.preventDefault();
        inputRef.current.focus();
        inputRef.current.select();
      }
    };
    addEventListener("keydown", focus);
    return () => removeEventListener("keydown", focus);
  }, []);

  const planning = useFinderQuery(
    {
      year,
      term,
      keys,
      includeIneligible: showUnavailable,
    },
    0,
    enabled,
  );
  const result = useFinderQuery(
    {
      year,
      term,
      keys,
      includeIneligible: showUnavailable,
      input,
    },
    catalogLanded,
    enabled,
  );
  const view = result.value;
  const courses = view?.courses;
  const list = view?.courses ?? [];
  // Only an empty year waits on the download; a stored catalog stays usable
  // while Core renews it.
  const firstDownload = downloading && list.length === 0;
  const factors = view?.factors ?? planning.value?.factors ?? [];
  const duplicates = view?.duplicates ?? 0;
  const onTimetable = new Set(view?.onTimetable ?? []);
  const ineligible = new Set(view?.ineligibleIds ?? []);
  const eligibilityLabel = (id: string) => {
    switch (view?.eligibility[id]?.reason) {
      case "undergraduateFirstYearUpperLevel":
        return t("Not available to regular first-year undergraduate students");
      case "masterFirstSemesterHumanitiesUpperLevel":
        return t("Available to master's students from their second semester");
      case "graduateCourseApprovalRequired":
        return t(
          "Course approval after the research proposal presentation must be confirmed",
        );
      case "leadershipWorkshopGradeRequired":
        return t(
          "A Leadership Workshop grade of 80 or higher must be confirmed",
        );
      case "doctoralFirstSemesterGenerallyRestricted":
        return t(
          "Generally unavailable during the first six months; confirm any exception",
        );
      case "doctoralFirstSemesterRestricted":
        return t("Not available during the first six months of doctoral study");
      case "separateApplicationRequired":
        return t("A separate course application must be confirmed");
      case "ruleYearUnsupported":
        return t(
          "This rule is not yet verified for the selected academic year",
        );
      default:
        return undefined;
    }
  };
  const unavailableLabel = (id: string) =>
    eligibilityLabel(id) ?? t("Not available for this study profile");

  const live = view?.live ?? "";
  const rows = view?.rows ?? [];
  const shown = rows.slice(0, visibleCount);
  const virtualRows = useVirtualizer({
    count: shown.length,
    getScrollElement: () => resultsRef.current,
    estimateSize: () => 96,
    getItemKey: (index) => shown[index].course.id,
    overscan: 5,
    // React Activity may reveal the list during a lifecycle commit.
    useFlushSync: false,
  });
  // Rows are local; this only widens how many are rendered at once.
  const loadMore = () => {
    if (visibleCount < rows.length)
      setVisibleCount((count) => count + RENDER_BATCH);
  };
  // IntersectionObserver follows the scroll container. A button exposes the
  // same action to keyboard users.
  const sentinel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!sentinel.current || typeof IntersectionObserver === "undefined")
      return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) loadMore();
      },
      { root: resultsRef.current, rootMargin: "240px" },
    );
    observer.observe(sentinel.current);
    return () => observer.disconnect();
  }, [visibleCount, rows.length]); // eslint-disable-line react-hooks/exhaustive-deps
  const conditionsKey = JSON.stringify([year, term, keys, showUnavailable]);
  useEffect(() => {
    setVisibleCount(RENDER_BATCH);
    resultsRef.current?.scrollTo?.({ top: 0 });
  }, [conditionsKey]);
  useEffect(() => setVisibleCount(RENDER_BATCH), [live]);
  const availableSuggestions = view?.suggestions ?? [];
  const suggestions: Suggestion[] = open ? availableSuggestions : [];
  const byId = useMemo(() => new Map(list.map((c) => [c.id, c])), [list]);
  const chosen = [...selected]
    .map((id) => byId.get(id))
    .filter((c): c is FinderCourse => Boolean(c))
    .filter((c) => !ineligible.has(c.id));
  const chosenCredits = chosen.reduce(
    (sum, c) => sum + (view?.credits[c.id] ?? 0),
    0,
  );
  const chosenClashes = chosen.filter((c) =>
    view?.clashes.includes(c.id),
  ).length;
  const editSequence = useRef(0);
  const commit = (key: string) => {
    const sequence = ++editSequence.current;
    void edit({ kind: "commit", key })
      .then((next) => {
        if (!next || sequence !== editSequence.current) return;
        setInput("");
        setActive(0);
      })
      .catch((reason) => setError(message(reason)));
  };
  const toggle = (course: FinderCourse) => {
    if (onTimetable.has(course.code) || ineligible.has(course.id)) return;
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(course.id)) next.delete(course.id);
      else next.add(course.id);
      return next;
    });
  };
  const apply = (s: Suggestion | undefined) => {
    if (!s) return;
    if (s.kind === "category") {
      ++editSequence.current;
      setInput(s.key);
      setActive(0);
      setOpen(true);
      inputRef.current?.focus();
    } else if (s.kind === "course")
      toggle(s.course); // keep the text: several matches can be picked in a row
    else commit(s.key);
  };
  const addCourses = async (courses: FinderCourse[]) => {
    if (mutating) return;
    setMutating(true);
    setError("");
    try {
      await add(courses);
      setLastAdded(courses);
      setSelected((current) => {
        const next = new Set(current);
        courses.forEach((course) => next.delete(course.id));
        return next;
      });
    } catch (reason) {
      setError(message(reason));
    } finally {
      setMutating(false);
    }
  };
  const addSelected = () => {
    if (!chosen.length) return;
    void addCourses(chosen);
  };
  const undoLast = async () => {
    if (!lastAdded || mutating) return;
    setMutating(true);
    setError("");
    try {
      await undo(lastAdded);
      setLastAdded(null);
    } catch (reason) {
      setError(message(reason));
    } finally {
      setMutating(false);
    }
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing || event.keyCode === 229) return;
    if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
      event.preventDefault();
      addSelected();
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (!availableSuggestions.length) return;
      setOpen(true);
      setActive((i) =>
        !open
          ? event.key === "ArrowDown"
            ? 0
            : availableSuggestions.length - 1
          : (i +
              (event.key === "ArrowDown" ? 1 : -1) +
              availableSuggestions.length) %
            availableSuggestions.length,
      );
    } else if (event.key === "Enter") {
      event.preventDefault();
      if (!input.trim() && chosen.length) addSelected();
      else if (suggestions[active]) apply(suggestions[active]);
      else if (input.trim()) commit(input.trim());
    } else if (event.key === "Tab" && input.trim() && suggestions.length) {
      event.preventDefault();
      apply(suggestions[active] ?? suggestions[0]);
    } else if (event.key === "Escape") {
      setOpen(false);
    } else if (event.key === "Backspace" && !input && keys.length) {
      replaceKeys((current) => current.slice(0, -1));
    }
  };
  const onChange = (value: string, composing = false) => {
    activate();
    const sequence = ++editSequence.current;
    setInput(value);
    setActive(0);
    setOpen(true);
    if (!composing && /\s$/.test(value) && value.trim()) {
      void finderQuery({
        year,
        term,
        keys,
        includeIneligible: showUnavailable,
        input: value,
      })
        .then((next) => {
          if (sequence === editSequence.current && next.commitKey)
            commit(next.commitKey);
        })
        .catch((reason) => setError(message(reason)));
    }
  };
  const onPaste = (event: React.ClipboardEvent<HTMLInputElement>) => {
    const text = event.clipboardData.getData("text");
    event.preventDefault();
    const field = event.currentTarget;
    const pasted =
      input.slice(0, field.selectionStart ?? input.length) +
      text +
      input.slice(field.selectionEnd ?? input.length);
    const sequence = ++editSequence.current;
    void edit({ kind: "paste", text })
      .then((next) => {
        if (sequence !== editSequence.current || !next) return;
        if (!next.pasteHandled) {
          onChange(pasted);
          return;
        }
        setSelected((current) => new Set([...current, ...next.pastedIds]));
        setError(
          [
            next.ineligibleCodes.length
              ? `${t("Not eligible")}: ${next.ineligibleCodes.join(", ")}`
              : "",
            next.missingCodes.length
              ? `${t("Not found")}: ${next.missingCodes.join(", ")}`
              : "",
          ]
            .filter(Boolean)
            .join(" · "),
        );
      })
      .catch((reason) => setError(message(reason)));
  };

  const pillLabel = (c: Condition) =>
    c.type === "code" ||
    c.type === "time" ||
    c.type === "level" ||
    c.type === "dept" ||
    c.type === "quarter"
      ? c.label
      : t(c.label);
  const filteredByText = live ? `· ${t("contains")} “${live}”` : "";

  return (
    <Paper
      component="aside"
      className="listCard courseFinder"
      withBorder
      shadow="xs"
      radius="md"
    >
      <header>
        <div className="finderTitle">
          <b>
            {term.replace("-", " ")}
            {summary && <small> · {summary}</small>}
          </b>
          {enabled && (
            <span>
              {firstDownload
                ? t("Downloading the {{year}} course catalog", { year })
                : courses
                  ? `${list.length} ${t("stored")}`
                  : t("Loading…")}
            </span>
          )}
        </div>
        <div className="courseListTools">
          {(showUnavailable || Boolean(view?.ineligibleIds.length)) && (
            <UiButton
              type="button"
              aria-pressed={showUnavailable}
              onClick={() => setShowUnavailable((show) => !show)}
            >
              {showUnavailable
                ? t("Hide unavailable courses")
                : t("Show unavailable courses")}
            </UiButton>
          )}
          {manual && (
            <UiButton type="button" onClick={manual}>
              {t("Manual")}
            </UiButton>
          )}
        </div>
      </header>

      <div className="finderOmni">
        <div className="finderBar" onClick={() => inputRef.current?.focus()}>
          {factors.map((f) =>
            f.alts.map((c, i) => (
              <span key={c.key} className="finderPillGroup">
                {i > 0 && <span className="finderOr">/</span>}
                <button
                  type="button"
                  className={`finderPill ${c.exclude ? "exclude" : c.type}`}
                  title={t("Remove")}
                  onClick={(event) => {
                    event.stopPropagation();
                    replaceKeys((current) =>
                      current.filter((key) => key !== c.key),
                    );
                  }}
                >
                  {c.exclude ? (
                    <b>{t("Not")}</b>
                  ) : TAG[c.type] ? (
                    <b>{t(TAG[c.type]!)}</b>
                  ) : null}
                  {pillLabel(c)}
                  {LOCAL_TYPES.has(c.type) && !c.exclude && (
                    <i className="finderLocal">{t("local")}</i>
                  )}
                  <span aria-hidden="true">×</span>
                </button>
              </span>
            )),
          )}
          <input
            ref={inputRef}
            id="course-finder-input"
            value={input}
            role="combobox"
            aria-expanded={open && suggestions.length > 0}
            aria-controls="course-finder-suggestions"
            aria-activedescendant={
              open && suggestions.length
                ? `course-finder-s${active}`
                : undefined
            }
            aria-label={t("Find courses")}
            autoComplete="off"
            spellCheck={false}
            placeholder={
              keys.length ? "" : t("Title, code, 火3-4, teacher:小林, room:W9…")
            }
            onChange={(event) =>
              onChange(
                event.target.value,
                (event.nativeEvent as InputEvent).isComposing,
              )
            }
            onKeyDown={onKeyDown}
            onPaste={onPaste}
            onFocus={() => {
              activate();
              setOpen(true);
            }}
            onBlur={() => setTimeout(() => setOpen(false), 120)}
          />
        </div>
        {open && suggestions.length > 0 && (
          <div
            className="finderSuggestions"
            id="course-finder-suggestions"
            role="listbox"
            onMouseDown={(event) => event.preventDefault()}
          >
            {suggestions.map((s, i) => {
              const head = {
                category: t("Filter categories"),
                condition: input.trim() ? t("Conditions") : t("Common"),
                course: t("Courses"),
                text: t("Search"),
              }[s.kind];
              const prev = suggestions[i - 1];
              const showHead = !prev || prev.kind !== s.kind;
              const query = input.replace(/^!/, "").trim();
              return (
                <div
                  key={`${s.kind}-${s.kind === "course" ? s.course.id : s.key}`}
                >
                  {showHead && <div className="finderHead">{head}</div>}
                  <div
                    id={`course-finder-s${i}`}
                    role="option"
                    aria-selected={i === active}
                    className="finderSuggestion"
                    onMouseEnter={() => setActive(i)}
                    onClick={() => apply(s)}
                  >
                    {s.kind === "category" && (
                      <>
                        <span>{t(s.label)}</span>
                        <small>›</small>
                      </>
                    )}
                    {s.kind === "condition" && (
                      <>
                        <span>
                          {s.condition.exclude ? (
                            <b>{t("Not")}</b>
                          ) : TAG[s.condition.type] ? (
                            <b>{t(TAG[s.condition.type]!)}</b>
                          ) : null}
                          <Highlight
                            text={pillLabel(s.condition)}
                            query={query}
                          />
                        </span>
                        {s.count >= 0 && (
                          <small className={s.count ? "" : "zero"}>
                            {s.count}
                          </small>
                        )}
                      </>
                    )}
                    {s.kind === "course" && (
                      <>
                        <span>
                          {selected.has(s.course.id) && (
                            <i className="finderCheck">✓</i>
                          )}
                          <Highlight
                            text={pickLocalized(s.course.title, language)}
                            query={query}
                          />
                          <em>
                            <Highlight text={s.course.code} query={query} /> ·{" "}
                            {schedule(s.course)} · {s.course.quarters.join(" ")}
                          </em>
                        </span>
                        <small>
                          {onTimetable.has(s.course.code)
                            ? t("On timetable")
                            : selected.has(s.course.id)
                              ? t("Selected")
                              : t("Enter to select")}
                        </small>
                      </>
                    )}
                    {s.kind === "text" && (
                      <>
                        <span>
                          {s.key.startsWith("!")
                            ? `${t("Not")} “${query}”`
                            : `${t("Search for")} “${query}”`}
                        </span>
                        {s.count >= 0 && (
                          <small className={s.count ? "" : "zero"}>
                            {s.count}
                          </small>
                        )}
                      </>
                    )}
                  </div>
                </div>
              );
            })}
            <div className="finderKeys">
              <span>
                <kbd>↑↓</kbd> {t("choose")}
              </span>
              <span>
                <kbd>↵</kbd> {t("apply")}
              </span>
              <span>
                <kbd>Tab</kbd> {t("complete")}
              </span>
              <span>
                <kbd>!</kbd> {t("exclude")}
              </span>
            </div>
          </div>
        )}
      </div>

      <div className="finderStatus">
        <span>
          {rows.length} {t("courses")} {filteredByText}
          {duplicates > 0 && ` · ${t("duplicates merged")}`}
        </span>
        {(error || result.error || planning.error) && (
          <span className="finderError">
            {error || result.error || planning.error}
          </span>
        )}
      </div>

      <div className="finderResults" ref={resultsRef} aria-busy={firstDownload}>
        {enabled && !courses && !error && (
          <div className="empty">{t("Loading…")}</div>
        )}
        {courses && rows.length === 0 && (
          <div className="empty finderEmpty">
            <span>
              {firstDownload
                ? t(
                    "Downloading the {{year}} course catalog. Results appear when it is saved.",
                    { year },
                  )
                : courses.length === 0 && signInNeeded
                  ? t("Sign in to 教務 to search courses")
                  : courses.length === 0
                  ? t("No {{year}} course catalog is saved yet.", { year })
                  : t(
                      "No matching courses. Remove a condition to widen the search.",
                    )}
            </span>
          </div>
        )}
        <div
          style={{ height: virtualRows.getTotalSize(), position: "relative" }}
        >
          {virtualRows.getVirtualItems().map((virtualRow) => {
            const { course, matched } = shown[virtualRow.index];
            const placed = onTimetable.has(course.code);
            const unavailable = ineligible.has(course.id);
            const eligibilityNote = eligibilityLabel(course.id);
            const eligibilityUnknown =
              view?.eligibility[course.id]?.status === "unknown" &&
              Boolean(eligibilityNote);
            const rec = view?.context.recommended[course.code];
            const clash = !placed && Boolean(view?.clashes.includes(course.id));
            const replacing =
              clash && Boolean(view?.replacements[course.id]?.length);
            return (
              <div
                key={course.id}
                data-index={virtualRow.index}
                ref={virtualRows.measureElement}
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  width: "100%",
                  transform: `translateY(${virtualRow.start}px)`,
                }}
                className={`finderRow${placed ? " placed" : ""}${activeId === course.id ? " active" : ""}`}
              >
                <input
                  type="checkbox"
                  id={`finder-${course.id}`}
                  aria-label={t("Select")}
                  checked={selected.has(course.id)}
                  disabled={placed || unavailable || readOnly}
                  title={unavailable ? unavailableLabel(course.id) : undefined}
                  onChange={() => toggle(course)}
                />
                <button
                  type="button"
                  className="finderMain"
                  onClick={() => openCourse?.(course)}
                >
                  <b>
                    <Highlight
                      text={pickLocalized(course.title, language)}
                      query={live}
                    />
                  </b>
                  <small>
                    <code>{course.code}</code> · {schedule(course)} ·{" "}
                    {course.quarters.join(" ")}
                    {` · ${course.credits.lecture}-${course.credits.exercise}-${course.credits.lab}`}
                    {matched.length > 0 && <em> · {matched.join(" / ")}</em>}
                  </small>
                  <span className="finderBadges">
                    {rec && (
                      <i className="rec">
                        {rec.group}
                        {rec.mark === "required"
                          ? " ◎"
                          : rec.mark
                            ? " ○"
                            : " 教務"}
                      </i>
                    )}
                    {clash && <i className="clash">{t("Conflict")}</i>}
                    {unavailable && (
                      <i className="clash">{unavailableLabel(course.id)}</i>
                    )}
                    {eligibilityUnknown && (
                      <i className="eligibilityUnknown">{eligibilityNote}</i>
                    )}
                  </span>
                </button>
                {placed ? (
                  <i className="finderPlaced">{t("Added")}</i>
                ) : (
                  <UiButton
                    type="button"
                    className={`finderAdd${replacing ? " replace" : ""}`}
                    disabled={readOnly || mutating || unavailable}
                    title={
                      unavailable
                        ? unavailableLabel(course.id)
                        : replacing
                          ? t("Replaces the course in the same periods")
                          : undefined
                    }
                    onClick={() => void addCourses([course])}
                  >
                    {replacing ? t("Replace") : t("Add")}
                  </UiButton>
                )}
              </div>
            );
          })}
        </div>
        <div ref={sentinel} className="finderLoadMore">
          {shown.length < rows.length && (
            <UiButton className="textButton" onClick={loadMore}>
              {t("Load more courses")}
            </UiButton>
          )}
        </div>
      </div>

      <footer className="finderFoot">
        {lastAdded ? (
          <span>
            {t("Added {{count}}", { count: lastAdded.length })} ·{" "}
            <button
              type="button"
              className="textButton"
              disabled={mutating}
              onClick={() => void undoLast()}
            >
              {t("Undo")}
            </button>
          </span>
        ) : (
          <span>
            {chosen.length
              ? `${chosen.length} ${t("selected")} · ${chosenCredits} ${t("credits")}${chosenClashes ? ` · ${chosenClashes} ${t("conflicts")}` : ""}`
              : t("Nothing selected")}
          </span>
        )}
        <UiButton
          type="button"
          className="button"
          disabled={!chosen.length || mutating}
          onClick={() => setSelected(new Set())}
        >
          {t("Clear")}
        </UiButton>
        <UiButton
          type="button"
          className="button primary"
          disabled={!chosen.length || mutating}
          onClick={addSelected}
        >
          {chosen.length
            ? t("Add {{count}}", { count: chosen.length })
            : t("Add")}
        </UiButton>
      </footer>
    </Paper>
  );
}
