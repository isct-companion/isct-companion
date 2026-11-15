import { UiButton } from "../../components/ui";
import type { ExamCourseList } from "../../generated/persistence";
import { t } from "../../i18n";
import { useScheduleQuery } from "../../platform/core-query";
import "./past-exams.css";

/** This quarter's enrolled courses, read from the device. A course opens its
 * subject only when chosen. */
export function CurrentCourses({
  active,
  disabled,
  choose,
}: {
  active: boolean;
  disabled: boolean;
  choose: (courseId: string) => void;
}) {
  const { value, error, retry } = useScheduleQuery<ExamCourseList>(
    { kind: "examCourses" },
    active,
  );

  return (
    <section aria-label={t("examShare.currentCourses")}>
      <h2>{t("examShare.currentCourses")}</h2>
      {error && (
        <div role="alert">
          {error} <UiButton onClick={retry}>{t("common.retry")}</UiButton>
        </div>
      )}
      {!value && !error && <p role="status">{t("common.loading")}</p>}
      {value &&
        (value.courses.length === 0 ? (
          <p role="status">{t("examShare.noCurrentCourses")}</p>
        ) : (
          <ul className="examImportList">
            {value.courses.map((course) => (
              <li key={course.courseId}>
                <div>
                  <UiButton
                    className="textButton"
                    disabled={disabled || course.subjectCodes.length === 0}
                    onClick={() => choose(course.courseId)}
                  >
                    {course.title}
                  </UiButton>
                  {course.instructor && <small>{course.instructor}</small>}
                </div>
                <small>
                  {course.subjectCodes.join(" / ") ||
                    t("examShare.noSubjectCode")}
                </small>
              </li>
            ))}
          </ul>
        ))}
    </section>
  );
}
