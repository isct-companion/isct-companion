import { CourseStorage } from "./CourseStorage";
import { t } from "../../i18n";
import {
  UiButton,
  UiInput,
  UiSelect,
  UiTextarea,
} from "../../components/mantine";
import { useState } from "react";
import { Badge } from "@mantine/core";

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
  const [schoolSchedule, setSchoolSchedule] = useState(
    () => localStorage.getItem("stws-hook-school") || "off",
  );
  const [exportSchedule, setExportSchedule] = useState(
    () => localStorage.getItem("stws-hook-export") || "off",
  );
  const [restoreSchoolPage, setRestoreSchoolPage] = useState(
    () => localStorage.getItem("stws-restore-school-page") === "true",
  );
  const applyWebPolicy = (restore: boolean) =>
    desktopBridge?.configureSchoolWebPolicy({
      restoreAcrossRestarts: restore,
      idleMinutes: 10,
    });
  const applySchedule = async (
    kind: "school" | "export",
    value: "off" | "launch" | "hourly",
  ) => {
    localStorage.setItem(`stws-hook-${kind}`, value);
    (kind === "school" ? setSchoolSchedule : setExportSchedule)(value);
    const other = kind === "school" ? exportSchedule : schoolSchedule;
    try {
      const acknowledged =
        kind === "school"
          ? await desktopBridge?.configureSchoolSchedule(value)
          : await desktopBridge?.configureExportSchedule(value);
      dispatchEvent(
        new CustomEvent("stws-scheduler-state", {
          detail: !acknowledged
            ? "failed"
            : value !== "off" || other !== "off"
              ? "active"
              : "off",
        }),
      );
    } catch {
      dispatchEvent(
        new CustomEvent("stws-scheduler-state", { detail: "failed" }),
      );
    }
  };
  void consentRevision;
  return (
    <>
      <h2>LMS</h2>
      <CourseStorage />
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
        const key = `stws-site-consent-${site}`;
        const enabled = localStorage.getItem(key) === "true";
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
                  localStorage.setItem(key, String(!enabled));
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
          checked={restoreSchoolPage}
          onChange={(enabled) => {
            localStorage.setItem("stws-restore-school-page", String(enabled));
            if (!enabled) localStorage.removeItem("stws-active-school-site");
            setRestoreSchoolPage(enabled);
            void applyWebPolicy(enabled);
          }}
        />
      </Row>
      <h3>{t("Schedules (while the app is open)")}</h3>
      <Row title={t("School refresh")}>
        <UiSelect
          disabled={!desktopBridge}
          value={schoolSchedule}
          onChange={(e) =>
            void applySchedule(
              "school",
              e.target.value as "off" | "launch" | "hourly",
            )
          }
        >
          <option value="off">{t("Off")}</option>
          <option value="launch">{t("On launch")}</option>
          <option value="hourly">{t("Hourly")}</option>
        </UiSelect>
      </Row>
      <Row
        title={t("Local export")}
        hint={t("latest-workspace.json in app data")}
      >
        <UiSelect
          disabled={!desktopBridge}
          value={exportSchedule}
          onChange={(e) =>
            void applySchedule(
              "export",
              e.target.value as "off" | "launch" | "hourly",
            )
          }
        >
          <option value="off">{t("Off")}</option>
          <option value="launch">{t("On launch")}</option>
          <option value="hourly">{t("Hourly")}</option>
        </UiSelect>
      </Row>
    </>
  );
}
