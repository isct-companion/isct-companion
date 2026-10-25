import { Button, Paper } from "../../components/ui";
import type { ReactNode } from "react";
import { openExternalUrl } from "../../components/ExternalLink";
import { examWhen } from "../../components/exams/ExamCards";
import { dayNames } from "../../components/timetable/timetable";
import { t } from "../../i18n";
import {
  pickLocalized,
  type ClassGroupChoice,
  type Course,
  type Lang,
  type Workspace,
} from "../../lib/model";
import { tokyoDate } from "../../lib/civil-date";
import { DAYS, type FinderCourse } from "../../lib/course-query";
import type { CourseConflict } from "../../generated/persistence";
import { notify } from "../../app/notify";
import { useScheduleQuery, type CourseDetail } from "../../platform/core-query";
import { useClock } from "../../platform/use-clock";
import { workspaceActions } from "../../store/workspace";
import { ClassGroupSwitch } from "./ClassGroupPicker";
import { SyllabusFrame } from "./syllabus-frame";

/**
 * The Timetable's planning panel. A timetable course and a search result open
 * the same frame: the facts that decide a timetable, the syllabus, and the
 * action that changes the timetable. Studying the course lives in Lectures.
 */
function PlanningPanel({
  title,
  subtitle,
  color,
  close,
  notice,
  children,
  foot,
}: {
  title: string;
  subtitle: string;
  color?: string;
  close: () => void;
  notice?: ReactNode;
  children: ReactNode;
  foot?: ReactNode;
}) {
  return (
    <Paper
      component="aside"
      className="listCard coursePanel"
      withBorder
      shadow="xs"
      radius="md"
      style={color ? ({ "--course": color } as React.CSSProperties) : undefined}
    >
      <header>
        <div className="finderTitle">
          <b>{title}</b>
          <span>{subtitle}</span>
        </div>
        <div className="courseListTools">
          <button type="button" onClick={close}>
            {t("Close")}
          </button>
        </div>
      </header>
      {notice}
      <div className="coursePanelBody">{children}</div>
      {foot && <footer className="coursePanelFoot">{foot}</footer>}
    </Paper>
  );
}

/** Opens in the app's Web page; the ↗ next to it opens the system browser. */
function OfficialLink({
  url,
  label,
  openWeb,
}: {
  url: string;
  label: ReactNode;
  openWeb: (url: string) => void;
}) {
  return (
    <span className="officialLink">
      <button type="button" onClick={() => openWeb(url)}>
        {label}
      </button>
      <button
        type="button"
        className="officialOutside"
        title={t("Open in browser")}
        aria-label={t("Open in browser")}
        onClick={() => void openExternalUrl(url).catch(() => undefined)}
      >
        ↗
      </button>
    </span>
  );
}

/**
 * A timetable course. Removing lives only here, at the bottom, so it always
 * takes two clicks.
 */
