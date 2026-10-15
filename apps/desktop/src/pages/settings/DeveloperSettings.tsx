import { useState } from "react";
import { t } from "../../i18n";
import { UiButton, UiInput, UiTextarea } from "../../components/mantine";
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
  calendarUpdate: (f: (d: Workspace) => Workspace) => void;
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
            footer={t("Quarter starts here override the official calendar.")}
          >
            <div data-setting="profile">
              <ProfileEditor
                profile={data.academicProfile}
                save={(profile) => {
                  calendarUpdate((draft) => ({
                    ...draft,
                    academicProfile: profile,
                  }));
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
  save: (profile: AcademicProfile) => void;
}) {
  const [draft, setDraft] = useState<AcademicProfile>(() =>
    structuredClone(profile),
  );
  const [raw, setRaw] = useState<string | null>(null);
  const [error, setError] = useState("");
  const edit = (fn: (p: AcademicProfile) => void) =>
    setDraft((prior) => {
      const next = structuredClone(prior);
      fn(next);
      return next;
    });
  const validated = async () =>
    scheduleQuery<AcademicProfile>({
      kind: "validateAcademicProfile",
      profile: raw === null ? draft : JSON.parse(raw),
    });
  const commit = async () => {
    try {
      const value = await validated();
      setDraft(value);
      setRaw(null);
      setError("");
      save(value);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  const nextYear = (years: number[]) =>
    years.length
      ? Math.max(...years) + 1
      : Number(tokyoDate(new Date()).slice(0, 4));
  return (
    <div className="profileForm">
      {raw !== null ? (
        <UiTextarea
          aria-label={t("Academic profile JSON")}
          className="codeArea"
          spellCheck={false}
          value={raw}
          onChange={(event) => setRaw(event.target.value)}
        />
      ) : (
        <>
          <section>
            <header>
              <b>{t("Grade path")}</b>
              <UiButton
                className="button small"
                onClick={() =>
                  edit((p) =>
                    p.gradePath.push({
                      grade: "",
                      academicYear: nextYear(
                        p.gradePath.map((g) => g.academicYear),
                      ),
                    }),
                  )
                }
              >
                ＋ {t("Add")}
              </UiButton>
            </header>
            {draft.gradePath.map((entry, index) => (
              <div className="profileLine" key={index}>
                <UiInput
                  aria-label={t("Grade")}
                  placeholder="B1"
                  value={entry.grade}
                  onChange={(event) =>
                    edit((p) => (p.gradePath[index].grade = event.target.value))
                  }
                />
                <UiInput
                  aria-label={t("Academic year")}
                  type="number"
                  value={entry.academicYear}
                  onChange={(event) =>
                    edit(
                      (p) =>
                        (p.gradePath[index].academicYear = Number(
                          event.target.value,
                        )),
                    )
                  }
                />
                <UiButton
                  className="iconButton"
                  aria-label={t("Remove")}
                  onClick={() => edit((p) => p.gradePath.splice(index, 1))}
                >
                  ×
                </UiButton>
              </div>
            ))}
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
                      quarterStarts: { "1Q": "", "2Q": "", "3Q": "", "4Q": "" },
                    });
                  })
                }
              >
                ＋ {t("Add")}
              </UiButton>
            </header>
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
      {error && (
        <p className="formError" role="alert">
          {error}
        </p>
      )}
      <div className="actions end">
        <UiButton
          className="textButton"
          onClick={async () => {
            setError("");
            if (raw === null) return setRaw(JSON.stringify(draft, null, 2));
            try {
              setDraft(
                await scheduleQuery<AcademicProfile>({
                  kind: "validateAcademicProfile",
                  profile: JSON.parse(raw),
                }),
              );
              setRaw(null);
            } catch (e) {
              setError(e instanceof Error ? e.message : String(e));
            }
          }}
        >
          {raw === null ? t("Edit JSON") : t("Back to form")}
        </UiButton>
        <UiButton className="button primary" onClick={() => void commit()}>
          {t("Save")}
        </UiButton>
      </div>
    </div>
  );
}
