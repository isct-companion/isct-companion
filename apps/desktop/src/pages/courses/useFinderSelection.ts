import { errorText } from "../../lib/error-text";
import { useState } from "react";
import { type FinderCourse } from "../../lib/course-query";
import type { FinderRowView } from "../../platform/finder-query";
import type { Finder } from "./useFinder";

export const unavailable = (row: FinderRowView) =>
  row.eligibility?.status === "ineligible";

/** An add or undo the row shows before Core confirms it. `after` is the
 * last request sent when it settled; a page sent later is authoritative. */
type Mark = { placed: boolean; saving: boolean; after: number };

export function useFinderSelection(
  finder: Finder,
  add: (courses: FinderCourse[]) => Promise<unknown>,
  undo: (courses: FinderCourse[]) => Promise<unknown>,
) {
  const [error, setError] = useState("");
  const [selected, setSelected] = useState<Map<string, FinderRowView>>(
    () => new Map(),
  );
  const [lastAdded, setLastAdded] = useState<FinderCourse[] | null>(null);
  const [marks, setMarks] = useState<Map<string, Mark>>(() => new Map());
  const [mutating, setMutating] = useState(false);
  const placed = (row: FinderRowView) => {
    const mark = marks.get(row.course.id);
    return mark && finder.shown <= mark.after ? mark.placed : row.placed;
  };
  const saving = (row: FinderRowView) =>
    Boolean(marks.get(row.course.id)?.saving);

  // A pick reads as the finder has it now (credits and clashes follow the
  // timetable); one that has left the results reads as it was picked.
  const chosen = [...selected.values()]
    .map(
      (row) =>
        finder.rows.find((current) => current.course.id === row.course.id) ??
        row,
    )
    .filter((row) => !unavailable(row) && !placed(row));
  const chosenCredits = chosen.reduce((sum, row) => sum + row.credits, 0);
  const chosenClashes = chosen.filter((row) => row.clash).length;
  const toggle = (row: FinderRowView) => {
    if (placed(row) || unavailable(row)) return;
    setSelected((current) => {
      const next = new Map(current);
      if (next.has(row.course.id)) next.delete(row.course.id);
      else next.set(row.course.id, row);
      return next;
    });
  };
  const mark = (courses: FinderCourse[], next: Mark | undefined) =>
    setMarks((current) => {
      const marked = new Map(current);
      for (const course of courses)
        if (next) marked.set(course.id, next);
        else marked.delete(course.id);
      return marked;
    });
  // The rows say "Added" at once; a failed write puts them back.
  const addCourses = async (courses: FinderCourse[]) => {
    if (mutating) return;
    setMutating(true);
    setError("");
    mark(courses, { placed: true, saving: true, after: Infinity });
    try {
      await add(courses);
      mark(courses, { placed: true, saving: false, after: finder.issued() });
      setLastAdded(courses);
      setSelected((current) => {
        const next = new Map(current);
        courses.forEach((course) => next.delete(course.id));
        return next;
      });
    } catch (reason) {
      mark(courses, undefined);
      setError(errorText(reason, { nativeMessage: false }) ?? String(reason));
    } finally {
      setMutating(false);
    }
  };
  const addSelected = () => {
    if (!chosen.length) return;
    void addCourses(chosen.map((row) => row.course));
  };
  const undoLast = async () => {
    if (!lastAdded || mutating) return;
    const courses = lastAdded;
    setMutating(true);
    setError("");
    mark(courses, { placed: false, saving: false, after: Infinity });
    try {
      await undo(courses);
      mark(courses, { placed: false, saving: false, after: finder.issued() });
      setLastAdded(null);
    } catch (reason) {
      mark(courses, { placed: true, saving: false, after: finder.issued() });
      setError(errorText(reason, { nativeMessage: false }) ?? String(reason));
    } finally {
      setMutating(false);
    }
  };

  const addPasted = (rows: FinderRowView[]) =>
    setSelected((current) => {
      const selection = new Map(current);
      for (const row of rows) selection.set(row.course.id, row);
      return selection;
    });

  const clearSelected = () => setSelected(new Map());

  return {
    error,
    setError,
    selected,
    addPasted,
    clearSelected,
    mutating,
    lastAdded,
    placed,
    saving,
    toggle,
    chosen,
    chosenCredits,
    chosenClashes,
    addCourses,
    addSelected,
    undoLast,
  };
}