export function CoursePanel({
  course,
  data,
  readOnly,
  close,
  remove,
  edit,
  editIntensive,
  openWeb,
  openLecture,
  conflicts = [],
  openCourse,
  findOtherTimes,
}: {
  course: Course;
  data: Workspace;
  readOnly: boolean;
  close: () => void;
  remove: () => void;
  edit: () => void;
  editIntensive: () => void;
  /** Opens an official URL in the Web page. */
  openWeb: (url: string) => void;
  /** Opens the course in Lectures, where its materials and work live. */
  openLecture: () => void;
  /** This course's clashes on the quarter template, from Core. */
  conflicts?: CourseConflict[];
  /** Shows another timetable course in this panel. */
  openCourse?: (courseId: string) => void;
  /** Searches the syllabus for this course's other class times. */
  findOtherTimes?: () => void;
}) {
  const now = useClock();
  const detail = useScheduleQuery<CourseDetail>({
    kind: "courseDetail",
    courseId: course.id,
  }).value;
  const groupChoices = (
    useScheduleQuery<ClassGroupChoice[]>({
      kind: "classGroups",
      today: tokyoDate(new Date(now)),
    }).value ?? []
  ).filter((choice) => choice.courses.some((item) => item.id === course.id));
  const exams = (detail?.exams ?? []).filter(
    (exam) => exam.kind !== "class" && !exam.hidden,
  );
  const registered = data.memberships.some(
    (item) =>
      item.courseId === course.id &&
      !item.deletedAt &&
      ["registered", "declared", "completed"].includes(item.status) &&
      item.evidence?.provider !== "manual",
  );
  const notice = conflicts.length > 0 && (
    <section className="coursePanelConflict" aria-label={t("Time conflict")}>
      <b>{t("Time conflict")}</b>
      <ul>
        {conflicts.map((conflict) => (
          <li
            key={`${conflict.day}-${conflict.startTime}-${conflict.withCourseId ?? conflict.withTitle}`}
          >
            <span className="num">
              {DAYS[conflict.day - 1]} {conflict.startTime}–{conflict.endTime}
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
  );
  return (
    <PlanningPanel
      title={course.title}
      subtitle={`${course.code} · ${course.termId.replace("-", " ")}`}
      color={course.color}
      close={close}
      notice={notice}
      foot={
        !readOnly && (
          <>
            {!course.meetings.length && (
              <Button variant="default" onClick={editIntensive}>
                {t("Intensive dates")}
              </Button>
            )}
            <Button variant="default" onClick={edit}>
              {t("Edit")}
            </Button>
            <Button color="red" variant="light" onClick={remove}>
              {registered ? t("Remove from local timetable") : t("Remove")}
            </Button>
          </>
        )
      }
    >
      <dl>
        <dt>{t("Schedule")}</dt>
        <dd>
          {course.meetings.length
            ? course.meetings.map((meeting, index) => (
                <div key={index}>
                  {meeting.days
                    .map((day) => t(dayNames[day - 1].slice(0, 3)))
                    .join(" / ")}{" "}
                  {meeting.startTime}–{meeting.endTime}
                  {meeting.room ? ` · ${meeting.room}` : ""}
                </div>
              ))
            : t("集中・時間未定")}
        </dd>
        {groupChoices.length > 0 && (
          <>
            <dt>{t("Physics group")}</dt>
            <dd>
              {groupChoices.map((choice) => (
                <ClassGroupSwitch
                  key={choice.semester}
                  choice={choice}
                  onChange={(group) =>
                    void workspaceActions
                      .setClassGroup(choice.semester, group)
                      .catch(notify.error)
                  }
                />
              ))}
            </dd>
          </>
        )}
        <dt>{t("Instructor")}</dt>
        <dd>{course.instructor || "—"}</dd>
        <dt>{t("Credits")}</dt>
        <dd>
          {course.creditSplit.lecture}-{course.creditSplit.exercise}-
          {course.creditSplit.lab} ({course.credits})
        </dd>
        {detail?.format && (
          <>
            <dt>{t("Format")}</dt>
            <dd>{detail.format.kinds.join(" / ") || "—"}</dd>
          </>
        )}
        {exams.length > 0 && (
          <>
            {/* One line each: enough to see a clash while planning; the
                cards and their editor are in Lectures. */}
            <dt>{t("Exams")}</dt>
            <dd>
              {exams.map((exam) => (
                <div key={exam.key}>{examWhen(exam)}</div>
              ))}
            </dd>
          </>
        )}
      </dl>
      <div className="coursePanelLinks">
        <Button size="xs" variant="light" onClick={openLecture}>
          {t("Open in Lectures")}
        </Button>
        {detail?.lmsUrl && (
          <OfficialLink url={detail.lmsUrl} label="LMS" openWeb={openWeb} />
        )}
      </div>
      <SyllabusFrame url={detail?.syllabusUrl} />
    </PlanningPanel>
  );
}

/** A syllabus search result, with Add at the bottom. */
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
  const times = course.schedule
    .map((entry) =>
      entry.kind === "weekly" && entry.day && entry.periods
        ? `${DAYS[entry.day - 1]}${entry.periods[0]}-${entry.periods[1]}${entry.room ? ` · ${pickLocalized(entry.room, language)}` : ""}`
        : t("集中・時間未定"),
    )
    .join(" / ");
  return (
    <PlanningPanel
      title={pickLocalized(course.title, language)}
      subtitle={`${course.code} · ${course.quarters.join(" ")}`}
      close={close}
      foot={
        !readOnly && (
          <>
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
          </>
        )
      }
    >
      <dl>
        <dt>{t("Schedule")}</dt>
        <dd>{times || "—"}</dd>
        <dt>{t("Instructor")}</dt>
        <dd>
          {course.instructors
            .map((name) => pickLocalized(name, language))
            .join(" / ") || "—"}
        </dd>
        <dt>{t("Credits")}</dt>
        <dd>
          {`${course.credits.lecture}-${course.credits.exercise}-${course.credits.lab}`}
        </dd>
        <dt>{t("Format")}</dt>
        <dd>{course.format.kinds.join(" / ") || "—"}</dd>
      </dl>
      <SyllabusFrame url={course.syllabusUrl} />
    </PlanningPanel>
  );
}
