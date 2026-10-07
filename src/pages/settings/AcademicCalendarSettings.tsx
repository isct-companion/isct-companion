import { useRef, useState, type ChangeEvent, type FormEvent } from "react";
import { Badge } from "@mantine/core";
import { t } from "../../i18n";
import {
  pickLocalized,
  type CalendarException,
  type Workspace,
} from "../../core/model";
import { validateAcademicProfile } from "../../core/academic-calendar";
import {
  academicYearCalendar,
  academicYearOf,
  type CalendarDayView,
  type CalendarOrigin,
} from "../../core/calendar-view";
import { subscriptionYear } from "../../platform/subscription-polling";
import { newUserRecordId } from "../../storage/ids";
import { UiButton, UiInput, UiSelect } from "../../components/mantine";
import {
  MoreOptions,
  SettingsGroup,
  SettingsHeader,
  Status,
} from "../../components/forms";
import {
  formatDay,
  languageTag,
  subscriptionState,
} from "./subscription-status";

type Draft = Pick<
  CalendarException,
  "date" | "type" | "replacementDay" | "label"
>;
type Editing = { id: string | "new"; draft: Draft; fixedDate?: boolean };
const blank: Draft = { date: "", type: "no-class", label: "" };

function isCalendarDate(value: string) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return false;
  const [year, month, day] = value.split("-").map(Number);
  const parsed = new Date(Date.UTC(year, month - 1, day));
  return (
    parsed.getUTCFullYear() === year &&
    parsed.getUTCMonth() === month - 1 &&
    parsed.getUTCDate() === day
  );
}

/** Personal overrides file: `{ schema, version, exceptions, profile? }`. */
function parseOverrides(value: unknown): Draft[] {
  const root = value as {
    schema?: unknown;
    version?: unknown;
    exceptions?: unknown;
  };
  if (!root || !Array.isArray(root.exceptions))
    throw new Error(t("This file has no calendar changes."));
  if (
    (root.schema !== undefined && root.schema !== "isct-academic-calendar") ||
    (root.version !== undefined && root.version !== 1)
  )
    throw new Error(t("This calendar file format is not supported."));
  const dates = new Set<string>();
  return root.exceptions.map((raw, index) => {
    const item = (raw ?? {}) as Record<string, unknown>;
    const date = String(item.date ?? "");
    const type = String(item.type);
    const replacementDay =
      type === "replacement" ? Number(item.replacementDay) : undefined;
    const label = typeof item.label === "string" ? item.label.trim() : "";
    if (
      !isCalendarDate(date) ||
      dates.has(date) ||
      !["no-class", "replacement", "break"].includes(type) ||
      (type === "replacement" &&
        (!Number.isInteger(replacementDay) ||
          replacementDay! < 1 ||
          replacementDay! > 7)) ||
      !label
    )
      throw new Error(
        t("Row {{row}} of the file is not valid.", { row: index + 1 }),
      );
    dates.add(date);
    return { date, type: type as Draft["type"], replacementDay, label };
  });
}

function weekdayName(day: number, locale: Workspace["settings"]["locale"]) {
  // 2024-01-01 was a Monday; day 1 = Monday … 7 = Sunday.
  return new Intl.DateTimeFormat(languageTag(locale), {
    weekday: "long",
    timeZone: "UTC",
  }).format(new Date(Date.UTC(2024, 0, day)));
}

function describe(
  day: { type: string; replacementDay?: number },
  locale: Workspace["settings"]["locale"],
) {
  if (day.type === "replacement")
    return t("{{weekday}} timetable", {
      weekday: weekdayName(day.replacementDay || 1, locale),
    });
  if (day.type === "holiday") return t("Holiday");
  if (day.type === "break") return t("Break");
  return t("No classes");
}

const ORIGIN: Record<CalendarOrigin, { label: string; color: string }> = {
  official: { label: "Official", color: "blue" },
  changed: { label: "Changed by you", color: "orange" },
  own: { label: "Added by you", color: "grape" },
  estimated: { label: "Estimated", color: "gray" },
};

function OriginBadge({ origin }: { origin: CalendarOrigin }) {
  return (
    <Badge size="sm" variant="light" tt="none" color={ORIGIN[origin].color}>
      {t(ORIGIN[origin].label)}
    </Badge>
  );
}

