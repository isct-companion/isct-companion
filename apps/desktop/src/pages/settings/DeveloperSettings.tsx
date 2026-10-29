import { useEffect, useState } from "react";
import { t } from "../../i18n";
import { UiButton, UiInput, UiTextarea } from "../../components/ui";
import { DEFAULT_TEACHING_PERIODS } from "../../generated/persistence";
import type { AcademicProfile, Workspace } from "../../lib/model";
import { scheduleQuery } from "../../platform/core-query";
import { tokyoDate } from "../../lib/civil-date";
import {
  Row,
  SettingsGroup,
  SettingsHeader,
  Toggle,
} from "../../components/forms";
import { DataSourceUrls } from "./DataUpdatesSettings";

const DEVELOPER_KEY = "isct-developer-mode";
const quarters = ["1Q", "2Q", "3Q", "4Q"] as const;

/** Per-device switch; storage can be unavailable, so default to off. */
export function useDeveloperMode() {
  const [on, setOn] = useState(() => {
    try {
      return localStorage.getItem(DEVELOPER_KEY) === "1";
    } catch {
      return false;
    }
  });
  const set = (value: boolean) => {
    setOn(value);
    try {
      if (value) localStorage.setItem(DEVELOPER_KEY, "1");
      else localStorage.removeItem(DEVELOPER_KEY);
    } catch {
      /* Session-only when storage is blocked. */
    }
  };
  return [on, set] as const;
}

export function DeveloperSettings({
  data,
  update,
  calendarUpdate,
  developer,
  setDeveloper,
  notify,
  desktop,
}: {
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
  calendarUpdate: (f: (d: Workspace) => Workspace) => Promise<unknown>;
  developer: boolean;
  setDeveloper: (value: boolean) => void;
  notify: (message: string) => void;
  desktop: boolean;
}) {
  return (
    <>
      <SettingsHeader title="settings.developer.developerOptions" />
      <SettingsGroup
        footer={
          developer
            ? t("settings.developer.newPagesAreListedUnder")
            : t("settings.developer.addsSiteStylesAndScripts")
        }
      >
        <Row id="developer-mode" title="settings.developer.developerOptions">
          <Toggle checked={developer} onChange={setDeveloper} />
        </Row>
      </SettingsGroup>
      {developer && desktop && (
        <>
          <SettingsGroup
            title="settings.developer.academicProfile"
            footer={t("settings.developer.customClassTimesAndQuarter")}
          >
            <div data-setting="profile">
              <ProfileEditor
                profile={data.academicProfile}
                save={async (profile) => {
                  await calendarUpdate((draft) => {
                    if (
                      JSON.stringify(draft.academicProfile) !==
                      JSON.stringify(data.academicProfile)
                    ) {
                      throw new Error(
                        t(
                          "settings.developer.theAcademicProfileChangedElsewhere",
                        ),
                      );
                    }
                    return { ...draft, academicProfile: profile };
                  });
                  notify(t("settings.developer.academicProfileSaved"));
                }}
              />
            </div>
          </SettingsGroup>
          <SettingsGroup
            title="settings.developer.dataSourceURLs"
            footer={t("settings.developer.eachURLPointsToAn")}
          >
            <div data-setting="sources">
              <DataSourceUrls data={data} />
            </div>
          </SettingsGroup>
        </>
      )}
    </>
  );
}

