import { useState } from "react";
import { UiButton, UiSelect } from "../../components/ui";
import type { ExamCourseImportPreview } from "../../generated/persistence";
import { t } from "../../i18n";
import { useScheduleQuery } from "../../platform/core-query";
import "./past-exams.css";

export function TimetableImportPreview({
  terms,
  currentTerm,
  active,
}: {
  terms: string[];
  currentTerm: string;
  active: boolean;
}) {
  const [term, setTerm] = useState<string | null>(null);
  const { value, error, pending, retry } =
    useScheduleQuery<ExamCourseImportPreview>(
      { kind: "examCourseImport", term },
      active,
    );
  const preview = value?.term === (term ?? currentTerm) ? value : undefined;

  return (
    <section className="page" aria-busy={pending}>
      <header className="pageBar">
        <div>
          <h2>{t("examShare.importPreview")}</h2>
        </div>
        <UiSelect
          aria-label={t("common.quarter")}
          value={term ?? ""}
          onChange={(value) => setTerm(value || null)}
          data={[{ value: "", label: t("examShare.currentQuarter") }, ...terms]}
        />
      </header>
      {error && (
        <div role="alert">
          {error} <UiButton onClick={retry}>{t("common.retry")}</UiButton>
        </div>
      )}
      {!preview && !error && <p role="status">{t("common.loading")}</p>}
      {preview &&
        (preview.courses.length === 0 ? (
          <p role="status">{t("examShare.emptyTimetable")}</p>
        ) : (
          <ul
            className="examImportList"
            aria-label={t("examShare.importPreview")}
          >
            {preview.courses.map((course) => (
              <li key={course.courseId}>
                <div>
                  <strong>{course.title}</strong>
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
