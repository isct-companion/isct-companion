import { useEffect, useState } from "react";
import {
  useTimetableViews,
  type WorkspaceOverview,
} from "../../platform/core-query";
import type { TermId } from "../../lib/model";

/** The quarter the planner shows and the workspace view the shell renders
 * for it: a previewed quarter reads its cached timetable while the planner
 * is active. The shell owns it because the loading gate and Today read the
 * same view. */
export function useCoursePlanningTerm(
  overview: WorkspaceOverview | undefined,
  active: boolean,
) {
  const [previewTerm, setPreviewTerm] = useState<TermId | null>(null);
  const courseViewTerm = previewTerm ?? overview?.coursesTerm;
  const timetableViews = useTimetableViews(overview?.allTerms ?? [], active);
  const cachedTimetable = courseViewTerm
    ? timetableViews.get(courseViewTerm)
    : undefined;
  useEffect(() => {
    if (previewTerm === overview?.coursesTerm) setPreviewTerm(null);
  }, [previewTerm, overview?.coursesTerm]);

  const workspaceView = active
    ? (cachedTimetable?.overview ?? overview)
    : overview;
  return { courseViewTerm, cachedTimetable, workspaceView, setPreviewTerm };
}
