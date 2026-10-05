import { Paper } from "@mantine/core";
import { useEffect, useMemo, useRef, useState } from "react";

import { UiButton } from "../../components/mantine";
import {
  DAYS,
  factorsOf,
  localFactors,
  remotePlan,
  liveText,
  normalizedKeys,
  run,
  suggest,
  totalCredits,
  clashes,
  LOCAL_TYPES,
  type Condition,
  type FinderContext,
  type FinderCourse,
  type Suggestion,
} from "../../core/course-query";
import type { Localized, TermId } from "../../core/model";
import { SYLLABUS_DEPARTMENTS } from "../../core/departments";
import type { DesktopBridge } from "../../platform/desktop-bridge";
import { t } from "../../i18n";

const pick = (value: Localized | undefined, locale: string) =>
  (locale === "ja" ? value?.ja ?? value?.en : value?.en ?? value?.ja) ?? "";

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
  if (course.detailed === false) return "…";
  return (
    course.schedule
      .map((s) =>
        s.kind === "weekly" && s.day && s.periods
          ? `${DAYS[s.day - 1] ?? s.day}${s.periods[0]}-${s.periods[1]}`
          : ({ intensive: "集中", seminar: "特課研", internship: "インターン", tba: "未定", "on-demand": "オンデマンド" } as Record<string, string>)[s.kind] ?? s.kind,
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

const PAGE_SIZE = 25;
const DETAIL_BATCH = 6;
const scopeOf = (unitCode: string) => SYLLABUS_DEPARTMENTS.find((d) => d.unitCode === unitCode)?.scope;
const message = (reason: unknown) => (reason instanceof Error ? reason.message : String(reason));

type Remote =
  | { state: "idle" }
  | { state: "searching" }
  | { state: "unsupported"; why: "empty" | "too-many" }
  | { state: "done"; ids: Set<string>; total: number; truncated: boolean };

/**
 * Browser-address-bar style course search: type, pick a suggestion, conditions become
 * pills. Same kind = or, different kinds = and, "!" excludes. Clicking a timetable slot
 * toggles a time condition (wired by the parent through `keys`).
 */
export function CourseFinder({
  year,
  term,
  locale,
  load,
  context,
  onTimetable,
  keys,
  setKeys,
  add,
  undo,
  manual,
  summary,
  openCourse,
  activeId,
  replaces,
  readOnly = false,
  query,
  syncDetails,
}: {
  year: number;
  term: TermId;
  locale: string;
  load: (year: number) => Promise<FinderCourse[]>;
  context: FinderContext;
  /** Course codes already on the timetable for this quarter. */
  onTimetable: ReadonlySet<string>;
  keys: string[];
  setKeys: (keys: string[]) => void;
  add: (courses: FinderCourse[]) => void;
  undo: (courses: FinderCourse[]) => void;
  manual?: () => void;
  /** Shows a result's syllabus details (over the timetable). */
  openCourse?: (course: FinderCourse) => void;
  /** The result whose details are open. */
  activeId?: string;
  /** Whether Add would replace a course in the same periods. */
  replaces?: (course: FinderCourse) => boolean;
  readOnly?: boolean;
  /** What is on this quarter's timetable, e.g. "3 on timetable · 6 credits". */
  summary?: string;
  /** One official syllabus search; absent outside the desktop app. */
  query?: NonNullable<DesktopBridge["catalogQuery"]>;
  /** Detail pages (schedule, credits) for stored search rows. */
  syncDetails?: NonNullable<DesktopBridge["catalogSyncOfferings"]>;
}) {
  const [courses, setCourses] = useState<FinderCourse[] | null>(null);
  const [error, setError] = useState("");
  const [input, setInput] = useState("");
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [lastAdded, setLastAdded] = useState<FinderCourse[] | null>(null);
  const [remote, setRemote] = useState<Remote>({ state: "idle" });
  const [details, setDetails] = useState<{ done: number; total: number } | null>(null);
  const [page, setPage] = useState(0);
  const [refresh, setRefresh] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const reload = () =>
    load(year)
      .then((fresh) => {
        setCourses(fresh);
        return fresh;
      })
      .catch((reason) => {
        setError(message(reason));
        return [] as FinderCourse[];
      });
  useEffect(() => void reload(), [year]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    const focus = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k" && inputRef.current?.offsetParent) {
        event.preventDefault();
        inputRef.current.focus();
        inputRef.current.select();
      }
    };
    addEventListener("keydown", focus);
    return () => removeEventListener("keydown", focus);
  }, []);

  const list = courses ?? [];
  const { factors, duplicates } = useMemo(() => factorsOf(keys, context), [keys, context]);
  const local = useMemo(() => localFactors(factors), [factors]);
  const plan = useMemo(() => remotePlan(factors, scopeOf), [factors]);
  const planKey = JSON.stringify(plan);

  // Pills the syllabus understands become one search (no paging, ≤100 rows);
  // detail pages are then fetched for those rows only, a few at a time.
  useEffect(() => {
    setPage(0);
    if ("unsupported" in plan) return setRemote({ state: "unsupported", why: plan.unsupported });
    if (!query) return setRemote({ state: "idle" });
    let cancelled = false;
    setRemote({ state: "searching" });
    setDetails(null);
    setError("");
    const timer = setTimeout(async () => {
      try {
        const results = await Promise.all(plan.queries.map((q) => query(year, q)));
        if (cancelled) return;
        const ids = new Set(results.flatMap((r) => r.ids));
        setRemote({
          state: "done",
          ids,
          total: results.reduce((sum, r) => sum + r.total, 0),
          truncated: results.some((r) => r.truncated),
        });
        const fresh = await reload();
        if (cancelled || !syncDetails) return;
        // Only rows of the planned quarter(s) need schedules.
        const quarterOnly = factors.filter((f) => f.type === "quarter");
        const pending = run(fresh.filter((c) => ids.has(c.id)), quarterOnly, context)
          .map((r) => r.course)
          .filter((c) => c.detailed === false);
        if (!pending.length) return;
        const problems: string[] = [];
        setDetails({ done: 0, total: pending.length });
        for (let i = 0; i < pending.length && !cancelled; i += DETAIL_BATCH) {
          const batch = pending.slice(i, i + DETAIL_BATCH).map((c) => c.id);
          problems.push(...(await syncDetails(batch).catch((reason) => [message(reason)])));
          if (cancelled) return;
          setDetails({ done: Math.min(i + DETAIL_BATCH, pending.length), total: pending.length });
          await reload();
        }
        if (!cancelled) {
          setDetails(null);
          if (problems.length) setError(problems.slice(0, 3).join(" · "));
        }
      } catch (reason) {
        if (!cancelled) {
          setRemote({ state: "idle" });
          setError(message(reason));
        }
      }
    }, 250);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [planKey, year, refresh]); // eslint-disable-line react-hooks/exhaustive-deps

  // Without a syllabus answer the finder works on what is stored locally.
  const base = useMemo(
    () => (remote.state === "done" ? list.filter((c) => remote.ids.has(c.id)) : remote.state === "idle" ? list : []),
    [list, remote],
  );
  const live = useMemo(() => liveText(input, keys, base, context), [input, keys, base, context]);
  const rows = useMemo(
    () => run(base, remote.state === "done" ? local : factors, context, live),
    [base, remote.state, local, factors, context, live],
  );
  const pages = Math.max(1, Math.ceil(rows.length / PAGE_SIZE));
  const shown = rows.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE);
  useEffect(() => setPage(0), [live]);
  const suggestions: Suggestion[] = useMemo(
    () => (open ? suggest(input, keys, base, context) : []),
    [open, input, keys, base, context],
  );
  const byId = useMemo(() => new Map(list.map((c) => [c.id, c])), [list]);
  const chosen = [...selected].map((id) => byId.get(id)).filter((c): c is FinderCourse => Boolean(c));
  const chosenCredits = chosen.reduce((sum, c) => sum + totalCredits(c), 0);
  const chosenClashes = chosen.filter((c) => clashes(c, context.busy)).length;

  const commit = (key: string) => {
    setKeys(normalizedKeys([...keys, key], context));
    setInput("");
    setActive(0);
  };
  const toggle = (course: FinderCourse) => {
    if (onTimetable.has(course.code)) return;
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(course.id)) next.delete(course.id);
      else next.add(course.id);
      return next;
    });
  };
  const apply = (s: Suggestion | undefined) => {
    if (!s) return;
    if (s.kind === "course") toggle(s.course); // keep the text: several matches can be picked in a row
    else commit(s.key);
  };
  const addOne = (course: FinderCourse) => {
    add([course]);
    setLastAdded([course]);
    setSelected((current) => {
      const next = new Set(current);
      next.delete(course.id);
      return next;
    });
  };
  const addSelected = () => {
    if (!chosen.length) return;
    add(chosen);
    setLastAdded(chosen);
    setSelected(new Set());
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (!suggestions.length) return;
      setOpen(true);
      setActive((i) => (i + (event.key === "ArrowDown" ? 1 : -1) + suggestions.length) % suggestions.length);
    } else if (event.key === "Enter") {
      event.preventDefault();
      if (!input.trim() && chosen.length) addSelected();
      else apply(suggestions[active]);
    } else if (event.key === "Tab" && input.trim() && suggestions.length) {
      event.preventDefault();
      apply(suggestions.find((s) => s.kind !== "course") ?? suggestions[0]);
    } else if (event.key === "Escape") {
      setOpen(false);
    } else if (event.key === "Backspace" && !input && keys.length) {
      setKeys(keys.slice(0, -1));
    }
  };
  const onChange = (value: string) => {
    // A complete condition followed by a space becomes a pill.
    if (/\s$/.test(value) && value.trim()) {
      const typed = value.trim();
      const top = suggest(typed, keys, list, context)[0];
      const exact = top && top.kind === "condition" && top.condition.type !== "text" &&
        normalizedKeys([typed], context)[0] === top.key;
      if (exact) return commit(top.key);
    }
    setInput(value);
    setActive(0);
    setOpen(true);
  };
  const onPaste = (event: React.ClipboardEvent<HTMLInputElement>) => {
    const text = event.clipboardData.getData("text").normalize("NFKC");
    const codes = [...new Set((text.match(/[A-Za-z]{3}\.[A-Za-z]\d{3}/g) ?? []).map((c) => c.toUpperCase()))];
    if (codes.length < 2) return;
    event.preventDefault();
    const quarter = Number(term.slice(-2, -1));
    setSelected((current) => {
      const next = new Set(current);
      for (const code of codes) {
        const match = list.find((c) => c.code === code && c.quarters.includes(`${quarter}Q` as never)) ?? list.find((c) => c.code === code);
        if (match && !onTimetable.has(match.code)) next.add(match.id);
      }
      return next;
    });
    const missing = codes.filter((code) => !list.some((c) => c.code === code));
    setError(missing.length ? `${t("Not found")}: ${missing.join(", ")}` : "");
  };

  const pillLabel = (c: Condition) => (c.type === "code" || c.type === "time" || c.type === "level" || c.type === "dept" || c.type === "quarter" ? c.label : t(c.label));
  const filteredByText = live ? `· ${t("contains")} “${live}”` : "";

  return (
    <Paper component="aside" className="listCard courseFinder" withBorder shadow="xs" radius="md">
      <header>
        <div className="finderTitle">
          <b>
            {term.replace("-", " ")}
            {summary && <small> · {summary}</small>}
          </b>
          <span>
            {remote.state === "searching"
              ? t("Searching the syllabus…")
              : details
                ? `${t("Loading details")} ${details.done}/${details.total}`
                : remote.state === "done"
                  ? `${remote.total} ${t("in syllabus")}`
                  : courses
                    ? `${list.length} ${t("stored")}`
                    : t("Loading…")}
          </span>
        </div>
        <div className="courseListTools">
          {manual && <UiButton type="button" onClick={manual}>{t("Manual")}</UiButton>}
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
                  onClick={(event) => { event.stopPropagation(); setKeys(keys.filter((k) => k !== c.key)); }}
                >
                  {c.exclude ? <b>{t("Not")}</b> : TAG[c.type] ? <b>{t(TAG[c.type]!)}</b> : null}
                  {pillLabel(c)}
                  {LOCAL_TYPES.has(c.type) && !c.exclude && <i className="finderLocal">{t("local")}</i>}
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
            aria-activedescendant={open && suggestions.length ? `course-finder-s${active}` : undefined}
            aria-label={t("Find courses")}
            autoComplete="off"
            spellCheck={false}
            placeholder={keys.length ? "" : t("Title, code, 火3-4, rec…")}
            onChange={(event) => onChange(event.target.value)}
            onKeyDown={onKeyDown}
            onPaste={onPaste}
            onFocus={() => setOpen(true)}
            onBlur={() => setTimeout(() => setOpen(false), 120)}
          />
        </div>
        {open && suggestions.length > 0 && (
          <div className="finderSuggestions" id="course-finder-suggestions" role="listbox" onMouseDown={(event) => event.preventDefault()}>
            {suggestions.map((s, i) => {
              const head = { condition: input.trim() ? t("Conditions") : t("Common"), course: t("Courses"), text: t("Search") }[s.kind];
              const prev = suggestions[i - 1];
              const showHead = !prev || prev.kind !== s.kind;
              const query = input.replace(/^!/, "").trim();
              return (
                <div key={`${s.kind}-${s.kind === "course" ? s.course.id : s.key}`}>
                  {showHead && <div className="finderHead">{head}</div>}
                  <div
                    id={`course-finder-s${i}`}
                    role="option"
                    aria-selected={i === active}
                    className="finderSuggestion"
                    onMouseEnter={() => setActive(i)}
                    onClick={() => apply(s)}
                  >
                    {s.kind === "condition" && (
                      <>
                        <span>
                          {s.condition.exclude ? <b>{t("Not")}</b> : TAG[s.condition.type] ? <b>{t(TAG[s.condition.type]!)}</b> : null}
                          <Highlight text={pillLabel(s.condition)} query={query} />
                        </span>
                        {s.count >= 0 && <small className={s.count ? "" : "zero"}>{s.count}</small>}
                      </>
                    )}
                    {s.kind === "course" && (
                      <>
                        <span>
                          {selected.has(s.course.id) && <i className="finderCheck">✓</i>}
                          <Highlight text={pick(s.course.title, locale)} query={query} />
                          <em>
                            <Highlight text={s.course.code} query={query} /> · {schedule(s.course)} · {s.course.quarters.join(" ")}
                          </em>
                        </span>
                        <small>{onTimetable.has(s.course.code) ? t("On timetable") : selected.has(s.course.id) ? t("Selected") : t("Enter to select")}</small>
                      </>
                    )}
                    {s.kind === "text" && (
                      <>
                        <span>{s.key.startsWith("!") ? `${t("Not")} “${query}”` : `${t("Search the syllabus for")} “${query}”`}</span>
                        {s.count >= 0 && <small className={s.count ? "" : "zero"}>{s.count}</small>}
                      </>
                    )}
                  </div>
                </div>
              );
            })}
            <div className="finderKeys">
              <span><kbd>↑↓</kbd> {t("choose")}</span><span><kbd>↵</kbd> {t("apply")}</span><span><kbd>Tab</kbd> {t("complete")}</span><span><kbd>!</kbd> {t("exclude")}</span>
            </div>
          </div>
        )}
      </div>

      <div className="finderStatus">
        <span>
          {rows.length} {t("courses")} {filteredByText}
          {duplicates > 0 && ` · ${t("duplicates merged")}`}
        </span>
        {remote.state === "done" && (
          <button type="button" className="textButton" onClick={() => setRefresh((n) => n + 1)}>
            {t("Refresh")}
          </button>
        )}
        {remote.state === "done" && remote.truncated && (
          <span className="finderWarn">
            {t("The syllabus returns at most 100 of {{total}} — narrow with a code, 番台 or word.", { total: remote.total })}
          </span>
        )}
        {error && <span className="finderError">{error}</span>}
      </div>

      <div className="finderResults">
        {!courses && !error && <div className="empty">{t("Loading…")}</div>}
        {courses && rows.length === 0 && (
          <div className="empty finderEmpty">
            {remote.state === "unsupported" ? (
              <span>
                {remote.why === "too-many"
                  ? t("Unsupported search: the syllabus takes at most 4 department × code combinations. Narrow it down.")
                  : t("Unsupported search: the syllabus needs a department, a code or a word. Add one or change the conditions.")}
              </span>
            ) : remote.state === "searching" ? (
              <span>{t("Searching the syllabus…")}</span>
            ) : details ? (
              <span>{t("Loading schedules from the syllabus…")}</span>
            ) : remote.state === "done" && remote.total === 0 ? (
              <span>{t("The syllabus found nothing. Change or remove a condition.")}</span>
            ) : (
              <span>{t("No matching courses. Remove a condition to widen the search.")}</span>
            )}
          </div>
        )}
        {shown.map(({ course, matched }) => {
          const placed = onTimetable.has(course.code);
          const rec = context.recommended.get(course.code);
          const clash = !placed && clashes(course, context.busy);
          const replacing = clash && Boolean(replaces?.(course));
          return (
            <div
              key={course.id}
              className={`finderRow${placed ? " placed" : ""}${activeId === course.id ? " active" : ""}`}
            >
              <input
                type="checkbox"
                id={`finder-${course.id}`}
                aria-label={t("Select")}
                checked={selected.has(course.id)}
                disabled={placed || readOnly}
                onChange={() => toggle(course)}
              />
              <button type="button" className="finderMain" onClick={() => openCourse?.(course)}>
                <b><Highlight text={pick(course.title, locale)} query={live} /></b>
                <small>
                  <code>{course.code}</code> · {schedule(course)} · {course.quarters.join(" ")}
                  {course.detailed !== false && ` · ${course.credits.lecture}-${course.credits.exercise}-${course.credits.lab}`}
                  {matched.length > 0 && <em> · {matched.join(" / ")}</em>}
                </small>
                <span className="finderBadges">
                  {rec && <i className="rec">{rec.group}{rec.mark === "required" ? " ◎" : rec.mark ? " ○" : " Guide"}</i>}
                  {clash && <i className="clash">{t("Conflict")}</i>}
                </span>
              </button>
              {placed ? (
                <i className="finderPlaced">{t("Added")}</i>
              ) : (
                <UiButton
                  type="button"
                  className={`finderAdd${replacing ? " replace" : ""}`}
                  disabled={readOnly}
                  title={replacing ? t("Replaces the course in the same periods") : undefined}
                  onClick={() => addOne(course)}
                >
                  {replacing ? t("Replace") : t("Add")}
                </UiButton>
              )}
            </div>
          );
        })}
      </div>

      {pages > 1 && (
        <nav className="finderPager" aria-label={t("Pages")}>
          <button type="button" className="textButton" disabled={page === 0} onClick={() => setPage(page - 1)}>‹ {t("Prev")}</button>
          <span>{page + 1} / {pages}</span>
          <button type="button" className="textButton" disabled={page >= pages - 1} onClick={() => setPage(page + 1)}>{t("Next")} ›</button>
        </nav>
      )}

      <footer className="finderFoot">
        {lastAdded ? (
          <span>
            {t("Added {{count}}", { count: lastAdded.length })} ·{" "}
            <button type="button" className="textButton" onClick={() => { undo(lastAdded); setLastAdded(null); }}>
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
        <UiButton type="button" className="button" disabled={!chosen.length} onClick={() => setSelected(new Set())}>
          {t("Clear")}
        </UiButton>
        <UiButton type="button" className="button primary" disabled={!chosen.length} onClick={addSelected}>
          {chosen.length ? t("Add {{count}}", { count: chosen.length }) : t("Add")}
        </UiButton>
      </footer>
    </Paper>
  );
}
