import {
  examConflictLabel,
  examKindLabel,
} from "../../components/exams/ExamCards";
import { ExamEditor } from "../../components/exams/ExamEditor";
import { TitleRow } from "../../components/TitleRow";
import { useState, type ReactNode } from "react";
import { FoldToggle } from "../../components/ui";
import { t } from "../../i18n";
import { type LectureList } from "../../platform/core-query";
import type { LectureSearch } from "../../generated/persistence";

export function LectureExamList({
  schedule,
  query,
  search,
  selection,
  chooseExam,
}: {
  schedule?: LectureList;
  query: string;
  search?: LectureSearch;
  selection: { key: string; request: number } | null;
  chooseExam: (key: string, term: string) => void;
}) {
  const [showEnded, setShowEnded] = useState(false);
  const examKey = selection?.key;
  const needle = query.trim().toLowerCase();
  const byKey = new Map(
    (schedule?.exams ?? []).map((exam) => [exam.key, exam]),
  );
  const groups = schedule?.examGroups;
  // Core's groups in their order: exams to come, exam-week classes to come,
  // then what is over, the latest first.
  const pick = (keys: string[] | undefined) =>
    (keys ?? []).flatMap((key) => {
      const exam = byKey.get(key);
      const shown =
        exam &&
        (needle
          ? Boolean(search?.examKeys.includes(exam.key))
          : Boolean(exam.courseId) || exam.key === examKey);
      return shown ? [exam] : [];
    });
  const upcoming = pick(groups?.upcomingKeys);
  // Classes the timetable lists in exam week are not exams.
  const classes = pick(groups?.classKeys);
  const ended = pick(groups?.endedKeys);
  // An ended exam opened from elsewhere unfolds its group once, so the
  // selected row shows; the person can fold it again.
  const [unfoldedFor, setUnfoldedFor] = useState<number | undefined>(undefined);
  if (
    selection &&
    selection.request !== unfoldedFor &&
    ended.some((exam) => exam.key === selection.key)
  ) {
    setUnfoldedFor(selection.request);
    setShowEnded(true);
  }
  // A search shows every match.
  const endedOpen = showEnded || Boolean(needle);
  const row = (exam: LectureList["exams"][number]) => (
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
        {exam.status === "cancelled" ? ` · ${t("exam.status.cancelled")}` : ""}
      </small>
      {exam.conflict && (
        <small className="examConflictTag">{examConflictLabel(exam)}</small>
      )}
    </button>
  );
  const nothing = !upcoming.length && !classes.length && !ended.length;
  return (
    <>
      <div className="lectureExamScope">
        <small>
          {needle ? t("lectures.searchPublishedExams") : t("lectures.myExams")}
        </small>
        {nothing && (
          <p>
            {t(
              needle
                ? "lectures.noMatchingExams"
                : "lectures.noLinkedExamsThisQuarter",
            )}
          </p>
        )}
      </div>
      {upcoming.map(row)}
      {classes.length > 0 && (
        <div className="lectureExamScope">
          <small>{t("lectures.classesInExamWeek")}</small>
        </div>
      )}
      {classes.map(row)}
      {ended.length > 0 && (
        <div className="dueGroup">
          {needle ? (
            <small>
              {t("lectures.endedExams")} · {ended.length}
            </small>
          ) : (
            <FoldToggle
              open={showEnded}
              onToggle={() => setShowEnded((value) => !value)}
            >
              {t("lectures.endedExams")} · {ended.length}
            </FoldToggle>
          )}
          {endedOpen && ended.map(row)}
        </div>
      )}
    </>
  );
}

export function LectureExamDetails({
  selectedExam,
  entering,
  endEntrance,
  sidebarToggle,
  onRemoved,
}: {
  selectedExam?: LectureList["exams"][number];
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
      <TitleRow className="lectureExamHeader">
        {sidebarToggle}
        {selectedExam && (
          <>
            <h1>{selectedExam.title || selectedExam.code}</h1>
            <small>
              {selectedExam.code} · {selectedExam.term.replace("-", " ")} ·{" "}
              {examKindLabel(selectedExam)}
            </small>
          </>
        )}
      </TitleRow>
      {selectedExam ? (
        <>
          <ExamEditor
            key={selectedExam.key}
            exam={selectedExam}
            onRemoved={onRemoved}
          />
        </>
      ) : (
        <p className="emptyHint">{t("lectures.selectAnExamToEdit")}</p>
      )}
    </div>
  );
}
