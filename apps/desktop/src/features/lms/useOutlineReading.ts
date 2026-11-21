import {
  loadReading,
  persistReading,
  type ReadingState,
  type OpenedContent,
} from "./reading-state";
import { useLayoutEffect, useEffect, useMemo, useRef, useState } from "react";
import type { Assignment } from "../../lib/model";
import {
  gravityTarget,
  springScroll,
  type SpringScroll,
} from "./spring-scroll";
import type { Dispatch, SetStateAction } from "react";
import type { LessonContent } from "../../generated/persistence";

/**
 * A section as Core lays it out (`courseContent`), keyed by its LMS section
 * id (its number in a course without an LMS record) and named by its title.
 */
export type Section = Omit<LessonContent, "title"> & { id: number; title: string };

/** The rail's entry for course activities outside any section. */
export const ACTIVITIES = -1;

export function useOutlineReadingState(key: string) {
  const [saved] = useState(() => loadReading(key));
  const [expanded, setExpanded] = useState<string[]>(saved?.expanded ?? []);
  const [readingSection, setReadingSection] = useState(saved?.anchor?.id);
  const [query, setQuery] = useState("");
  const [focusLesson, setFocusLesson] = useState<number | undefined>(
    saved?.lesson,
  );
  const outline = useRef<HTMLDivElement>(null);
  return {
    saved,
    expanded,
    setExpanded,
    readingSection,
    setReadingSection,
    query,
    setQuery,
    focusLesson,
    setFocusLesson,
    outline,
  };
}

