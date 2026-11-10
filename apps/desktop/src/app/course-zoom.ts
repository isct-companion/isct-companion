/**
 * Opening a course from a timetable block or a Today row morphs that element
 * into the course page's title (slotledger approach/motion-course-zoom). The
 * View Transition that react-router already starts pairs the two by one
 * shared `view-transition-name`; the pairing lives only from the click until
 * the next page change away from the course has played, so back reverses it
 * and nothing else ever morphs. Hidden kept pages are not captured, so the
 * source and the title never count as duplicates.
 */
const NAME = "course-zoom";
let source: HTMLElement | null = null;

export function zoomFrom(element: HTMLElement) {
  clearZoom();
  if (!("startViewTransition" in document)) return;
  source = element;
  element.style.viewTransitionName = NAME;
  document.documentElement.dataset.courseZoom = "";
}

export function clearZoom() {
  if (source) source.style.viewTransitionName = "";
  source = null;
  delete document.documentElement.dataset.courseZoom;
}
