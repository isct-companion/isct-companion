import { t } from "../../i18n";
import {
  contentLanguage,
  pickLocalized,
  type Workspace,
} from "../../core/model";
import { SYLLABUS_DEPARTMENTS, departmentById } from "../../core/departments";
import { UiButton, UiSelect } from "../../components/mantine";
import { Row, SettingsGroup, SettingsHeader } from "../../components/forms";
import { jstDate } from "../../app/shared";
import {
  useScheduleQuery,
  type AcademicYearIndex,
} from "../../platform/core-query";

export function StudyProfileSettings({
  data,
  update,
  setAdmissionYear,
  refreshReferences,
  openSection,
}: {
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
  setAdmissionYear: (year: number | undefined) => void;
  refreshReferences?: () => Promise<string>;
  /** Opens the academic calendar; absent where it is not available. */
  openSection?: (section: "calendar") => void;
}) {
  const department = departmentById(data.settings.departmentId);
  const admissionYear = data.academicProfile.admissionYear;
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
        <Row id="admission-year" title="Year you started">
          <UiSelect
            value={admissionYear ?? ""}
            onChange={(event) => {
              const year = event.target.value
                ? Number(event.target.value)
                : undefined;
              setAdmissionYear(year);
              // The guide for another cohort is a separate download.
              if (year && year !== admissionYear)
                void refreshReferences?.().catch(() => undefined);
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