function ProfileEditor({
  profile,
  save,
}: {
  profile: AcademicProfile;
  save: (profile: AcademicProfile) => Promise<void>;
}) {
  const [draft, setDraft] = useState<AcademicProfile>(() =>
    structuredClone(profile),
  );
  const [baseline, setBaseline] = useState(() => JSON.stringify(profile));
  const [saving, setSaving] = useState(false);
  const [dirty, setDirty] = useState(false);
  useEffect(() => {
    if (dirty || saving) return;
    setDraft(structuredClone(profile));
    setBaseline(JSON.stringify(profile));
    setRaw(null);
  }, [profile, dirty, saving]);
  const [raw, setRaw] = useState<string | null>(null);
  const [error, setError] = useState("");
  const edit = (fn: (p: AcademicProfile) => void) => {
    setDirty(true);
    setDraft((prior) => {
      const next = structuredClone(prior);
      fn(next);
      return next;
    });
  };
  const validated = async () =>
    scheduleQuery<AcademicProfile>({
      kind: "validateAcademicProfile",
      profile: raw === null ? draft : JSON.parse(raw),
    });
  const commit = async () => {
    if (saving) return;
    if (baseline !== JSON.stringify(profile)) {
      setError(t("settings.developer.theAcademicProfileChangedElsewhere"));
      return;
    }
    setSaving(true);
    try {
      const value = await validated();
      await save(value);
      setDraft(value);
      setRaw(null);
      setError("");
      setDirty(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };
  const nextYear = (years: number[]) =>
    years.length
      ? Math.max(...years) + 1
      : Number(tokyoDate(new Date()).slice(0, 4));
  return (
    <div className="profileForm">
      <p className="settingsNote">
        {t("settings.developer.officialClassTimesAndQuarter")}
      </p>
      <fieldset disabled={saving} className="profileFields">
        {raw !== null ? (
          <UiTextarea
            aria-label={t("settings.developer.academicProfileJSON")}
            className="codeArea"
            spellCheck={false}
            value={raw}
            onChange={(event) => {
              setRaw(event.target.value);
              setDirty(true);
            }}
          />
        ) : (
          <>
            <section data-setting="teaching-periods">
              <header>
                <b>{t("settings.developer.classTimes")}</b>
                <UiButton
                  className="textButton"
                  onClick={() =>
                    edit((p) => {
                      p.teachingPeriods = null;
                    })
                  }
                >
                  {t("settings.developer.useOfficialTimes")}
                </UiButton>
              </header>
              <p className="settingsNote">
                {t("settings.developer.setsTheClockTimesFor")}
              </p>
              <small>
                {draft.teachingPeriods
                  ? t("settings.developer.customTimes")
                  : t("settings.developer.officialTimes")}
              </small>
              <div className="profileTimes columnHeads" aria-hidden>
                <span>{t("settings.developer.periods")}</span>
                <span>{t("common.startTime")}</span>
                <span>{t("common.endTime")}</span>
              </div>
              {(draft.teachingPeriods ?? DEFAULT_TEACHING_PERIODS).map(
                (slot, index) => (
                  <div className="profileTimes" key={slot.periods[0]}>
                    <b>{slot.periods.join("–")}</b>
                    {(["startTime", "endTime"] as const).map((field) => (
                      <UiInput
                        key={field}
                        type="time"
                        value={slot[field]}
                        aria-label={t(
                          field === "startTime"
                            ? "settings.developer.periodsStartTime"
                            : "settings.developer.periodsEndTime",
                          { periods: slot.periods.join("–") },
                        )}
                        onChange={(event) =>
                          edit((p) => {
                            p.teachingPeriods ??= structuredClone(
                              DEFAULT_TEACHING_PERIODS,
                            );
                            p.teachingPeriods[index][field] =
                              event.target.value;
                          })
                        }
                      />
                    ))}
                  </div>
                ),
              )}
            </section>
            <section>
              <header>
                <b>{t("settings.developer.quarterStarts")}</b>
                <UiButton
                  className="button small"
                  onClick={() =>
                    edit((p) => {
                      const year = nextYear(p.years.map((y) => y.academicYear));
                      p.years.push({
                        academicYear: year,
                        quarterStarts: {
                          "1Q": "",
                          "2Q": "",
                          "3Q": "",
                          "4Q": "",
                        },
                      });
                    })
                  }
                >
                  ＋ {t("common.add")}
                </UiButton>
              </header>
              <p className="settingsNote">
                {t("settings.developer.quarterStartsHereOverrideThe")}{" "}
                {t("settings.developer.removeAYearToUse")}
              </p>
              {!draft.years.length && (
                <small>
                  {t("settings.developer.usingOfficialQuarterDates")}
                </small>
              )}
              {draft.years.length > 0 && (
                <div className="profileLine quarters columnHeads" aria-hidden>
                  <span>{t("settings.developer.year")}</span>
                  {quarters.map((quarter) => (
                    <span key={quarter}>{quarter}</span>
                  ))}
                  <span />
                </div>
              )}
              {draft.years.map((year, index) => (
                <div className="profileLine quarters" key={index}>
                  <UiInput
                    aria-label={t("common.academicYear")}
                    type="number"
                    value={year.academicYear}
                    onChange={(event) =>
                      edit(
                        (p) =>
                          (p.years[index].academicYear = Number(
                            event.target.value,
                          )),
                      )
                    }
                  />
                  {quarters.map((quarter) => (
                    <UiInput
                      key={quarter}
                      aria-label={quarter}
                      type="date"
                      value={year.quarterStarts[quarter]}
                      onChange={(event) =>
                        edit(
                          (p) =>
                            (p.years[index].quarterStarts[quarter] =
                              event.target.value),
                        )
                      }
                    />
                  ))}
                  <UiButton
                    className="iconButton"
                    aria-label={t("common.remove")}
                    onClick={() => edit((p) => p.years.splice(index, 1))}
                  >
                    ×
                  </UiButton>
                </div>
              ))}
            </section>
          </>
        )}
      </fieldset>
      {error && (
        <p className="formError" role="alert">
          {error}
        </p>
      )}
      <div className="actions end">
        <UiButton
          className="textButton"
          disabled={saving}
          onClick={() => {
            setDraft(structuredClone(profile));
            setRaw(null);
            setDirty(false);
            setBaseline(JSON.stringify(profile));
            setError("");
          }}
        >
          {t("settings.developer.reloadSavedProfile")}
        </UiButton>
        <UiButton
          className="textButton"
          disabled={saving}
          onClick={async () => {
            setError("");
            if (raw === null) return setRaw(JSON.stringify(draft, null, 2));
            try {
              setDraft(await validated());
              setRaw(null);
            } catch (e) {
              setError(e instanceof Error ? e.message : String(e));
            }
          }}
        >
          {raw === null
            ? t("settings.developer.editJSON")
            : t("settings.developer.backToForm")}
        </UiButton>
        <UiButton
          className="button primary"
          disabled={saving || !dirty}
          onClick={() => void commit()}
        >
          {t("common.save")}
        </UiButton>
      </div>
    </div>
  );
}
