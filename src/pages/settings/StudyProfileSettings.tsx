import { useState } from "react";
import { t } from "../../i18n";
import { contentLanguage, pickLocalized, type Workspace } from "../../core/model";
import { SYLLABUS_DEPARTMENTS, departmentById } from "../../core/departments";
import { academicYearOf } from "../../core/calendar-view";
import { tokyoDate } from "../../core/schedule";
import { UiButton, UiSelect } from "../../components/mantine";
import {
  Row,
  SettingsGroup,
  SettingsHeader,
  Status,
} from "../../components/forms";
import { formatDay, subscriptionRow } from "./subscription-status";

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
  openSection: (section: "sources") => void;
}) {
  const locale = data.settings.locale;
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const department = departmentById(data.settings.departmentId);
  const admissionYear = data.academicProfile.admissionYear;
  const thisYear = academicYearOf(tokyoDate(new Date()));
  const years = Array.from({ length: 10 }, (_, index) => thisYear + 1 - index);
  if (admissionYear && !years.includes(admissionYear))
    years.push(admissionYear);

  const refresh = async () => {
    if (!refreshReferences) return;
    setBusy(true);
    setMessage("");
    try {
      setMessage(t(await refreshReferences()));
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  const guide = subscriptionRow(data, "departments");
  const recommended = data.recommendedCourses.filter(
    (row) =>
      !row.deletedAt &&
      !row.missingSince &&
      row.admissionYear === admissionYear &&
      (!department || departmentById(row.departmentId)?.id === department.id),
  ).length;
  const guideStatus = !admissionYear ? (
    <Status tone="idle">
      {t("Choose the year you started to load your study guide.")}
    </Status>
  ) : guide?.enabled === false ? (
    <Status
      tone="idle"
      action={
        <UiButton className="textButton" onClick={() => openSection("sources")}>
          {t("Data updates")}
        </UiButton>
      }
    >
      {t("Study guide updates are turned off.")}
    </Status>
  ) : guide?.lastError && guide.year === admissionYear ? (
    <Status
      tone="error"
      action={
        refreshReferences && (
          <UiButton className="textButton" disabled={busy} onClick={refresh}>
            {t("Try again")}
          </UiButton>
        )
      }
    >
      {t("Could not update the study guide.")} <small>{guide.lastError}</small>
    </Status>
  ) : recommended > 0 ? (
    <Status tone="ok">
      {t("{{year}} study guide · {{count}} listed courses", {
        year: admissionYear,
        count: recommended,
      })}
      {guide?.fetchedAt && guide.year === admissionYear && (
        <small>
          {" · "}
          {t("Updated {{date}}", { date: formatDay(guide.fetchedAt, locale) })}
        </small>
      )}
    </Status>
  ) : (
    <Status
      tone="warn"
      action={
        refreshReferences && (
          <UiButton className="textButton" disabled={busy} onClick={refresh}>
            {busy ? t("Checking…") : t("Check now")}
          </UiButton>
        )
      }
    >
      {department
        ? t("No study guide courses for this department and year yet.")
        : t("Choose your department to see its study guide.")}
    </Status>
  );

  return (
    <>
      <SettingsHeader
        title="Study profile"
        description="Used to suggest courses and to pick your study guide."
      />
      <SettingsGroup footer={message || undefined}>
        <Row
          id="department"
          title="Department"
          hint="The course finder filters and marks recommended courses with it"
        >
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
                {pickLocalized(item.name, contentLanguage(data.settings), item.id)}
              </option>
            ))}
          </UiSelect>
        </Row>
        <Row
          id="admission-year"
          title="Year you started"
          hint="Your study guide follows this year and stays the same as you move up"
        >
          <UiSelect
            value={admissionYear ?? ""}
            onChange={(event) => {
              const year = event.target.value
                ? Number(event.target.value)
                : undefined;
              setAdmissionYear(year);
              // The guide for another cohort is a separate download.
              if (year && year !== admissionYear) void refresh();
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
        {guideStatus}
      </SettingsGroup>
    </>
  );
}
