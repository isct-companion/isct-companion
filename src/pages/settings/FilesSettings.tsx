import { useState } from "react";
import { t } from "../../i18n";
import { UiInput } from "../../components/mantine";
import { COURSE_FILE_POLICY } from "../../core/course-file-policy";
import type { Workspace } from "../../core/model";
import {
  Row,
  SettingsGroup,
  SettingsHeader,
  Toggle,
} from "../../components/forms";
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
      <SettingsHeader
        title="Course files"
        description="Where course materials are saved, and how much downloads by itself."
      />
      <SettingsGroup>
        <CourseStorage />
        <Row
          id="auto-download"
          title="Download new files automatically"
          hint="Files up to this size. Larger ones download when you open them."
        >
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
          id="cleanup"
          title="Remove unused downloads"
          hint="After this many days without opening them. Your notes are kept."
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
      </SettingsGroup>
      <SettingsGroup title="Reading">
        <Row
          id="bilingual"
          title="Show one language in bilingual LMS"
          hint="When a page has both Japanese and English, show only the app language"
        >
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
      </SettingsGroup>
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
