import { useState } from "react";
import { UiButton, UiInput } from "../../components/ui";
import { confirmAction } from "../../components/confirm";
import { COURSE_FILE_POLICY } from "../../lib/course-file-policy";
import type { Workspace } from "../../lib/model";
import { Row, SettingsGroup, SettingsHeader } from "../../components/forms";
import { CourseStorage } from "./CourseStorage";
import { t } from "../../i18n";
import { cleanWorkspaces } from "../../features/lms/assignment-workspace";

export function FilesSettings({
  data,
  update,
}: {
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
}) {
  return (
    <>
      <SettingsHeader title="settings.files.courseFiles" />
      <SettingsGroup>
        <CourseStorage />
        <Row
          id="auto-download"
          title="settings.files.downloadNewFilesAutomatically"
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
          id="workspaces"
          group
          title="settings.files.assignmentWorkspaces"
          hint={t("settings.files.keptUntilYouDeleteThem")}
        >
          <CleanWorkspaces />
        </Row>
      </SettingsGroup>
    </>
  );
}

/** Workspaces hold the student's own work, so cleanup is manual and only takes month-old ones. */
function CleanWorkspaces() {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState("");
  return (
    <span className="cleanWorkspaces">
      {result && <small role="status">{result}</small>}
      <UiButton
        className="button small"
        disabled={busy}
        onClick={async () => {
          if (
            !(await confirmAction(
              t("settings.files.deleteEveryWorkspaceUnchangedFor"),
              t("common.remove"),
            ))
          )
            return;
          setBusy(true);
          try {
            setResult(
              t("settings.files.removedCountWorkspaces", {
                count: await cleanWorkspaces(),
              }),
            );
          } catch (error) {
            setResult(String(error));
          } finally {
            setBusy(false);
          }
        }}
      >
        {t("settings.files.removeMonthOldWorkspaces")}
      </UiButton>
    </span>
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
