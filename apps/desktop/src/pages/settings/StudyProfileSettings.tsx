import { errorText } from "../../lib/error-text";
import { useEffect, useState } from "react";
import { desktopBridge } from "../../platform/native";
import { t } from "../../i18n";
import type { StudyEnrollment, Workspace } from "../../lib/model";
import { departmentById } from "../../lib/departments";
import { tokyoDate } from "../../lib/civil-date";
import { UiButton, UiSelect } from "../../components/ui";
import { Row, SettingsGroup, SettingsHeader } from "../../components/forms";
import {
  useScheduleQuery,
  type AcademicYearIndex,
} from "../../platform/core-query";
import { finderQuery } from "../../platform/finder-query";
import { useViewQuery } from "../../platform/view-query";
import type { FinderDepartment } from "../../generated/persistence";
import { useClock } from "../../platform/use-clock";

type DraftEnrollment = Omit<StudyEnrollment, "admittedTerm"> & {
  admittedTerm: StudyEnrollment["admittedTerm"] | "";
};

const draftEnrollments = (rows: StudyEnrollment[]): DraftEnrollment[] =>
  rows.map((row) => ({ ...row }));

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
  // The syllabus departments, named in the content language, as the course
  // finder lists them.
  const departments =
    useViewQuery<FinderDepartment[]>(
      ["finderDepartments", data.selectedTerm],
      async () => {
        const page = await finderQuery({
          year: Number(data.selectedTerm.slice(0, 4)),
          term: data.selectedTerm,
          limit: 0,
        });
        return {
          value: page.context.departments,
          reads: page.reads,
          seq: page.seq,
        };
      },
    ).value ?? [];
  const department = departmentById(departments, data.settings.departmentId);
  const admissionYear = data.academicProfile.admissionYear;
  const studyEnrollments = data.academicProfile.studyEnrollments ?? [];
  const committedKey = JSON.stringify([admissionYear, studyEnrollments]);
  const [draft, setDraft] = useState(() => ({
    admissionYear,
    enrollments: draftEnrollments(studyEnrollments),
  }));
  // A programme chosen before the year waits here for it; every other
  // choice saves at once, like the department above it.
  const [pending, setPending] = useState(false);
  const [baselineKey, setBaselineKey] = useState(committedKey);
  const [awaitingKey, setAwaitingKey] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  // The year the school account suggests is offered, never saved unasked.
  const [suggestedYear, setSuggestedYear] = useState<number>();
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

  const adopt = () => {
    setDraft({
      admissionYear,
      enrollments: draftEnrollments(studyEnrollments),
    });
    setBaselineKey(committedKey);
    setAwaitingKey(null);
  };
  useEffect(() => {
    if (pending || saving) return;
    // The mutation promise may resolve one render before the parent adopts
    // its snapshot. Ignore only that exact pre-save baseline. A different
    // revision is newer external state and must be adopted.
    if (awaitingKey && committedKey === baselineKey) return;
    adopt();
  }, [committedKey, pending, saving, awaitingKey, baselineKey]);
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
  const choose = async (next: typeof draft) => {
    if (saving) return;
    setDraft(next);
    setError("");
    if (pending && committedKey !== baselineKey) {
      // The profile changed on another device while a programme waited.
      setError(t("settings.studyProfile.theStudyProfileChangedElsewhere"));
      setPending(false);
      return;
    }
    if (
      next.enrollments.some((enrollment) => !enrollment.admittedTerm) &&
      !next.admissionYear
    ) {
      setError(t("settings.studyProfile.chooseAnAdmissionYear"));
      setPending(true);
      return;
    }
    setPending(false);
    setSaving(true);
    try {
      const submitted = next.enrollments
        .map((enrollment) => ({
          ...enrollment,
          admittedTerm: (enrollment.admittedTerm ||
            `${next.admissionYear}-1Q`) as StudyEnrollment["admittedTerm"],
        }))
        .sort((left, right) =>
          left.admittedTerm.localeCompare(right.admittedTerm),
        );
      await setStudyProfile(next.admissionYear, submitted);
      setDraft({ ...next, enrollments: draftEnrollments(submitted) });
      setBaselineKey(committedKey);
      setAwaitingKey(JSON.stringify([next.admissionYear, submitted]));
    } catch (reason) {
      // The fields show what is stored; the alert says why the choice did
      // not stay.
      setError(errorText(reason, { nativeMessage: false }) ?? String(reason));
      adopt();
    } finally {
      setSaving(false);
    }
  };
  const chooseYear = (year: number | undefined) =>
    void choose({ ...draft, admissionYear: year });
  const chooseDegree = (stage: StudyEnrollment["stage"]) => {
    const enrollments = structuredClone(draft.enrollments);
    if (enrollments.length) enrollments[enrollments.length - 1].stage = stage;
    else enrollments.push({ stage, entryKind: "unknown", admittedTerm: "" });
    void choose({ ...draft, enrollments });
  };
  const offerYear =
    !draft.admissionYear && suggestedYear && !saving
      ? suggestedYear
      : undefined;

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
              ...departments.map((item) => ({
                value: item.id,
                label: item.name,
              })),
            ]}
          />
        </Row>
        <Row
          id="admission-year"
          title="settings.main.admissionYear"
          hint={
            offerYear && (
              <UiButton
                className="textButton"
                onClick={(event) => {
                  // Inside the row's label: the click is the button's alone.
                  event.preventDefault();
                  chooseYear(offerYear);
                }}
              >
                {t("settings.studyProfile.useDetectedYear", {
                  year: offerYear,
                })}
              </UiButton>
            )
          }
        >
          <UiSelect
            disabled={saving}
            value={draft.admissionYear ?? ""}
            onChange={(value) => chooseYear(value ? Number(value) : undefined)}
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
            onChange={(value) =>
              chooseDegree(value as StudyEnrollment["stage"])
            }
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
