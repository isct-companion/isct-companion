import { SearchSelect } from "../../components/SearchSelect";
import { t } from "../../i18n";
import { useEffect, useRef, useState } from "react";
import type { SyllabusCourse } from "../../core/syllabus";
import type { SyllabusFilters } from "../../core/syllabus-filters";
import type { RecommendedCourse } from "../../core/model";
import departmentOptions from "../../data/syllabus-departments.json";
import { Text } from "@mantine/core";

type Response = {
  items: SyllabusCourse[];
  total: number;
  truncated: boolean;
};

const schools: Record<string, string> = {
  "1": "Science",
  "2": "Engineering",
  "3": "Materials",
  "4": "Computing",
  "5": "Life science",
  "6": "Environment",
  "7": "Liberal arts",
};
const departments = departmentOptions.map((d) => ({
  ...d,
  label: `${d.label} · ${schools[d.value.split("/")[0]] ?? "Program"}`,
}));

export function CourseCatalog({
  year,
  term,
  locale = "en",
  available,
  structuredAvailable = true,
  planned,
  search,
  hydrate,
  add,
  remove,
  close,
  manual,
  departmentScope,
  recommendations = [],
  syncDepartment,
}: {
  year: number;
  term: string;
  locale?: "en" | "ja" | "zh";
  available: boolean;
  structuredAvailable?: boolean;
  planned: ReadonlySet<string>;
  search: (
    query: string,
    year: number,
    filters?: SyllabusFilters,
  ) => Promise<Response>;
  hydrate: (url: string) => Promise<SyllabusCourse>;
  add: (course: SyllabusCourse) => void;
  remove: (externalId: string) => void;
  close: () => void;
  manual: () => void;
  departmentScope?: string;
  recommendations?: RecommendedCourse[];
  syncDepartment?: () => Promise<{ listed: number; synced: number; warnings?: string[] }>;
}) {
  const [selectedQuarter, setQuarter] = useState(
    term.split("-")[1] as SyllabusCourse["quarter"],
  );
  useEffect(() => {
    setQuarter(term.split("-")[1] as SyllabusCourse["quarter"]);
    setYear(year);
  }, [term, year]);
  const [query, setQuery] = useState("");
  const [selectedYear, setYear] = useState(year);
  const [departmentOnly, setDepartmentOnly] = useState(Boolean(departmentScope));
  const [filters, setFilters] = useState<SyllabusFilters>(() => ({
    department: departmentScope,
  }));
  const [results, setResults] = useState<SyllabusCourse[]>([]);
  const [origin, setOrigin] = useState<"saved" | "official">("saved");
  const [state, setState] = useState<"idle" | "loading" | "ready" | "error">(
    "idle",
  );
  const [composing, setComposing] = useState(false);
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const [rowError, setRowError] = useState<string>();
  const [total, setTotal] = useState(results.length);
  const [truncated, setTruncated] = useState(false);
  const [syncStatus, setSyncStatus] = useState("");
  const request = useRef(0),
    mounted = useRef(true);
  const searchRef = useRef(search),
    hydrateRef = useRef(hydrate);
  searchRef.current = search;
  hydrateRef.current = hydrate;
  const active = Boolean(
    query.trim() ||
    filters.name ||
    filters.teacher ||
    filters.code ||
    filters.department ||
    filters.groups !== undefined ||
    selectedYear !== year ||
    selectedQuarter !== term.split("-")[1],
  );
  const suggestions = results;
  useEffect(() => {
    if (composing) return;
    const generation = ++request.current;
    if (!available) {
      setResults([]);
      setTotal(0);
      setState("idle");
      return;
    }
    setState("loading");
    const transportFilters = !active
      ? filters
      : !query.trim() &&
          !filters.teacher &&
          !filters.name &&
          !filters.code &&
          !filters.department &&
          filters.groups === undefined
        ? { ...filters, groups: ["100", "200", "300", "400", "500", "600"] }
        : filters;
    const timer = setTimeout(
      () =>
        searchRef
          .current(query.trim(), selectedYear, transportFilters)
          .then((response) => {
            if (generation !== request.current) return;
            setResults(response.items);
            setOrigin(active ? "official" : "saved");
            setTotal(response.total);
            setTruncated(response.truncated);
            setState("ready");
          })
          .catch(() => {
            if (generation === request.current) setState("error");
          }),
      280,
    );
    return () => {
      clearTimeout(timer);
      request.current++;
    };
  }, [
    query,
    selectedYear,
    selectedQuarter,
    filters,
    available,
    composing,
    term,
    locale,
  ]);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      request.current++;
    };
  }, []);
  const update = (name: keyof SyllabusFilters, value: string) =>
    setFilters((f) => ({ ...f, [name]: value || undefined }));
  const desired = selectedQuarter;
  const recommendation = (code: string) =>
    recommendations.find((item) => item.code === code && !item.deletedAt);
  const shownResults = results.filter(
    (c) =>
      c.year === selectedYear &&
      (c.quarter === selectedQuarter || c.quarters?.includes(selectedQuarter)),
  ).sort((left, right) =>
    Number(Boolean(recommendation(right.code))) - Number(Boolean(recommendation(left.code))) ||
    left.code.localeCompare(right.code),
  );
  async function choose(course: SyllabusCourse) {
    const key = `${course.externalId}:${course.year}`;
    if (busy.has(key)) return;
    setBusy((s) => new Set(s).add(key));
    setRowError(undefined);
    try {
      let detail = await hydrateRef.current(course.url);
      if (!mounted.current) return;
      if (
        detail.externalId !== course.externalId ||
        detail.year !== course.year
      )
        throw new Error("Course identity mismatch");
      if (detail.quarter !== desired && !detail.quarters?.includes(desired))
        throw new Error("Course quarter mismatch");
      if (detail.quarters?.includes(desired))
        detail = { ...detail, quarter: desired };
      add(detail);
    } catch {
      if (mounted.current) setRowError(key);
    } finally {
      if (mounted.current)
        setBusy((s) => {
          const n = new Set(s);
          n.delete(key);
          return n;
        });
    }
  }
  const completion = (field: "title" | "code" | "instructor") =>
    [...new Set(suggestions.map((c) => c[field]).filter(Boolean))].slice(
      0,
      100,
    );
  return (
    <section className="catalogDrawer coursePlanner" aria-label={t("Course planner")}>
      <header className="coursePlannerHeader">
        <div>
          <Text fw={700}>{t("Add courses")}</Text>
          <Text c="dimmed" size="xs">
            {t("Syllabus → draft timetable")}
          </Text>
        </div>
        <UiButton className="textButton" onClick={close}>{t("Done")}</UiButton>
      </header>
        <div className="catalogControls">
          {departmentScope && (
            <div className="catalogDepartmentToggle">
              <label>
                <UiInput
                  type="checkbox"
                  checked={departmentOnly}
                  onChange={(event) => {
                    setDepartmentOnly(event.currentTarget.checked);
                    setFilters((current) => ({
                      ...current,
                      department: event.currentTarget.checked
                        ? departmentScope
                        : undefined,
                    }));
                  }}
                />
                <span>{t("My department")}</span>
              </label>
              {syncDepartment && (
                <UiButton
                  className="textButton"
                  disabled={syncStatus === "Syncing catalog…"}
                  onClick={() => {
                    setSyncStatus("Syncing catalog…");
                    void syncDepartment()
                      .then((result) =>
                        setSyncStatus(
                          `${result.synced}/${result.listed} offerings refreshed`,
                        ),
                      )
                      .catch(() => setSyncStatus("Catalog sync failed; saved results remain"));
                  }}
                >
                  Sync catalog
                </UiButton>
              )}
              {syncStatus && <small role="status">{syncStatus}</small>}
            </div>
          )}
          <label className="catalogKeyword">
            <span>{t("Keyword")}</span>
            <UiInput
              autoFocus
              type="search"
              value={query}
              placeholder={t("Search courses")}
              autoComplete="off"
              onCompositionStart={() => setComposing(true)}
              onCompositionEnd={(event) => {
                setComposing(false);
                setQuery(event.currentTarget.value);
              }}
              onChange={(event) => setQuery(event.target.value)}
              aria-controls="syllabus-results"
            />
          </label>
          <details className="catalogAdvanced">
            <summary>{t("More filters")}</summary>
            <div className="catalogFilterGrid">
              <label>
                <span>{t("Academic year")}</span>
                <UiSelect
                  disabled={!structuredAvailable}
                  value={selectedYear}
                  onChange={(e) => {
                    setYear(Number(e.target.value));
                    update("department", "");
                  }}
                >
                  {Array.from({ length: 7 }, (_, i) => 2026 - i).map((y) => (
                    <option key={y}>{y}</option>
                  ))}
                </UiSelect>
              </label>
              <label>
                <span>{t("Quarter")}</span>
                <UiSelect
                  aria-label={t("Quarter")}
                  value={selectedQuarter}
                  onChange={(e) =>
                    setQuarter(e.target.value as SyllabusCourse["quarter"])
                  }
                >
                  {["1Q", "2Q", "3Q", "4Q"].map((q) => (
                    <option key={q}>{q}</option>
                  ))}
                </UiSelect>
              </label>
              <SearchSelect
                label={t("Department / program")}
                disabled={!structuredAvailable || selectedYear !== 2026}
                value={filters.department ?? ""}
                options={departments}
                placeholder={t("All departments")}
                onChange={(value) => update("department", value)}
              />
              {(["name", "code", "teacher"] as const).map((field, i) => (
                <label
                  key={field}
                  className={field === "teacher" ? "catalogTeacher" : undefined}
                >
                  <span>
                    {t(["Course title", "Course code", "Instructor"][i])}
                  </span>
                  <UiInput
                    disabled={!structuredAvailable}
                    list={field === "code" ? undefined : `syllabus-${field}`}
                    autoComplete="off"
                    value={filters[field] ?? ""}
                    placeholder={t(
                      ["Any title", "e.g. CSC.T354", "Any instructor"][i],
                    )}
                    onCompositionStart={() => setComposing(true)}
                    onCompositionEnd={() => setComposing(false)}
                    onChange={(e) => update(field, e.target.value)}
                  />
                  {field !== "code" && (
                    <datalist id={`syllabus-${field}`}>
                      {completion(
                        field === "name"
                          ? "title"
                          : field === "teacher"
                            ? "instructor"
                            : "code",
                      ).map((v) => (
                        <option key={v} value={v} />
                      ))}
                    </datalist>
                  )}
                </label>
              ))}
            </div>
            <fieldset className="catalogSeries" disabled={!structuredAvailable}>
              <legend>{t("Series")}</legend>
              {["100", "200", "300", "400", "500", "600"].map((value) => (
                <label key={value}>
                  <UiInput
                    type="checkbox"
                    checked={filters.groups?.includes(value) ?? true}
                    onChange={(e) =>
                      setFilters((f) => ({
                        ...f,
                        groups: e.target.checked
                          ? [
                              ...(f.groups ?? [
                                "100",
                                "200",
                                "300",
                                "400",
                                "500",
                                "600",
                              ]),
                              value,
                            ]
                          : (
                              f.groups ?? [
                                "100",
                                "200",
                                "300",
                                "400",
                                "500",
                                "600",
                              ]
                            ).filter((g) => g !== value),
                      }))
                    }
                  />
                  <span>{value}</span>
                </label>
              ))}
            </fieldset>
          </details>
          <div className="catalogStatus">
            <span role="status">
              {t(
                state === "loading"
                  ? "Searching syllabus…"
                  : state === "error"
                    ? "Showing saved results"
                    : origin === "saved"
                      ? "Saved results"
                      : "Official syllabus results",
              )}
            </span>
            {active && (
              <UiButton
                className="textButton"
                onClick={() => {
                  setQuery("");
                  setFilters({});
                  setYear(year);
                  setQuarter(term.split("-")[1] as SyllabusCourse["quarter"]);
                }}
              >
                {t("Reset")}
              </UiButton>
            )}
          </div>
        </div>
        <div className="catalogCount">
          {t("{{count}} results", {
            count: shownResults.length,
          })}
          {truncated && <span> · {t("Narrow filters to see more")}</span>}
        </div>
        <div
          className="catalogResults"
          id="syllabus-results"
          role="list"
          aria-label={t("Syllabus results")}
        >
          {shownResults.map((course) => {
            const recommended = recommendation(course.code);
            const targetQuarter = course.quarters?.includes(desired)
              ? desired
              : course.quarter;
            const sourceKey = `${course.externalId}:${course.year}-${targetQuarter}`;
            const key = `${course.externalId}:${course.year}`;
            const added = planned.has(sourceKey);
            const wrongTerm = `${course.year}-${targetQuarter}` !== term;
            return (
              <article className="catalogResult" role="listitem" key={key}>
                <div>
                  <b>{course.title}</b>
                  {recommended && (
                    <strong className="recommendationBadge">
                      {recommended.group} {recommended.mark === "required" ? "◎" : "○"}
                    </strong>
                  )}
                  <p>
                    {course.code} ·{" "}
                    {course.instructor || t("Instructor not listed")}
                  </p>
                  <span>
                    {course.year} {course.quarters?.join("–") ?? course.quarter}
                    {course.meetings?.length
                      ? ` · ${course.meetings.map((m) => `${["", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][m.days[0]]} ${m.startTime}`).join(" / ")}`
                      : ` · ${t(course.content ? "Schedule not published" : "Schedule on selection")}`}
                  </span>
                  {rowError === key && (
                    <small>{t("Could not load details. Retry.")}</small>
                  )}
                </div>
                {added ? (
                  <UiButton
                    className="button compactButton catalogUndo"
                    onClick={() => remove(sourceKey)}
                  >
                    {t("Remove")}
                  </UiButton>
                ) : (
                  <UiButton
                    className="button primary compactButton"
                    disabled={busy.has(key)}
                    onClick={() => void choose(course)}
                  >
                    {t(
                      busy.has(key)
                        ? "Loading…"
                        : rowError === key
                          ? "Retry"
                          : wrongTerm
                            ? t("Add to {{quarter}}", {
                                quarter: targetQuarter,
                              })
                            : "Add",
                    )}
                  </UiButton>
                )}
              </article>
            );
          })}
          {!shownResults.length && (
            <div className="catalogNotice">
              {t(
                state === "loading"
                  ? "Looking up matching courses…"
                  : "No matching courses. Try fewer filters.",
              )}
            </div>
          )}
        </div>
        <footer>
          <span
            title={t(
              "Draft planning only. This does not register a course with the school.",
            )}
          >
            {t("{{count}} added in {{quarter}}", {
              count: [...planned].filter((key) =>
                key.endsWith(`:${selectedYear}-${selectedQuarter}`),
              ).length,
              quarter: selectedQuarter,
            })}
          </span>
          <UiButton className="textButton" onClick={manual}>
            {t("Manual entry")}
          </UiButton>
        </footer>
    </section>
  );
}
import { UiButton, UiInput, UiSelect } from "../../components/mantine";
