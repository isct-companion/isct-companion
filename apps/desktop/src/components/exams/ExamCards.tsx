import { Modal, UiButton } from "../ui";
import { useState } from "react";
import type { ExamView } from "../../generated/persistence";
import type { Course } from "../../lib/model";
import { ExamEditor } from "./ExamEditor";
import { t } from "../../i18n";
import "./exams.css";

/** A class the exam timetable lists in the exam period is not an exam. */
export const examKindLabel = (exam: Pick<ExamView, "kind">) =>
  t(exam.kind === "class" ? "Class in exam week" : "Exam");

/** "2026-12-08 · 10:45–12:25", or the date with "Time not announced". */
export const examWhen = (
  exam: Pick<ExamView, "date" | "startTime" | "endTime">,
) =>
  `${exam.date} · ${
    exam.startTime && exam.endTime
      ? `${exam.startTime}–${exam.endTime}`
      : t("Time not announced")
  }`;

export function ExamCards({
  exams,
  courses,
  open,
  openExam,
}: {
  exams: ExamView[];
  courses: Course[];
  open?: (courseId: string) => void;
  openExam?: (key: string, term: string) => void;
}) {
  const [editing, setEditing] = useState<ExamView | null>(null);
  // Exams first; the timetable's exam-week classes form their own group.
  const classes = exams.filter((exam) => exam.kind === "class");
  const ordered = [
    ...exams.filter((exam) => exam.kind !== "class"),
    ...classes,
  ];
  return (
    <div className="examCards">
      {ordered.map((exam) => (
        <article
          key={exam.key}
          className={`examCard${exam.kind === "class" ? " examWeekClass" : ""}${exam.hidden ? " hidden" : ""}`}
          style={{ "--course": exam.color } as React.CSSProperties}
        >
          <div className="examCardHeading">
            <span>
              {examKindLabel(exam)}
              {exam.status === "cancelled" ? ` · ${t("Cancelled")}` : ""}
            </span>
            <UiButton
              className="textButton"
              onClick={() => {
                setEditing(exam);
              }}
            >
              {t("Edit")}
            </UiButton>
          </div>
          {openExam || (exam.courseId && open) ? (
            <button
              type="button"
              className="examCourseLink"
              onClick={() =>
                openExam
                  ? openExam(exam.key, exam.term)
                  : exam.courseId && open?.(exam.courseId)
              }
            >
              {exam.title || exam.code}
            </button>
          ) : (
            <b>{exam.title || exam.code}</b>
          )}
          <small>
            {exam.code}
            {!exam.courseId ? ` · ${t("Not linked")}` : ""}
            {exam.edited ? ` · ${t("Edited")}` : ""}
            {exam.hidden ? ` · ${t("Hidden")}` : ""}
          </small>
          <time dateTime={exam.date}>{examWhen(exam)}</time>
          {exam.room && <small>{exam.room}</small>}
          {exam.notes && <small>{exam.notes}</small>}
        </article>
      ))}
      <Modal
        opened={Boolean(editing)}
        onClose={() => setEditing(null)}
        title={t("Edit exam")}
      >
        {editing && (
          <ExamEditor
            key={editing.key}
            exam={editing}
            courses={courses}
            onSaved={() => setEditing(null)}
          />
        )}
      </Modal>
    </div>
  );
}
