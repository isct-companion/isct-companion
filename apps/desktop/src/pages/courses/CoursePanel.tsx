import type { PageProgress } from "../../platform/sync";
import { ExternalLink } from "../../components/ExternalLink";
import { Button, Paper } from "@mantine/core";
import { t } from "../../i18n";
import {
  pickLocalized,
  type Course,
  type Lang,
  type Workspace,
} from "../../lib/model";
import type { FinderCourse } from "../../lib/course-query";
import type { CourseConflict } from "../../generated/persistence";
import { getDesktopBridge } from "../../platform/desktop-bridge";
import { SyllabusDetails } from "./syllabus-details";
import { CourseDetails } from "./CourseDetails";

/**
 * A course's details, shown over the syllabus search after a timetable click.
 * Removing lives only here, at the bottom, so it always takes two clicks.
 */
export function CoursePanel({
  course,
  data,
  readOnly,
  close,
  remove,
  editIntensive,
  openWeb,
  fetchLms,
  progress,
  conflicts = [],
  openCourse,
  findOtherTimes,
}: {
  course: Course;
  data: Workspace;
  readOnly: boolean;
  close: () => void;
  remove: () => void;
  editIntensive: () => void;
  /** Opens an official URL in the Web page. */
  openWeb: (url: string) => void;
  /** Pulls LMS (assignments and materials) without changing the timetable. */
  fetchLms?: () => void;
  progress?: PageProgress;
  /** This course's clashes on the quarter template, from Core. */
  conflicts?: CourseConflict[];
  /** Shows another timetable course in this panel. */
  openCourse?: (courseId: string) => void;
  /** Searches the syllabus for this course's other class times. */
  findOtherTimes?: () => void;
}) {
  const registered = data.memberships.some(
    (item) =>
      item.courseId === course.id &&
      !item.deletedAt &&
      ["registered", "declared", "completed"].includes(item.status) &&
      item.evidence?.provider !== "manual",
  );
  return (
    <Paper
      component="aside"
      className="listCard coursePanel"
      withBorder
      shadow="xs"
      radius="md"
      style={{ "--course": course.color } as React.CSSProperties}
    >
      <header>
        <div className="finderTitle">
          <b>{course.title}</b>
          <span>
            {course.code} · {course.termId.replace("-", " ")}
          </span>
        </div>
        <div className="courseListTools">
          <button type="button" onClick={close}>
            {t("Close")}
          </button>
        </div>
      </header>
      {conflicts.length > 0 && (
        <section
          className="coursePanelConflict"
          aria-label={t("Time conflict")}
        >
          <b>{t("Time conflict")}</b>
          <ul>
            {conflicts.map((conflict) => (
              <li
                key={`${conflict.day}-${conflict.startTime}-${conflict.withCourseId ?? conflict.withTitle}`}
              >
                <span className="num">
                  {FINDER_DAYS[conflict.day - 1]} {conflict.startTime}–
                  {conflict.endTime}
                </span>
                <span>{conflict.withTitle}</span>
                {conflict.withCourseId && openCourse && (
                  <button
                    type="button"
                    className="textButton"
                    onClick={() => openCourse(conflict.withCourseId!)}
                  >
                    {t("Open")}
                  </button>
                )}
              </li>
            ))}
          </ul>
          <p>
            {t(
              "Keep one: open the other course, or find another time for this one.",
            )}
          </p>
          {!readOnly && findOtherTimes && (
            <Button size="xs" variant="light" onClick={findOtherTimes}>
              {t("Find another class time")}
            </Button>
          )}
        </section>
      )}
      <CourseDetails
        key={course.id}
        course={course}
        data={data}
        openWeb={openWeb}
        fetchLms={fetchLms}
        progress={progress}
      />
      {!readOnly && (
        <footer className="coursePanelFoot">
          {!course.meetings.length && (
            <Button variant="default" onClick={editIntensive}>
              {t("Intensive dates")}
            </Button>
          )}
          <Button color="red" variant="light" onClick={remove}>
            {registered ? t("Remove from local timetable") : t("Remove")}
          </Button>
        </footer>
      )}
    </Paper>
  );
}

const FINDER_DAYS = ["月", "火", "水", "木", "金", "土", "日"];

/**
 * A syllabus search result, shown over the timetable: what the syllabus says
 * about it, with Add at the bottom.
 */
export function OfferingPanel({
  course,
  language,
  placed,
  replaces = [],
  readOnly,
  close,
  add,
}: {
  course: FinderCourse;
  language: Lang;
  placed: boolean;
  /** Timetable courses that Add would replace (same periods). */
  replaces?: string[];
  readOnly: boolean;
  close: () => void;
  add: () => void;
}) {
  const bridge = getDesktopBridge();
  const times = course.schedule
    .map((entry) =>
      entry.kind === "weekly" && entry.day && entry.periods
        ? `${FINDER_DAYS[entry.day - 1]}${entry.periods[0]}-${entry.periods[1]}${entry.room ? ` · ${pickLocalized(entry.room, language)}` : ""}`
        : t("集中・時間未定"),
    )
    .join(" / ");
  return (
    <Paper
      component="aside"
      className="listCard coursePanel"
      withBorder
      shadow="xs"
      radius="md"
    >
      <header>
        <div className="finderTitle">
          <b>{pickLocalized(course.title, language)}</b>
          <span>
            {course.code} · {course.quarters.join(" ")}
          </span>
        </div>
        <div className="courseListTools">
          <ExternalLink href={course.syllabusUrl}>
            {t("Syllabus")} ↗
          </ExternalLink>
          <button type="button" onClick={close}>
            {t("Close")}
          </button>
        </div>
      </header>
      <div className="coursePanelBody">
        <dl>
          <dt>{t("Schedule")}</dt>
          <dd>{course.detailed === false ? "…" : times || "—"}</dd>
          <dt>{t("Instructor")}</dt>
          <dd>
            {course.instructors
              .map((name) => pickLocalized(name, language))
              .join(" / ") || "—"}
          </dd>
          <dt>{t("Credits")}</dt>
          <dd>
            {course.detailed === false
              ? "…"
              : `${course.credits.lecture}-${course.credits.exercise}-${course.credits.lab}`}
          </dd>
        </dl>
        <SyllabusDetails
          url={course.syllabusUrl}
          fetchCourse={
            bridge ? (url) => bridge.syllabusCourse(url, language) : undefined
          }
        />
      </div>
      {!readOnly && (
        <footer className="coursePanelFoot">
          {!placed && replaces.length > 0 && (
            <span className="coursePanelNote">
              {t("Replaces")} {replaces.join(", ")}
            </span>
          )}
          <Button
            disabled={placed}
            color={replaces.length ? "orange" : undefined}
            onClick={add}
          >
            {placed
              ? t("On timetable")
              : replaces.length
                ? t("Replace")
                : t("Add")}
          </Button>
        </footer>
      )}
    </Paper>
  );
}
