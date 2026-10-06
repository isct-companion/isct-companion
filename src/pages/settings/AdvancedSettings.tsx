import { CourseStorage } from "./CourseStorage";
import { t } from "../../i18n";
import {
  UiButton,
  UiInput,
  UiTextarea,
} from "../../components/mantine";
import { useState } from "react";
import { Badge } from "@mantine/core";
import { COURSE_FILE_POLICY } from "../../core/course-file-policy";

import {
  customizationNames,
  desktopBridge,
  type CustomizationSite,
} from "../../app/native";

import type {
  Course,
  PersonalEvent,
  TermId,
  Workspace,
} from "../../core/model";

import { Row, Toggle } from "../../components/forms";

export function AdvancedSettings({
  data,
  update,
}: {
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
}) {
  const custom = data.settings.customization!;
  const [consentRevision, setConsentRevision] = useState(0);
  const applyWebPolicy = (restore: boolean) =>
    desktopBridge?.configureSchoolWebPolicy({
      restoreAcrossRestarts: restore,
      idleMinutes: 10,
    });
  void consentRevision;
  return (
    <>
      <h2>LMS</h2>
      <CourseStorage />
      <Row
        title={t("Download previews automatically up to")}
        hint={t(
          "Larger files ask first. Previews always work; this only skips the automatic download.",
        )}
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
        title={t("Remove downloaded files unused for")}
        hint={t("Only downloaded course files. Notes are never removed.")}
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
      <Row
        title={t("Prefer one language in bilingual LMS content")}
        hint={t(
          "Only clearly separated Japanese and English blocks are filtered. Original content is always available.",
        )}
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
      <h2>{t("Customize")}</h2>
      <Row title={t("Background image URL")}>
        <UiInput
          value={custom.background}
          placeholder="https://…"
          onChange={(event) =>
            update((d) => {
              d.settings.customization!.background = event.target.value;
              return d;
            })
          }
        />
      </Row>
      <h3>{t("School sites")}</h3>
      {(Object.keys(custom.sites) as CustomizationSite[]).map((site) => {
        const enabled = data.settings.siteConsents?.[site] ?? false;
        return (
          <details className="siteEditor" key={site}>
            <summary>
              <b>{customizationNames[site]}</b>
              <Badge variant="light" color={enabled ? "green" : "gray"}>
                {t(enabled ? "enabled" : "off")}
              </Badge>
            </summary>
            <div className="actions">
              <UiButton
                className="button small"
                disabled={!desktopBridge}
                onClick={() => {
                  update((draft) => {
                    draft.settings.siteConsents ??= { portal: false, lms: false, syllabus: false };
                    draft.settings.siteConsents[site] = !enabled;
                    return draft;
                  });
                  setConsentRevision((value) => value + 1);
                }}
              >
                {t(
                  enabled ? "Disable on this device" : "Enable on this device",
                )}
              </UiButton>
              <small>{t("Runs only inside that site's sandboxed view.")}</small>
            </div>
            <UiTextarea
              aria-label={`${site} CSS`}
              placeholder="CSS"
              spellCheck={false}
              value={custom.sites[site].css}
              onChange={(event) =>
                update((d) => {
                  d.settings.customization!.sites[site].css =
                    event.target.value;
                  return d;
                })
              }
            />
            <UiTextarea
              aria-label={`${site} JavaScript`}
              placeholder="JavaScript"
              spellCheck={false}
              value={custom.sites[site].js}
              onChange={(event) =>
                update((d) => {
                  d.settings.customization!.sites[site].js = event.target.value;
                  return d;
                })
              }
            />
          </details>
        );
      })}
      <h3>{t("Web page memory")}</h3>
      <Row title={t("Restore last page after restart")}>
        <Toggle
          disabled={!desktopBridge}
          checked={data.settings.restoreSchoolPage ?? false}
          onChange={(enabled) => {
            update((draft) => {
              draft.settings.restoreSchoolPage = enabled;
              if (!enabled) draft.settings.activeSchoolSite = undefined;
              return draft;
            });
            void applyWebPolicy(enabled);
          }}
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