export function useOutlineReading(
  state: ReturnType<typeof useOutlineReadingState>,
  sections: Section[],
  lessons: Section[],
  at: Section | undefined,
  opened: OpenedContent | undefined,
) {
  const {
    saved,
    expanded,
    query,
    focusLesson,
    setFocusLesson,
    setReadingSection,
    outline,
  } = state;
  const current = useRef<ReadingState>({
    version: 3,
    expanded,
    opened,
    scroll: saved?.scroll ?? 0,
    anchor: saved?.anchor,
  });
  current.current = {
    ...current.current,
    expanded,
    opened,
    lesson: focusLesson,
  };
  // The whole course is laid out at once; lessons behave as pages of it.
  // Sections without a number (一般, 連絡先 …) sit in the flow and are
  // reached from the rail, like Activities outside any section.
  const pinned = useMemo(
    () => sections.filter((s) => !lessons.some((l) => l.id === s.id)),
    [sections, lessons],
  );
  const paged = lessons.length > 0 && !query.trim();
  // The page being read: set by arriving, the rail, a pull, or scrolling.
  const [page, setPage] = useState<number | undefined>(focusLesson);
  const shownId =
    page === ACTIVITIES || sections.some((s) => s.id === page) ? page : at?.id;
  const lessonId = lessons.some((l) => l.id === shownId) ? shownId : undefined;
  const pageElement = (id: number | undefined) =>
    id == null
      ? null
      : (outline.current?.querySelector<HTMLElement>(
          id === ACTIVITIES
            ? '[data-page="activities"]'
            : `[data-page="${id}"]`,
        ) ?? null);
  const goLesson = (id: number) => {
    const host = outline.current;
    const target = pageElement(id);
    setPage(id);
    setFocusLesson(id);
    if (!host || !target) return;
    // The page follows the scroll again once this scroll has arrived.
    const state = pull.current;
    state.turning = target.offsetTop;
    state.motion?.cancel();
    // Only where the spring arrives counts as placed: a jump the person
    // interrupts rests where they stopped it, and gravity pulls as usual.
    state.placed = Math.min(
      target.offsetTop,
      Math.max(0, host.scrollHeight - host.clientHeight),
    );
    state.motion = springScroll(host, target.offsetTop, {
      onDone: () => {
        state.turning = null;
        state.motion = null;
      },
    });
  };
  // Sections attract like gravity wells. Scrolling itself stays native, with
  // its own speed and momentum, so a fast flick can fly over several
  // sections. Where it comes to rest decides: inside a section (the middle
  // of the outline) nothing pulls; with a section boundary near an edge of
  // the outline, the nearer section draws the outline to it, the new one by
  // bringing its start to the top, the old one by bringing its end back to
  // the bottom.
  const pull = useRef({
    turning: null as number | null,
    rest: 0,
    motion: null as SpringScroll | null,
    placed: null as number | null,
  });
  // Where the app itself puts the outline (arriving, revealing an assignment,
  // restoring the reading position) is where the person asked to be, so
  // gravity pulls only an outline the person scrolled. Every programmatic
  // placement of the outline goes through here.
  const placeOutline = (apply: (host: HTMLDivElement) => void) => {
    const host = outline.current;
    if (!host) return;
    const state = pull.current;
    state.motion?.cancel();
    clearTimeout(state.rest);
    apply(host);
    state.placed = host.scrollTop;
  };
  // An assignment already in full view stays where it is, so asking for it
  // again never moves the outline.
  const revealAssignment = (id: string) =>
    placeOutline((host) => {
      const card = host.querySelector<HTMLElement>(
        `[data-assignment="${CSS.escape(id)}"]`,
      );
      if (!card) return;
      const view = host.getBoundingClientRect();
      const box = card.getBoundingClientRect();
      if (box.height > 0 && box.top >= view.top && box.bottom <= view.bottom)
        return;
      card.scrollIntoView?.({
        block: box.height > view.height ? "start" : "center",
      });
    });
  const pageRef = useRef(shownId);
  pageRef.current = shownId;
  const settleInto = (host: HTMLDivElement) => {
    const state = pull.current;
    if (!paged || state.turning != null) return;
    if (state.placed != null && Math.abs(host.scrollTop - state.placed) < 1)
      return;
    state.placed = null;
    const boundaries = [...host.querySelectorAll<HTMLElement>("[data-page]")]
      .slice(1)
      .map((element) => element.offsetTop);
    const target = gravityTarget(
      host.scrollTop,
      host.clientHeight,
      boundaries,
      host.scrollHeight,
    );
    if (target == null) return;
    state.motion?.cancel();
    state.motion = springScroll(host, target, {
      onDone: () => (state.motion = null),
    });
  };
  // Scrolling has come to rest once its events stop (momentum included).
  const restAfterScroll = (host: HTMLDivElement) => {
    const state = pull.current;
    // The app's own spring scrolls do not count as coming to rest.
    if (state.motion) return;
    clearTimeout(state.rest);
    state.rest = window.setTimeout(() => settleInto(host), 90);
  };
  // Scrolling keeps the page in step with what is read:
  // the last page whose top has reached the upper third of the outline.
  const followScroll = (host: HTMLDivElement) => {
    const state = pull.current;
    if (state.turning != null) {
      if (Math.abs(host.scrollTop - state.turning) > 2) return;
      state.turning = null;
    }
    if (!paged) return;
    const line = host.scrollTop + host.clientHeight / 3;
    let found: number | undefined;
    for (const element of host.querySelectorAll<HTMLElement>("[data-page]")) {
      if (element.offsetTop > line) break;
      const key = element.dataset.page!;
      found = key === "activities" ? ACTIVITIES : Number(key);
    }
    if (found != null && found !== pageRef.current) setPage(found);
  };
  // One accent bar slides along the rail to the page being read.
  const rail = useRef<HTMLElement>(null);
  const railIndicator = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    const nav = rail.current;
    const bar = railIndicator.current;
    if (!nav || !bar) return;
    const place = () => {
      const current = nav.querySelector<HTMLElement>("button.current");
      bar.style.opacity = current ? "1" : "0";
      if (!current) return;
      bar.style.transform = `translateY(${current.offsetTop + 4}px)`;
      bar.style.height = `${Math.max(0, current.offsetHeight - 8)}px`;
      // Keep the current entry in view on a rail that scrolls down.
      if (
        current.offsetTop < nav.scrollTop ||
        current.offsetTop + current.offsetHeight >
          nav.scrollTop + nav.clientHeight
      )
        nav.scrollTo?.({
          top: current.offsetTop - nav.clientHeight / 2,
          behavior: "smooth",
        });
    };
    place();
    const resize =
      typeof ResizeObserver === "function"
        ? new ResizeObserver(place)
        : undefined;
    resize?.observe(nav);
    return () => resize?.disconnect();
  }, [shownId, paged, lessons, pinned.length]);
  const recordPosition = (host: HTMLDivElement) => {
    current.current.scroll = host.scrollTop;
    if (query) return;
    const viewport = host.getBoundingClientRect();
    if (viewport.height <= 0) return;
    const center = viewport.top + viewport.height / 2;
    let closest:
      { el: HTMLElement; rect: DOMRect; distance: number } | undefined;
    // Scan once without allocating and sorting a second array on every scroll.
    for (const el of host.querySelectorAll<HTMLElement>(
      "[data-reading-section]",
    )) {
      const rect = el.getBoundingClientRect();
      if (rect.height <= 0) continue;
      const distance =
        center < rect.top
          ? rect.top - center
          : center > rect.bottom
            ? center - rect.bottom
            : 0;
      if (!closest || distance < closest.distance)
        closest = { el, rect, distance };
      if (distance === 0) break;
    }
    if (!closest || !closest.rect.height) return;
    const id = closest.el.dataset.readingSection!;
    current.current.anchor = {
      id,
      ratio: Math.max(
        0,
        Math.min(1, (center - closest.rect.top) / closest.rect.height),
      ),
    };
    setReadingSection(id);
  };

  return {
    pinned,
    paged,
    shownId,
    lessonId,
    rail,
    railIndicator,
    goLesson,
    revealAssignment,
    recordPosition,
    followScroll,
    restAfterScroll,
    current,
    setPage,
    placeOutline,
  };
}

