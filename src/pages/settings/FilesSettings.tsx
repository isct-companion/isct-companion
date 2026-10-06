import { useState } from "react";
import { t } from "../../i18n";
import { UiInput } from "../../components/mantine";
import { COURSE_FILE_POLICY } from "../../core/course-file-policy";
import type { Workspace } from "../../core/model";
import { Row, Toggle } from "../../components/forms";
import { CourseStorage } from "./CourseStorage";

export function FilesSettings({
  data,
  update,
}: {
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
}) {
  return (
    <>
      <h2>{t("Files")}</h2>
      <CourseStorage />
      <Row title={t("Auto-download files up to")}>
        <NumberSetting
          value={
            data.settings.autoDownloadLimitMb ??
            COURSE_FILE_POLICY.autoDownloadLimitMb.default
          }
          min={COURSE_FILE_POLICY.autoDownloadLimitMb.min}
          max={COURSE_FILE_POLICY.autoDownloadLimitMb.max}
          unit="MB"
          onChange={(value) =>
            update((d) => {
              d.settings.autoDownloadLimitMb = value;
              return d;
            })
          }
        />
      </Row>
      <Row
        title={t("Remove unused downloads after")}
        hint={t("Notes are kept.")}
      >
        <NumberSetting
          value={
            data.settings.autoCleanDays ??
            COURSE_FILE_POLICY.retentionDays.default
          }
          min={COURSE_FILE_POLICY.retentionDays.min}
          max={COURSE_FILE_POLICY.retentionDays.max}
          unit={t("days")}
          onChange={(value) =>
            update((d) => {
              d.settings.autoCleanDays = value;
              return d;
            })
          }
        />
      </Row>
      <Row title={t("Show one language in bilingual LMS")}>
        <Toggle
          checked={data.settings.lmsBilingualFilter ?? false}
          onChange={(checked) =>
            update((d) => {
              d.settings.lmsBilingualFilter = checked;
              return d;
            })
          }
        />
      </Row>
    </>
  );
}

/** Whole-number setting; out-of-range input is clamped on blur. */
function NumberSetting({
  value,
  min,
  max,
  unit,
  onChange,
}: {
  value: number;
  min: number;
  max: number;
  unit: string;
  onChange: (value: number) => void;
}) {
  const [draft, setDraft] = useState(String(value));
  const commit = () => {
    const next = Math.min(max, Math.max(min, Math.round(Number(draft)) || min));
    setDraft(String(next));
    if (next !== value) onChange(next);
  };
  return (
    <span className="numberSetting">
      <UiInput
        type="number"
        min={min}
        max={max}
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") commit();
        }}
      />
      <span>{unit}</span>
    </span>
  );
}
