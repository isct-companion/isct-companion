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
      <SettingsHeader title="Developer options" />
      <SettingsGroup
        footer={
          developer
            ? t("New pages are listed under Developer in the sidebar.")
            : t(
                "Adds site styles and scripts, diagnostics, and raw data settings.",
              )
        }
      >
        <Row id="developer-mode" title="Developer options">
          <Toggle checked={developer} onChange={setDeveloper} />
        </Row>
      </SettingsGroup>
      {developer && desktop && (
        <>
          <SettingsGroup
            title="Academic profile"
            footer={t(
              "Custom class times and quarter dates apply to the timetable and calendar exports.",
            )}
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
                          "The academic profile changed elsewhere. Reload it before saving your edits.",
                        ),
                      );
                    }
                    return { ...draft, academicProfile: profile };
                  });
                  notify(t("Academic profile saved"));
                }}
              />
            </div>
          </SettingsGroup>
          <SettingsGroup
            title="Data source URLs"
            footer={t(
              "Each URL points to an index.json in the Isct-Data format.",
            )}
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
      setError(
        t(
          "The academic profile changed elsewhere. Reload it before saving your edits.",
        ),
      );
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
        {t(
          "Official class times and quarter dates are used by default. Change them here to match your program.",
        )}
      </p>
      <fieldset disabled={saving} className="profileFields">
        {raw !== null ? (
          <UiTextarea
            aria-label={t("Academic profile JSON")}
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
                <b>{t("Class times")}</b>
                <UiButton
                  className="textButton"
                  onClick={() =>
                    edit((p) => {
                      p.teachingPeriods = null;
                    })
                  }
                >
                  {t("Use official times")}
                </UiButton>
              </header>
              <p className="settingsNote">
                {t(
                  "Sets the clock times for periods 1–12. Courses scheduled by period follow this table; courses with explicit times keep their own times.",
                )}
              </p>
              <small>
                {draft.teachingPeriods
                  ? t("Custom times")
                  : t("Official times")}
              </small>
              <div className="profileTimes columnHeads" aria-hidden>
                <span>{t("Periods")}</span>
                <span>{t("Start time")}</span>
                <span>{t("End time")}</span>
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
                            ? "Periods {{periods}} start time"
                            : "Periods {{periods}} end time",
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
                <b>{t("Quarter starts")}</b>
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
                  ＋ {t("Add")}
                </UiButton>
              </header>
              <p className="settingsNote">
                {t("Quarter starts here override the official calendar.")}{" "}
                {t("Remove a year to use its official dates again.")}
              </p>
              {!draft.years.length && (
                <small>{t("Using official quarter dates")}</small>
              )}
              {draft.years.length > 0 && (
                <div className="profileLine quarters columnHeads" aria-hidden>
                  <span>{t("Year")}</span>
                  {quarters.map((quarter) => (
                    <span key={quarter}>{quarter}</span>
                  ))}
                  <span />
                </div>
              )}
              {draft.years.map((year, index) => (
                <div className="profileLine quarters" key={index}>
                  <UiInput
                    aria-label={t("Academic year")}
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
                    aria-label={t("Remove")}
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
          {t("Reload saved profile")}
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
          {raw === null ? t("Edit JSON") : t("Back to form")}
        </UiButton>
        <UiButton
          className="button primary"
          disabled={saving || !dirty}
          onClick={() => void commit()}
        >
          {t("Save")}
        </UiButton>
      </div>
    </div>
  );
}
