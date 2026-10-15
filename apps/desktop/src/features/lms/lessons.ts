/**
 * Lessons are the numbered sections of a course (第3回, Lecture 3, Week 3,
 * #3). The number is a leaf value read from the section title; sections
 * without one (一般, 連絡先・オフィスアワー) stay ordinary sections.
 */
const LESSON =
  /第\s*([0-9０-９]+)\s*回|(?:lecture|lesson|class|week|session|day)\s*#?\s*([0-9]+)\b|^\s*#?\s*([0-9]+)\s*(?:[.:：．、]|$)/i;

export function lessonNumber(title: string): number | undefined {
  const match = title.normalize("NFKC").match(LESSON);
  const digits = match?.[1] ?? match?.[2] ?? match?.[3];
  return digits ? Number(digits) : undefined;
}

export type Lesson = { id: number; number: number; posted: boolean };

/**
 * The course's lessons in section order. Fewer than two numbered sections is
 * not a lesson course; the outline then keeps its plain sections.
 */
export function courseLessons(
  sections: { id: number; title: string }[],
  posted: (sectionId: number) => boolean,
): Lesson[] {
  const lessons = sections.flatMap((section) => {
    const number = lessonNumber(section.title);
    return number == null
      ? []
      : [{ id: section.id, number, posted: posted(section.id) }];
  });
  return lessons.length >= 2 ? lessons : [];
}

/**
 * The lesson the course is at: the last one with posted material. Teachers
 * post a lesson's material around its class, so a cancelled class posts
 * nothing and does not advance it. Before anything is posted, the first.
 */
export function frontierLesson(lessons: Lesson[]): Lesson | undefined {
  return [...lessons].reverse().find((lesson) => lesson.posted) ?? lessons[0];
}

/**
 * A folded lesson's one-line summary: the first line of the section text,
 * without a short leading label such as "授業計画：" or "Course schedule:".
 */
export function lessonSummary(html?: string): string {
  if (!html) return "";
  const page = new DOMParser().parseFromString(html.slice(0, 20000), "text/html");
  for (const br of page.querySelectorAll("br")) br.replaceWith("\n");
  for (const block of page.querySelectorAll("p, div, li, tr, h1, h2, h3, h4, h5, h6"))
    block.append("\n");
  const line = (page.body.textContent ?? "")
    .split("\n")
    .map((text) => text.replace(/\s+/g, " ").trim())
    .find(Boolean);
  return (line ?? "").replace(/^[^：:]{1,16}[：:]\s*/, "");
}

/**
 * Marks the rows of lesson schedule tables against the lesson the course is
 * at: earlier rows `past`, its row `now`, cancelled rows `off`. A table
 * qualifies when at least two of its rows start a cell with a lesson number
 * (1回目, 第2回, 3).
 */
export function markSchedule(html: string, current?: number): string {
  if (current == null || !html.includes("<table")) return html;
  const page = new DOMParser().parseFromString(html, "text/html");
  let changed = false;
  for (const table of page.querySelectorAll("table")) {
    const rows = [...table.querySelectorAll("tr")];
    const numbered = rows.map((row) => {
      for (const cell of row.cells) {
        const match = cell.textContent
          ?.normalize("NFKC")
          .trim()
          .match(/^(?:第\s*)?(\d{1,2})\s*(?:回目?|$)/);
        if (match) return Number(match[1]);
      }
      return undefined;
    });
    if (numbered.filter((n) => n != null).length < 2) continue;
    const nowIndex = numbered.findIndex((n) => n === current);
    rows.forEach((row, index) => {
      const number = numbered[index];
      const cancelled = /休講|cancel(?:l)?ed|no class/i.test(row.textContent ?? "");
      const before = nowIndex >= 0 ? index < nowIndex : number != null && number < current;
      if (cancelled) row.classList.add("lmsRowOff");
      if (number === current) row.classList.add("lmsRowNow");
      else if (before && (number != null || cancelled)) row.classList.add("lmsRowPast");
    });
    changed = true;
  }
  return changed ? page.body.innerHTML : html;
}

/**
 * Rail labels: the lessons' own numbers when they count up by one, else their
 * order. Teachers' titles can skip or repeat a number (7, 9, 8, 11, 11).
 */
export function railLabels(lessons: Lesson[]): string[] {
  const counts = lessons.every(
    (lesson, index) => index === 0 || lesson.number === lessons[index - 1].number + 1,
  );
  return lessons.map((lesson, index) => String(counts ? lesson.number : index + 1));
}

export type LessonMark = { occurrence: string; sectionId: number };

/**
 * The lesson the person marked for the latest class. It stays the current
 * lesson until a later class starts (`lastStarted`, the start of the most
 * recent class that has begun); from then on the course is at the next one.
 */
export function markedLesson(
  lessons: Lesson[],
  log: LessonMark[] | null | undefined,
  lastStarted?: string,
): Lesson | undefined {
  const last = (log ?? [])
    .filter((mark) => lessons.some((lesson) => lesson.id === mark.sectionId))
    .sort((left, right) => left.occurrence.localeCompare(right.occurrence))
    .at(-1);
  if (!last) return undefined;
  const index = lessons.findIndex((lesson) => lesson.id === last.sectionId);
  return lastStarted && last.occurrence < lastStarted
    ? (lessons[index + 1] ?? lessons[index])
    : lessons[index];
}

/** Where sections pull: bands at the outline's edges, as fractions of its height. */
export const GRAVITY = { nearTop: 0.2, beyondTop: 0.15, nearBottom: 0.2 };

/**
 * Where a resting outline settles, given the tops of the sections after the
 * first (`boundaries`). Inside a section nothing pulls. With a boundary near
 * the top (or just above it) the new section brings its start to the top;
 * with one near the bottom the old section brings its end to the bottom. A
 * start near the top wins over any end near the bottom, so a section shorter
 * than the outline shows from its beginning; otherwise the nearer pull wins.
 * `undefined` when nothing pulls.
 */
export function gravityTarget(
  top: number,
  height: number,
  boundaries: number[],
  scrollHeight = Infinity,
): number | undefined {
  const nearest = (pairs: [number, number][]) =>
    pairs.sort((left, right) => left[0] - right[0])[0]?.[1];
  const starts: [number, number][] = [];
  const ends: [number, number][] = [];
  for (const boundary of boundaries) {
    const at = boundary - top;
    if (at >= -GRAVITY.beyondTop * height && at <= GRAVITY.nearTop * height)
      starts.push([Math.abs(at), boundary]);
    else if (at >= (1 - GRAVITY.nearBottom) * height && at <= height)
      ends.push([height - at, boundary - height]);
  }
  let target = nearest(starts) ?? nearest(ends);
  if (target == null) return undefined;
  target = Math.max(0, Math.min(target, scrollHeight - height));
  return Math.abs(target - top) < 2 ? undefined : target;
}
