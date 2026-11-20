import {
  examConflictLabel,
  examKindLabel,
} from "../../components/exams/ExamCards";
import { ExamEditor } from "../../components/exams/ExamEditor";
import { type ReactNode } from "react";
import { t } from "../../i18n";
import { type ScheduleView } from "../../platform/core-query";
import type { LectureSearch } from "../../generated/persistence";

export function LectureExamList({
  schedule,
  query,
  search,
  examKey,
  chooseExam,
}: {
  schedule?: ScheduleView;
  query: string;
  search?: LectureSearch;
  examKey: string | null;
  chooseExam: (key: string, term: string) => void;
}) {
  const exams = schedule?.exams ?? [];
  const needle = query.trim().toLowerCase();
  const shown = exams.filter((exam) =>
    needle
      ? Boolean(search?.examKeys.includes(exam.key))
      : Boolean(exam.courseId) || exam.key === examKey,
  );
  // Classes the timetable lists in exam week are not exams.
  const classes = shown.filter((exam) => exam.kind === "class");
  const row = (exam: (typeof shown)[number]) => (
    <button
      type="button"
      key={exam.key}
      className={`lectureRow exam${exam.kind === "class" ? " examWeekClass" : ""}${examKey === exam.key ? " selected" : ""}`}
      aria-current={examKey === exam.key ? "true" : undefined}
      style={{ "--course": exam.color } as React.CSSProperties}
      onClick={() => chooseExam(exam.key, exam.term)}
    >
      <b>{exam.title || exam.code}</b>
      <small>
        {exam.code} · {exam.date}
        {exam.startTime && exam.endTime
          ? ` · ${exam.startTime}–${exam.endTime}`
          : ""}
      </small>
      <small>
        {exam.kind === "quiz" ? `${t("common.quiz")} · ` : ""}
        {exam.origin === "session"
          ? t("exam.markedOnTheCalendar")
          : exam.courseId
            ? t("lectures.linked")
            : t("common.notLinked")}
        {exam.status === "cancelled"
          ? ` · ${t("exam.status.cancelled")}`
          : ""}
      </small>
      {exam.conflict && (
        <small className="examConflictTag">
          {examConflictLabel(exam)}
        </small>
      )}
    </button>
  );
  return (
    <>
      <div className="lectureExamScope">
        <small>
          {needle
            ? t("lectures.searchPublishedExams")
            : t("lectures.myExams")}
        </small>
        {!shown.length && (
          <p>
            {t(
              needle
                ? "lectures.noMatchingExams"
                : "lectures.noLinkedExamsThisQuarter",
            )}
          </p>
        )}
      </div>
      {shown.filter((exam) => exam.kind !== "class").map(row)}
      {classes.length > 0 && (
        <div className="lectureExamScope">
          <small>{t("lectures.classesInExamWeek")}</small>
        </div>
      )}
      {classes.map(row)}
    </>
  );
}

export function LectureExamDetails({
  selectedExam,
  courses,
  entering,
  endEntrance,
  sidebarToggle,
  onRemoved,
}: {
  selectedExam?: ScheduleView["exams"][number];
  courses: ScheduleView["courses"];
  entering: string | null;
  endEntrance: (event: React.AnimationEvent) => void;
  sidebarToggle: ReactNode;
  onRemoved: () => void;
}) {
  return (
    <div
      className={`lectureExamDetails${selectedExam && entering === `exam:${selectedExam.key}` ? " entering" : ""}`}
      onAnimationEnd={endEntrance}
    >
      {sidebarToggle}
      {selectedExam ? (
        <>
          <header>
            <h1>{selectedExam.title || selectedExam.code}</h1>
            <p>
              {selectedExam.code} ·{" "}
              {selectedExam.term.replace("-", " ")} ·{" "}
              {examKindLabel(selectedExam)}
            </p>
          </header>
          <ExamEditor
            key={selectedExam.key}
            exam={selectedExam}
            courses={courses}
            onRemoved={onRemoved}
          />
        </>
      ) : (
        <p className="emptyHint">{t("lectures.selectAnExamToEdit")}</p>
      )}
    </div>
  );
}
