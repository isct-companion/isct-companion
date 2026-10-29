import { useEffect, useRef, useState } from "react";
import { desktopBridge } from "../../app/native";
import { t } from "../../i18n";
import {
  contentLanguage,
  pickLocalized,
  type StudyEnrollment,
  type Workspace,
} from "../../lib/model";
import { SYLLABUS_DEPARTMENTS, departmentById } from "../../lib/departments";
import { tokyoDate } from "../../lib/civil-date";
import { UiButton, UiSelect } from "../../components/ui";
import { Row, SettingsGroup, SettingsHeader } from "../../components/forms";
import {
  useScheduleQuery,
  type AcademicYearIndex,
} from "../../platform/core-query";
import { useClock } from "../../platform/use-clock";

type DraftEnrollment = Omit<StudyEnrollment, "admittedTerm"> & {
  admittedTerm: StudyEnrollment["admittedTerm"] | "";
};

const draftEnrollments = (rows: StudyEnrollment[]): DraftEnrollment[] =>
  rows.map((row) => ({ ...row }));

const message = (reason: unknown) =>
  reason instanceof Error ? reason.message : String(reason);

export function StudyProfileSettings({
  data,
  update,
  setStudyProfile,
  openSection,
}: {
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
  setStudyProfile: (
    admissionYear: number | undefined,
    studyEnrollments: StudyEnrollment[],
  ) => Promise<void>;
  /** Opens the academic calendar; absent where it is not available. */
  openSection?: (section: "calendar") => void;
}) {
  const now = useClock();
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
  const [suggestedYear, setSuggestedYear] = useState<number>();
  const suggested = useRef(false);
  useEffect(() => {
    let active = true;
    void desktopBridge
      ?.schoolCredentialStatus()
      .then((status) => {
        if (active) setSuggestedYear(status.suggestedAdmissionYear);
      })
      .catch(() => undefined);
    return () => {
      active = false;
    };
  }, []);
  useEffect(() => {
    if (admissionYear || dirty || !suggestedYear || suggested.current) return;
    suggested.current = true;
    setDraft((current) => ({ ...current, admissionYear: suggestedYear }));
    setDirty(true);
  }, [admissionYear, dirty, suggestedYear]);

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
    date: tokyoDate(new Date(now)),
    selectedYear: Number(data.selectedTerm.slice(0, 4)),
  }).value;
  const thisYear =
    yearIndex?.currentYear ?? Number(data.selectedTerm.slice(0, 4));
  const years = Array.from({ length: 10 }, (_, index) => thisYear + 1 - index);
  if (draft.admissionYear && !years.includes(draft.admissionYear))
    years.push(draft.admissionYear);
  years.sort((a, b) => b - a);
  const currentEnrollment = draft.enrollments.at(-1);
  const setDegree = (stage: StudyEnrollment["stage"]) => {
    setDraft((current) => {
      const enrollments = structuredClone(current.enrollments);
      if (enrollments.length) enrollments[enrollments.length - 1].stage = stage;
      else enrollments.push({ stage, entryKind: "unknown", admittedTerm: "" });
      return { ...current, enrollments };
    });
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
      setError(t("settings.studyProfile.theStudyProfileChangedElsewhere"));
      return;
    }
    if (
      draft.enrollments.some((enrollment) => !enrollment.admittedTerm) &&
      !draft.admissionYear
    ) {
      setError(t("settings.studyProfile.chooseAnAdmissionYear"));
      return;
    }
    setSaving(true);
    setError("");
    try {
      const submitted = draft.enrollments
        .map((enrollment) => ({
          ...enrollment,
          admittedTerm: (enrollment.admittedTerm ||
            `${draft.admissionYear}-1Q`) as StudyEnrollment["admittedTerm"],
        }))
        .sort((left, right) =>
          left.admittedTerm.localeCompare(right.admittedTerm),
        );
      await setStudyProfile(draft.admissionYear, submitted);
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
      <SettingsHeader title="settings.academicCalendar.study" />
      <SettingsGroup>
        <Row id="department" title="settings.main.department">
          <UiSelect
            value={department?.id ?? ""}
            onChange={(value) => {
              const departmentId = value || undefined;
              update((draft) => {
                draft.settings.departmentId = departmentId;
                return draft;
              });
            }}
            data={[
              { value: "", label: t("settings.studyProfile.notSelected") },
              ...SYLLABUS_DEPARTMENTS.map((item) => ({
                value: item.id,
                label: pickLocalized(
                  item.name,
                  contentLanguage(data.settings),
                  item.id,
                ),
              })),
            ]}
          />
        </Row>
        <Row id="admission-year" title="settings.main.admissionYear">
          <UiSelect
            disabled={saving}
            value={draft.admissionYear ?? ""}
            onChange={(value) => {
              const year = value ? Number(value) : undefined;
              setDraft((current) => ({ ...current, admissionYear: year }));
              setDirty(true);
              setError("");
            }}
            data={[
              { value: "", label: t("settings.studyProfile.notSet") },
              ...years.map((year) => ({
                value: String(year),
                label: t("settings.dataUpdates.yearEntry", { year }),
              })),
            ]}
          />
        </Row>
        <Row id="study-stages" title="settings.studyProfile.degreeProgramme">
          <UiSelect
            disabled={saving}
            aria-label={t("settings.studyProfile.degreeProgramme")}
            value={currentEnrollment?.stage ?? "unknown"}
            onChange={(value) => setDegree(value as StudyEnrollment["stage"])}
            data={[
              { value: "unknown", label: t("settings.studyProfile.notSet") },
              {
                value: "undergraduate",
                label: t("settings.studyProfile.undergraduate"),
              },
              { value: "master", label: t("settings.studyProfile.master") },
              {
                value: "professional",
                label: t("settings.studyProfile.professionalDegree"),
              },
              { value: "doctoral", label: t("settings.studyProfile.doctoral") },
            ]}
          />
        </Row>
        <div className="studyProfileActions">
          <UiButton
            type="button"
            className="button"
            disabled={!dirty || saving}
            onClick={cancel}
          >
            {t("common.cancel")}
          </UiButton>
          <UiButton
            type="button"
            className="button primary"
            disabled={!dirty || saving}
            onClick={() => void save()}
          >
            {saving
              ? t("settings.studyProfile.saving")
              : t("settings.studyProfile.saveEnrollmentDetails")}
          </UiButton>
        </div>
        {error && (
          <p className="settingsNote studyProfileError" role="alert">
            {error}
          </p>
        )}
      </SettingsGroup>
      {openSection && (
        <SettingsGroup>
          <Row id="calendar" title="settings.academicCalendar.academicCalendar">
            <UiButton
              className="textButton"
              onClick={() => openSection("calendar")}
            >
              {t("common.edit")} ›
            </UiButton>
          </Row>
        </SettingsGroup>
      )}
    </>
  );
}