export function useOutlineEntrance({
  key,
  deviceId,
  state,
  reading,
  contentRead,
  entry,
  assignments,
  sections,
  lessons,
  at,
  opened,
  setOpenDetails,
  setViewing,
}: {
  key: string;
  deviceId: string;
  state: ReturnType<typeof useOutlineReadingState>;
  reading: ReturnType<typeof useOutlineReading>;
  contentRead: boolean;
  entry: { request: number; assignmentId?: string };
  assignments: Assignment[];
  sections: Section[];
  lessons: Section[];
  at: Section | undefined;
  opened: OpenedContent | undefined;
  setOpenDetails: Dispatch<SetStateAction<string[]>>;
  setViewing: Dispatch<SetStateAction<boolean>>;
}) {
  const { outline, setExpanded, setFocusLesson } = state;
  const { current, setPage, placeOutline, revealAssignment } = reading;
  const positionedRequest = useRef<number | undefined>(undefined);
  useEffect(() => {
    // Revealing Activity resumes the retained DOM; only a new entrance repositions it.
    if (positionedRequest.current === entry.request || !contentRead) return;
    const assignment = assignments.find((a) => a.id === entry.assignmentId);
    if (assignment) {
      const section = sections.find((s) =>
        s.assignmentIds.includes(assignment.id),
      );
      if (section) {
        const key = `section:${section.id}`;
        setExpanded((previous) =>
          previous.includes(key)
            ? previous.filter((id) => id !== key)
            : previous,
        );
        if (lessons.some((l) => l.id === section.id)) {
          setFocusLesson(section.id);
          setPage(section.id);
        }
      }
      // Arriving at one assignment shows its requirements at once.
      setOpenDetails((list) =>
        list.includes(assignment.id) ? list : [...list, assignment.id],
      );
      setViewing(false);
    }
    // No date→section association exists in the current importer. Resume the
    // real reading position rather than guessing the lesson from the week.
    const frame = requestAnimationFrame(() => {
      positionedRequest.current = entry.request;
      if (assignment) return revealAssignment(assignment.id);
      placeOutline((host) => {
        const anchor = current.current.anchor;
        const element =
          anchor &&
          [
            ...host.querySelectorAll<HTMLElement>("[data-reading-section]"),
          ].find((el) => el.dataset.readingSection === anchor.id);
        if (element && anchor) {
          const panel = host.getBoundingClientRect();
          const rect = element.getBoundingClientRect();
          host.scrollTop +=
            rect.top -
            panel.top +
            rect.height * anchor.ratio -
            panel.height / 2;
        } else if (lessons.length && at) {
          // Arriving at a lesson course opens the page the course is at.
          const page = host.querySelector<HTMLElement>(
            `[data-page="${at.id}"]`,
          );
          host.scrollTop = page?.offsetTop ?? 0;
          setPage(at.id);
        } else host.scrollTop = current.current.scroll;
      });
    });
    return () => cancelAnimationFrame(frame);
    // Explicit entrance only; refreshing content must never jump the reader.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [entry.request, contentRead]);
  const saveReading = () => persistReading(key, deviceId, current.current);
  useEffect(() => {
    window.addEventListener("pagehide", saveReading);
    return () => {
      window.removeEventListener("pagehide", saveReading);
      saveReading();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  useLayoutEffect(() => {
    const host = outline.current;
    const anchor = current.current.anchor;
    if (!host || !anchor) return;
    const element = [
      ...host.querySelectorAll<HTMLElement>("[data-reading-section]"),
    ].find((el) => el.dataset.readingSection === anchor.id);
    if (!element) return;
    const panel = host.getBoundingClientRect();
    const rect = element.getBoundingClientRect();
    if (panel.height > 0 && rect.height > 0)
      placeOutline(() => {
        host.scrollTop +=
          rect.top - panel.top + rect.height * anchor.ratio - panel.height / 2;
      });
  }, [Boolean(opened)]);
}
