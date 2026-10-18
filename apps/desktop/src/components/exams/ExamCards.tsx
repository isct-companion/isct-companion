import { Modal } from "@mantine/core";
import { useState } from "react";
import type { ExamView } from "../../generated/persistence";
import type { Course, Workspace } from "../../lib/model";
import { ExamEditor } from "./ExamEditor";
import { t } from "../../i18n";
import "./exams.css";

export function ExamCards({
  exams,
  data,
  courses,
  open,
}: {
  exams: ExamView[];
  data: Workspace;
  courses: Course[];
  open?: (courseId: string) => void;
}) {
  const [editing, setEditing] = useState<ExamView | null>(null);
  return (
    <div className="examCards">
      {exams.map((exam) => (
        <article
          key={exam.key}
          className={`examCard${exam.hidden ? " hidden" : ""}`}
          style={{ "--course": exam.color } as React.CSSProperties}
        >
          <div className="examCardHeading">
            <span>
              {t("Exam")}
              {exam.status === "cancelled" ? ` · ${t("Cancelled")}` : ""}
            </span>
            <button
              type="button"
              className="textButton"
              onClick={() => {
                setEditing(exam);
              }}
            >
              {t("Edit")}
            </button>
          </div>
          {exam.courseId && open ? (
            <button
              type="button"
              className="examCourseLink"
              onClick={() => open(exam.courseId!)}
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
          <time dateTime={exam.date}>
            {exam.date}
            {exam.startTime && exam.endTime
              ? ` · ${exam.startTime}–${exam.endTime}`
              : ` · ${t("Time not announced")}`}
          </time>
          {exam.room && <small>{exam.room}</small>}
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
            data={data}
            courses={courses}
            onSaved={() => setEditing(null)}
          />
        )}
      </Modal>
    </div>
  );
}