export function AcademicCalendarSettings({
  data,
  update,
  notify,
  readOnly,
  refreshReferences,
  openSection,
}: {
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
  notify: (message: string) => void;
  readOnly: boolean;
  refreshReferences?: () => Promise<string>;
  openSection: (section: "sources") => void;
}) {
  const locale = data.settings.locale;
  const followedYear =
    subscriptionYear(data, "calendar") ?? Number(data.selectedTerm.slice(0, 4));
  const [year, setYear] = useState(followedYear);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [busy, setBusy] = useState(false);
  const file = useRef<HTMLInputElement>(null);
  const view = academicYearCalendar(data, year);
  const years = [
    ...new Set([
      followedYear,
      year,
      ...data.academicCalendar
        .filter((row) => !row.deletedAt)
        .map((row) => row.academicYear ?? academicYearOf(row.date)),
      ...data.exceptions
        .filter((row) => !row.deletedAt)
        .map((row) => academicYearOf(row.date)),
    ]),
  ].sort((a, b) => b - a);

  const checkNow = async () => {
    if (!refreshReferences) return;
    setBusy(true);
    try {
      notify(t(await refreshReferences()));
    } catch (error) {
      notify(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };
  const checkButton = refreshReferences && year === followedYear && (
    <UiButton className="textButton" disabled={busy} onClick={checkNow}>
      {busy ? t("Checking…") : t("Check now")}
    </UiButton>
  );
  const subscription = subscriptionState(data, "calendar");
  const status =
    year !== followedYear ? (
      <Status tone={view.published ? "ok" : "idle"}>
        {view.published
          ? t("Official calendar for {{year}}", { year })
          : t("No official calendar saved for {{year}}.", { year })}
      </Status>
    ) : subscription.state === "off" ? (
      <Status
        tone="idle"
        action={
          <UiButton
            className="textButton"
            onClick={() => openSection("sources")}
          >
            {t("Data updates")}
          </UiButton>
        }
      >
        {t("Official calendar updates are turned off.")}
      </Status>
    ) : subscription.state === "error" ? (
      <Status tone="error" action={checkButton}>
        {t("Could not update the official calendar.")}{" "}
        <small>{subscription.message}</small>
      </Status>
    ) : view.published ? (
      <Status tone="ok" action={checkButton}>
        {t("Official calendar for {{year}}", { year })}
        {subscription.state === "current" && (
          <small>
            {" · "}
            {t("Updated {{date}}", {
              date: formatDay(subscription.fetchedAt, locale),
            })}
          </small>
        )}
      </Status>
    ) : (
      <Status tone="warn" action={checkButton}>
        {subscription.state === "current"
          ? t("The official calendar for {{year}} is not published yet.", {
              year,
            })
          : t("The official calendar has not been downloaded yet.")}
      </Status>
    );

  const own = data.exceptions.filter((row) => !row.deletedAt);
  const save = async (event: FormEvent) => {
    event.preventDefault();
    if (readOnly || !editing) return;
    const { draft } = editing;
    if (
      !isCalendarDate(draft.date) ||
      !draft.label.trim() ||
      (draft.type === "replacement" && !draft.replacementDay)
    )
      return notify(t("Fill in the date, type and name."));
    const existing = own.find((row) => row.date === draft.date);
    if (existing && existing.id !== editing.id)
      return notify(t("That date already has a change. Edit it instead."));
    const now = new Date().toISOString();
    const id = editing.id === "new" ? await newUserRecordId() : editing.id;
    const values = {
      ...draft,
      label: draft.label.trim(),
      replacementDay:
        draft.type === "replacement" ? draft.replacementDay : undefined,
    };
    update((workspace) => ({
      ...workspace,
      exceptions:
        editing.id === "new"
          ? [
              ...workspace.exceptions,
              {
                ...values,
                id,
                rev: 1,
                updatedAt: now,
                updatedBy: workspace.deviceId,
              },
            ]
          : workspace.exceptions.map((row) =>
              row.id === id
                ? {
                    ...row,
                    ...values,
                    rev: row.rev + 1,
                    updatedAt: now,
                    updatedBy: workspace.deviceId,
                  }
                : row,
            ),
    }));
    setEditing(null);
    notify(t("Calendar saved"));
  };
  const remove = (id: string) => {
    if (readOnly) return;
    const now = new Date().toISOString();
    update((workspace) => ({
      ...workspace,
      exceptions: workspace.exceptions.map((row) =>
        row.id === id
          ? {
              ...row,
              deletedAt: now,
              rev: row.rev + 1,
              updatedAt: now,
              updatedBy: workspace.deviceId,
            }
          : row,
      ),
    }));
    setEditing(null);
  };
  const beginEdit = (day: CalendarDayView) =>
    setEditing(
      day.exception
        ? {
            id: day.exception.id,
            fixedDate: day.origin === "changed",
            draft: {
              date: day.date,
              type: day.exception.type,
              replacementDay: day.exception.replacementDay,
              label: day.exception.label,
            },
          }
        : {
            id: "new",
            fixedDate: true,
            draft: {
              date: day.date,
              type: day.type === "holiday" ? "no-class" : day.type,
              replacementDay: day.replacementDay,
              label: day.label,
            },
          },
    );

  const importFile = async (event: ChangeEvent<HTMLInputElement>) => {
    const selected = event.target.files?.[0];
    event.target.value = "";
    if (!selected || readOnly) return;
    try {
      const parsed = JSON.parse(await selected.text()) as { profile?: unknown };
      const incoming = parseOverrides(parsed);
      const profile =
        parsed.profile === undefined
          ? undefined
          : validateAcademicProfile(parsed.profile);
      const now = new Date().toISOString();
      const ids = await Promise.all(incoming.map(() => newUserRecordId()));
      update((workspace) => ({
        ...workspace,
        academicProfile: profile ?? workspace.academicProfile,
        exceptions: [
          ...workspace.exceptions.filter(
            (row) =>
              row.deletedAt || !incoming.some((next) => next.date === row.date),
          ),
          ...incoming.map((row, index) => ({
            ...row,
            id: ids[index],
            rev: 1,
            updatedAt: now,
            updatedBy: workspace.deviceId,
          })),
        ],
      }));
      notify(
        t("{{count}} calendar changes imported", { count: incoming.length }),
      );
    } catch (error) {
      notify(error instanceof Error ? error.message : t("Import failed."));
    }
  };
  const exportFile = () => {
    const payload = {
      schema: "isct-academic-calendar",
      version: 1,
      exportedAt: new Date().toISOString(),
      profile: data.academicProfile,
      exceptions: own
        .sort((a, b) => a.date.localeCompare(b.date))
        .map(({ date, type, replacementDay, label }) => ({
          date,
          type,
          ...(replacementDay ? { replacementDay } : {}),
          label,
        })),
    };
    const url = URL.createObjectURL(
      new Blob([JSON.stringify(payload, null, 2)], {
        type: "application/json",
      }),
    );
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = "academic-calendar-changes.json";
    anchor.click();
    URL.revokeObjectURL(url);
  };

  const editor = editing && !readOnly && (
    <form className="calendarEditor" onSubmit={save}>
      <label>
        {t("Date")}
        <UiInput
          required
          type="date"
          disabled={editing.fixedDate}
          value={editing.draft.date}
          onChange={(event) =>
            setEditing({
              ...editing,
              draft: { ...editing.draft, date: event.target.value },
            })
          }
        />
      </label>
      <label>
        {t("What happens")}
        <UiSelect
          value={editing.draft.type}
          onChange={(event) => {
            const type = event.target.value as Draft["type"];
            setEditing({
              ...editing,
              draft: {
                ...editing.draft,
                type,
                replacementDay:
                  type === "replacement"
                    ? editing.draft.replacementDay || 1
                    : undefined,
              },
            });
          }}
        >
          <option value="no-class">{t("No classes")}</option>
          <option value="replacement">{t("Another day's timetable")}</option>
          <option value="break">{t("Break")}</option>
        </UiSelect>
      </label>
      {editing.draft.type === "replacement" && (
        <label>
          {t("Use the timetable of")}
          <UiSelect
            value={editing.draft.replacementDay}
            onChange={(event) =>
              setEditing({
                ...editing,
                draft: {
                  ...editing.draft,
                  replacementDay: Number(event.target.value),
                },
              })
            }
          >
            {[1, 2, 3, 4, 5, 6].map((day) => (
              <option key={day} value={day}>
                {weekdayName(day, locale)}
              </option>
            ))}
          </UiSelect>
        </label>
      )}
      <label className="wide">
        {t("Name")}
        <UiInput
          required
          value={editing.draft.label}
          placeholder={t("e.g. Foundation Day")}
          onChange={(event) =>
            setEditing({
              ...editing,
              draft: { ...editing.draft, label: event.target.value },
            })
          }
        />
      </label>
      <div className="actions end wide">
        <UiButton
          className="button"
          type="button"
          onClick={() => setEditing(null)}
        >
          {t("Cancel")}
        </UiButton>
        <UiButton className="button primary" type="submit">
          {t("Save")}
        </UiButton>
      </div>
    </form>
  );

  return (
    <>
      <SettingsHeader
        title="Academic calendar"
        description="Holidays, make-up days and quarter dates. Official dates update by themselves; your own changes always win."
        actions={
          years.length > 1 && (
            <UiSelect
              aria-label={t("Academic year")}
              value={year}
              onChange={(event) => {
                setYear(Number(event.target.value));
                setEditing(null);
              }}
            >
              {years.map((item) => (
                <option key={item} value={item}>
                  {t("AY {{year}}", { year: item })}
                </option>
              ))}
            </UiSelect>
          )
        }
      />
      {status}
      <SettingsGroup title="Quarters">
        <div className="quarterGrid" data-setting="quarters">
          {view.quarters.map((quarter) => (
            <div key={quarter.quarter}>
              <b>{quarter.quarter}</b>
              <span>
                {formatDay(quarter.from, locale)} –{" "}
                {formatDay(quarter.to, locale)}
              </span>
              <OriginBadge origin={quarter.origin} />
            </div>
          ))}
        </div>
      </SettingsGroup>
      <SettingsGroup
        title="Class days"
        footer={
          readOnly
            ? t("Change class days in the desktop app.")
            : t(
                "Change an official day, or add your own, to fix your timetable on that date.",
              )
        }
      >
        <div data-setting="days" className="calendarDays">
          {view.days.length === 0 && (
            <p className="paneEmpty">{t("No holidays or make-up days yet.")}</p>
          )}
          {view.days.map((day) => (
            <div className="calendarDay" key={day.date}>
              <time dateTime={day.date}>
                {formatDay(day.date, locale, { weekday: "short" })}
              </time>
              <span>
                <b>{day.label}</b>
                <small>
                  {describe(day, locale)}
                  {day.official &&
                    ` · ${t("Official: {{label}}", { label: day.official.label })}`}
                </small>
              </span>
              <OriginBadge origin={day.origin} />
              {!readOnly && (
                <span className="actions">
                  <UiButton
                    className="textButton"
                    onClick={() => beginEdit(day)}
                  >
                    {day.origin === "official" ? t("Change") : t("Edit")}
                  </UiButton>
                  {day.exception && (
                    <UiButton
                      className="textButton"
                      onClick={() => remove(day.exception!.id)}
                    >
                      {day.origin === "changed"
                        ? t("Use official")
                        : t("Delete")}
                    </UiButton>
                  )}
                </span>
              )}
            </div>
          ))}
        </div>
        {editor}
        {!readOnly && !editing && (
          <div className="actions">
            <UiButton
              className="button small"
              onClick={() =>
                setEditing({
                  id: "new",
                  draft: {
                    ...blank,
                    date: year === followedYear ? "" : `${year}-04-01`,
                  },
                })
              }
            >
              ＋ {t("Add a day")}
            </UiButton>
          </div>
        )}
      </SettingsGroup>
      <SettingsGroup
        title="Important dates"
        footer={t(
          "Shown on Today and in Schedule. They do not cancel classes.",
        )}
      >
        <div data-setting="important-dates" className="calendarDays">
          {view.importantDates.length === 0 ? (
            <p className="paneEmpty">
              {t("No important dates for this year.")}
            </p>
          ) : (
            view.importantDates.map((row) => (
              <div className="calendarDay" key={row.id}>
                <time dateTime={row.startDate}>
                  {formatDay(row.startDate, locale)}
                  {row.endDate !== row.startDate &&
                    ` – ${formatDay(row.endDate, locale)}`}
                </time>
                <span>
                  <b>{pickLocalized(row.label, locale, row.category)}</b>
                </span>
                <OriginBadge origin="official" />
              </div>
            ))
          )}
        </div>
      </SettingsGroup>
      {!readOnly && (
        <MoreOptions label="Back up your calendar changes">
          <div className="actions">
            <UiInput
              ref={file}
              hidden
              type="file"
              accept="application/json,.json"
              onChange={importFile}
            />
            <UiButton className="button small" onClick={exportFile}>
              {t("Export changes")}
            </UiButton>
            <UiButton
              className="button small"
              onClick={() => file.current?.click()}
            >
              {t("Import changes…")}
            </UiButton>
          </div>
        </MoreOptions>
      )}
    </>
  );
}
