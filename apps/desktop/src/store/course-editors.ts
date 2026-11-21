import { create } from "zustand";
import type { Course, PersonalEvent, TermId } from "../lib/model";

/** Which course-planning editor is open. The shell keeps `CourseEditors`
 * mounted beside its retained pages, so an editor opened from Today shows
 * while the planner is hidden, and closing a page does not close it. */
type CourseEditorsState = {
  course: { course?: Course; term: TermId } | null;
  event: { event?: PersonalEvent; term: TermId } | null;
  intensiveCourseId: string | null;
  addingTerm: boolean;
};

export const useCourseEditors = create<CourseEditorsState>(() => ({
  course: null,
  event: null,
  intensiveCourseId: null,
  addingTerm: false,
}));

const set = useCourseEditors.setState;

export const courseEditors = {
  editCourse: (course: CourseEditorsState["course"]) => set({ course }),
  editEvent: (event: CourseEditorsState["event"]) => set({ event }),
  editIntensive: (intensiveCourseId: string | null) =>
    set({ intensiveCourseId }),
  addTerm: (addingTerm: boolean) => set({ addingTerm }),
};
