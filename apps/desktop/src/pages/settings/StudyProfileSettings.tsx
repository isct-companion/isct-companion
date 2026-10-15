import { useEffect, useState } from "react";
import { t } from "../../i18n";
import {
  contentLanguage,
  pickLocalized,
  type StudyEnrollment,
  type Workspace,
} from "../../lib/model";
import { SYLLABUS_DEPARTMENTS, departmentById } from "../../lib/departments";
import { UiButton, UiSelect } from "../../components/mantine";
import { Row, SettingsGroup, SettingsHeader } from "../../components/forms";
import { jstDate } from "../../app/shared";
import {
  useScheduleQuery,
  type AcademicYearIndex,
} from "../../platform/core-query";

type DraftEnrollment = Omit<StudyEnrollment, "admittedTerm"> & {
  admittedTerm: StudyEnrollment["admittedTerm"] | "";
  uiId: string;
};

const draftEnrollments = (rows: StudyEnrollment[]): DraftEnrollment[] =>
  rows.map((row, index) => ({ ...row, uiId: `${index}:${row.admittedTerm}` }));

const message = (reason: unknown) =>
  reason instanceof Error ? reason.message : String(reason);

export function StudyProfileSettings({
  data,
  update,
  setStudyProfile,
  refreshReferences,
  openSection,
}: {
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
  setStudyProfile: (
    admissionYear: number | undefined,
    studyEnrollments: StudyEnrollment[],
  ) => Promise<void>;
  refreshReferences?: () => Promise<string>;
  /** Opens the academic calendar; absent where it is not available. */
  openSection?: (section: "calendar") => void;
}) {
  const department = departmentById(data.settings.departmentId);
  const admissionYear = data.academicProfile.admissionYear;
  const studyEnrollments = data.academicProfile.studyEnrollments ?? [];
  const committedKey = JSON.stringify([admissionYear, studyEnrollments]);
  const [draft, setDraft] = useState(() => ({
    admissionYear,
    enrollments: draftEnrollments(studyEnrollments),
  }));
  const [dirty, setDirty] = useState(false);
  const [baselineKey, setBaselineKey] = useState(committedKey);
  const [awaitingKey, setAwaitingKey] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    if (dirty) return;
    // The mutation promise may resolve one render before the parent adopts
    // its snapshot. Ignore only that exact pre-save baseline. A different
    // revision is newer external state and must be adopted.
    if (awaitingKey && committedKey === baselineKey) return;
    setDraft({
      admissionYear,
      enrollments: draftEnrollments(studyEnrollments),
    });
    setBaselineKey(committedKey);
    setAwaitingKey(null);
  }, [committedKey, dirty, awaitingKey, baselineKey]);
  const yearIndex = useScheduleQuery<AcademicYearIndex>({
    kind: "academicYears",
    date: jstDate(new Date()),
    selectedYear: Number(data.selectedTerm.slice(0, 4)),
  }).value;
  const thisYear =
    yearIndex?.currentYear ?? Number(data.selectedTerm.slice(0, 4));
  const years = Array.from({ length: 10 }, (_, index) => thisYear + 1 - index);
  if (admissionYear && !years.includes(admissionYear))
    years.push(admissionYear);
  for (const enrollment of draft.enrollments) {
    const year = Number(enrollment.admittedTerm.slice(0, 4));
    if (Number.isFinite(year) && !years.includes(year)) years.push(year);
  }
  years.sort((a, b) => b - a);
  const termOptions = years.flatMap((year) =>
    (["1Q", "2Q", "3Q", "4Q"] as const).map(
      (quarter) => `${year}-${quarter}` as const,
    ),
  );
  const editDraft = (
    edit: (enrollments: DraftEnrollment[]) => DraftEnrollment[],
  ) => {
    setDraft((current) => ({
      ...current,
      enrollments: edit(structuredClone(current.enrollments)),
    }));
    setDirty(true);
    setError("");
  };
  const cancel = () => {
    setDraft({
      admissionYear,
      enrollments: draftEnrollments(studyEnrollments),
    });
    setDirty(false);
    setBaselineKey(committedKey);
    setAwaitingKey(null);
    setError("");
  };
  const save = async () => {
    if (!dirty || saving) return;
    if (committedKey !== baselineKey) {
      setError(
        t(
          "The study profile changed elsewhere. Cancel to load it, then apply your edits again.",
        ),
      );
      return;
    }
    if (draft.enrollments.some((enrollment) => !enrollment.admittedTerm)) {
      setError(t("Choose an admission term for every study stage."));
      return;
    }
    setSaving(true);
    setError("");
    try {
      const submitted = draft.enrollments
        .map(({ uiId: _, ...enrollment }) => ({
          ...enrollment,
          admittedTerm:
            enrollment.admittedTerm as StudyEnrollment["admittedTerm"],
        }))
        .sort((left, right) =>
          left.admittedTerm.localeCompare(right.admittedTerm),
        );
      await setStudyProfile(draft.admissionYear, submitted);
      if (draft.admissionYear !== admissionYear)
        await refreshReferences?.().catch(() => undefined);
      setDraft((current) => ({
        ...current,
        enrollments: draftEnrollments(submitted),
      }));
      setAwaitingKey(JSON.stringify([draft.admissionYear, submitted]));
      setDirty(false);
    } catch (reason) {
      setError(message(reason));
    } finally {
      setSaving(false);
    }
  };

  return (
    <>
      <SettingsHeader title="Study" />
      <SettingsGroup>
        <Row id="department" title="Department">
          <UiSelect
            value={department?.id ?? ""}
            onChange={(event) =>
              update((draft) => {
                draft.settings.departmentId = event.target.value || undefined;
                return draft;
              })
            }
          >
            <option value="">{t("Not selected")}</option>
            {SYLLABUS_DEPARTMENTS.map((item) => (
              <option key={item.id} value={item.id}>
                {pickLocalized(
                  item.name,
                  contentLanguage(data.settings),
                  item.id,
                )}
              </option>
            ))}
          </UiSelect>
        </Row>
        <Row id="admission-year" title="Study-guide cohort year">
          <UiSelect
            disabled={saving}
            value={draft.admissionYear ?? ""}
            onChange={(event) => {
              const year = event.target.value
                ? Number(event.target.value)
                : undefined;
              setDraft((current) => ({ ...current, admissionYear: year }));
              setDirty(true);
              setError("");
            }}
          >
            <option value="">{t("Not set")}</option>
            {years.map((year) => (
              <option key={year} value={year}>
                {t("{{year}} entry", { year })}
              </option>
            ))}
          </UiSelect>
        </Row>
        <Row id="study-stages" title="Study stages" group>
          <div className="studyStages">
            <p className="settingsNote">
              {t(
                "Used for course eligibility by term. Missing details stay unknown.",
              )}
            </p>
            {draft.enrollments.map((enrollment, index) => (
              <div className="studyStage" key={enrollment.uiId}>
                <UiSelect
                  disabled={saving}
                  aria-label={t("Degree stage, stage {{stage}}", {
                    stage: index + 1,
                  })}
                  value={enrollment.stage}
                  onChange={(event) =>
                    editDraft((rows) => {
                      rows[index].stage = event.target
                        .value as StudyEnrollment["stage"];
                      return rows;
                    })
                  }
                >
                  <option value="unknown">{t("Stage not set")}</option>
                  <option value="undergraduate">{t("Undergraduate")}</option>
                  <option value="master">{t("Master")}</option>
                  <option value="doctoral">{t("Doctoral")}</option>
                </UiSelect>
                <UiSelect
                  disabled={saving}
                  aria-label={t("Entry kind, stage {{stage}}", {
                    stage: index + 1,
                  })}
                  value={enrollment.entryKind}
                  onChange={(event) =>
                    editDraft((rows) => {
                      rows[index].entryKind = event.target
                        .value as StudyEnrollment["entryKind"];
                      return rows;
                    })
                  }
                >
                  <option value="unknown">{t("Entry kind not set")}</option>
                  <option value="regular">{t("Regular entry")}</option>
                  <option value="transfer">{t("Transfer entry")}</option>
                </UiSelect>
                <UiSelect
                  disabled={saving}
                  aria-label={t("Entry grade, stage {{stage}}", {
                    stage: index + 1,
                  })}
                  value={enrollment.entryGrade ?? ""}
                  onChange={(event) =>
                    editDraft((rows) => {
                      rows[index].entryGrade = event.target.value
                        ? Number(event.target.value)
                        : undefined;
                      return rows;
                    })
                  }
                >
                  <option value="">{t("Grade not set")}</option>
                  {Array.from({ length: 9 }, (_, grade) => grade + 1).map(
                    (grade) => (
                      <option key={grade} value={grade}>
                        {t("Grade {{grade}}", { grade })}
                      </option>
                    ),
                  )}
                </UiSelect>
                <UiSelect
                  disabled={saving}
                  aria-label={t("Stage admission term, stage {{stage}}", {
                    stage: index + 1,
                  })}
                  value={enrollment.admittedTerm}
                  onChange={(event) =>
                    editDraft((rows) => {
                      rows[index].admittedTerm = event.target
                        .value as StudyEnrollment["admittedTerm"];
                      return rows;
                    })
                  }
                >
                  <option value="">{t("Choose admission term")}</option>
                  {termOptions.map((term) => (
                    <option key={term} value={term}>
                      {term.replace("-", " ")}
                    </option>
                  ))}
                </UiSelect>
                <UiButton
                  type="button"
                  className="textButton"
                  disabled={saving}
                  aria-label={t("Remove stage {{stage}}", {
                    stage: index + 1,
                  })}
                  onClick={() =>
                    editDraft((rows) =>
                      rows.filter((_, rowIndex) => rowIndex !== index),
                    )
                  }
                >
                  {t("Remove")}
                </UiButton>
              </div>
            ))}
            <UiButton
              type="button"
              disabled={saving}
              onClick={() => {
                const used = new Set(
                  draft.enrollments.map(
                    (enrollment) => enrollment.admittedTerm,
                  ),
                );
                editDraft((rows) => [
                  ...rows,
                  {
                    stage: "unknown",
                    entryKind: "unknown",
                    admittedTerm: used.has(data.selectedTerm)
                      ? ""
                      : data.selectedTerm,
                    uiId: `new:${Date.now()}:${rows.length}`,
                  },
                ]);
              }}
            >
              {t("Add study stage")}
            </UiButton>
            <div className="studyStageActions">
              <UiButton
                type="button"
                className="textButton"
                disabled={!dirty || saving}
                onClick={cancel}
              >
                {t("Cancel")}
              </UiButton>
              <UiButton
                type="button"
                disabled={!dirty || saving}
                onClick={() => void save()}
              >
                {saving ? t("Saving…") : t("Save study profile")}
              </UiButton>
            </div>
            {error && (
              <p className="settingsNote studyStageError" role="alert">
                {error}
              </p>
            )}
          </div>
        </Row>
      </SettingsGroup>
      {openSection && (
        <SettingsGroup>
          <Row id="calendar" title="Academic calendar">
            <UiButton
              className="textButton"
              onClick={() => openSection("calendar")}
            >
              {t("Edit")} ›
            </UiButton>
          </Row>
        </SettingsGroup>
      )}
    </>
  );
}
