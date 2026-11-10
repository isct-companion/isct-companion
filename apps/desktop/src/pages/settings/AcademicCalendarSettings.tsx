import { CalendarDays } from "lucide-react";
import { formatJst, tokyoDate } from "../../lib/civil-date";
import { useClock } from "../../platform/use-clock";
import { refresh, useSync } from "../../app/useSync";
import { runsFor } from "../../platform/sync";
import { notify as appNotify } from "../../app/notify";
import { Fragment, useEffect, useRef, useState, type FormEvent } from "react";
import { t, type MessageKey } from "../../i18n";
import {
  contentLanguage,
  pickLocalized,
  type CalendarException,
  type Workspace,
} from "../../lib/model";
import {
  useScheduleQuery,
  type AcademicYearIndex,
  type AcademicYearCalendar,
  type CalendarDayView,
  type CalendarOrigin,
} from "../../platform/core-query";
import type { WorkspaceIntent } from "../../lib/workspace-intents";
import { UiButton, UiInput, UiSelect, EmptyState } from "../../components/ui";
import { SettingsGroup, SettingsHeader, Status } from "../../components/forms";
import {
  formatDay,
  languageTag,
  useSubscriptionViews,
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

function weekdayName(day: number, locale: Workspace["settings"]["locale"]) {
  // 2024-01-01 was a Monday; day 1 = Monday … 7 = Sunday.
  return formatJst(new Date(Date.UTC(2024, 0, day)), locale, {
    weekday: "long",
    timeZone: "UTC",
  });
}

function describe(
  day: { type: string; replacementDay?: number },
  locale: Workspace["settings"]["locale"],
) {
  if (day.type === "replacement")
    return t("settings.academicCalendar.weekdayTimetable", {
      weekday: weekdayName(day.replacementDay || 1, locale),
    });
  if (day.type === "holiday") return t("common.holiday");
  if (day.type === "break") return t("settings.academicCalendar.break");
  return t("settings.academicCalendar.noClasses");
}

/** Official rows need no mark; only personal and estimated ones say so. */
const ORIGIN_NOTE: Partial<Record<CalendarOrigin, MessageKey>> = {
  changed: "settings.academicCalendar.changedByYou",
  own: "settings.academicCalendar.addedByYou",
  estimated: "settings.academicCalendar.estimated",
};

export function AcademicCalendarSettings({
  data,
  mutate,
  notify,
  readOnly,
  refreshReferences,
  back,
}: {
  data: Workspace;
  mutate: (intent: WorkspaceIntent) => Promise<unknown>;
  notify: (message: string) => void;
  readOnly: boolean;
  /** Whether this shell can ask Core to check the reference data now. */
  refreshReferences?: boolean;
  back: () => void;
}) {
  const sync = useSync();
  const now = useClock();
  const locale = data.settings.locale;
  const subscription = useSubscriptionViews().value?.calendar;
  const followedYear =
    subscription?.year ?? Number(data.selectedTerm.slice(0, 4));
  const [year, setYear] = useState(Number(data.selectedTerm.slice(0, 4)));
  const initializedYear = useRef(false);
  useEffect(() => {
    if (initializedYear.current || subscription?.year == null) return;
    initializedYear.current = true;
    setYear(subscription.year);
  }, [subscription?.year]);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [showPast, setShowPast] = useState(false);
  const view = useScheduleQuery<AcademicYearCalendar>({
    kind: "academicYear",
    year,
  }).value ?? {
    year,
    quarters: [],
    days: [],
    importantDates: [],
    published: false,
  };
  const yearIndex = useScheduleQuery<AcademicYearIndex>({
    kind: "academicYears",
    date: tokyoDate(new Date(now)),
    selectedYear: year,
  }).value;
  const years = [...new Set(yearIndex?.years ?? [followedYear, year])].sort(
    (a, b) => b - a,
  );

  // Core runs the check; the button only follows the reference run.
  const busy = runsFor(sync, ["reference"]).some((run) => !run.held);
  const checkNow = () => refresh({ kind: "references" });
  const checkButton = refreshReferences && year === followedYear && (
    <UiButton className="textButton" disabled={busy} onClick={checkNow}>
      {busy
        ? t("settings.academicCalendar.checking")
        : t("settings.academicCalendar.checkNow")}
    </UiButton>
  );
  const subscriptionState = subscription?.state ?? {
    state: "pending" as const,
  };
  // Only problems are worth a line; a healthy official calendar says nothing.
  const status =
    year !== followedYear ? (
      !view.published && (
        <Status tone="idle">
          {t("settings.academicCalendar.noOfficialCalendarSavedFor", { year })}
        </Status>
      )
    ) : subscriptionState.state === "error" ? (
      <Status tone="error" action={checkButton}>
        {t("settings.academicCalendar.couldNotUpdateTheOfficial")}{" "}
        <small>{subscriptionState.message}</small>
      </Status>
    ) : (
      !view.published &&
      subscriptionState.state !== "off" && (
        <Status tone="warn" action={checkButton}>
          {subscriptionState.state === "current"
            ? t("settings.academicCalendar.theOfficialCalendarForYear", {
                year,
              })
            : t("settings.academicCalendar.theOfficialCalendarHasNot")}
        </Status>
      )
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
      return notify(t("settings.academicCalendar.fillInTheDateType"));
    const values = {
      ...draft,
      label: draft.label.trim(),
      replacementDay:
        draft.type === "replacement" ? draft.replacementDay : undefined,
    };
    try {
      await mutate({
        kind: "saveCalendarException",
        draft: values,
        existingId: editing.id === "new" ? undefined : editing.id,
      });
      setEditing(null);
      notify(t("settings.academicCalendar.calendarSaved"));
    } catch (error) {
      appNotify.error(error);
    }
  };
  const remove = async (id: string) => {
    if (readOnly) return;
    try {
      await mutate({ kind: "deleteCalendarException", exceptionId: id });
      setEditing(null);
    } catch (error) {
      appNotify.error(error);
    }
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

  const editingException =
    editing && editing.id !== "new"
      ? view.days.find((day) => day.exception?.id === editing.id)
      : undefined;
  const editingExceptionRow = editingException?.exception;
  const editor = editing && !readOnly && (
    <form className="calendarEditor" onSubmit={save}>
      <label>
        {t("common.date")}
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
        {t("settings.academicCalendar.whatHappens")}
        <UiSelect
          value={editing.draft.type}
          onChange={(value) => {
            const type = value as Draft["type"];
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
          data={[
            {
              value: "no-class",
              label: t("settings.academicCalendar.noClasses"),
            },
            {
              value: "replacement",
              label: t("settings.academicCalendar.anotherDaysTimetable"),
            },
            { value: "break", label: t("settings.academicCalendar.break") },
          ]}
        />
      </label>
      {editing.draft.type === "replacement" && (
        <label>
          {t("settings.academicCalendar.useTheTimetableOf")}
          <UiSelect
            value={editing.draft.replacementDay}
            onChange={(value) =>
              setEditing({
                ...editing,
                draft: {
                  ...editing.draft,
                  replacementDay: Number(value),
                },
              })
            }
            data={[1, 2, 3, 4, 5, 6].map((day) => ({
              value: String(day),
              label: weekdayName(day, locale),
            }))}
          />
        </label>
      )}
      <label className="wide">
        {t("settings.academicCalendar.name")}
        <UiInput
          required
          value={editing.draft.label}
          placeholder={t("settings.academicCalendar.exampleName")}
          onChange={(event) =>
            setEditing({
              ...editing,
              draft: { ...editing.draft, label: event.target.value },
            })
          }
        />
      </label>
      <div className="actions end wide">
        {editingExceptionRow && (
          <UiButton
            className="textButton"
            type="button"
            onClick={() => remove(editingExceptionRow!.id)}
          >
            {editingException!.origin === "changed"
              ? t("settings.academicCalendar.useOfficial")
              : t("common.delete")}
          </UiButton>
        )}
        <UiButton
          className="button"
          type="button"
          onClick={() => setEditing(null)}
        >
          {t("common.cancel")}
        </UiButton>
        <UiButton className="button primary" type="submit">
          {t("common.save")}
        </UiButton>
      </div>
    </form>
  );

  const today = tokyoDate(new Date(now));
  const past = view.days.filter((day) => day.date < today);
  const days = showPast ? view.days : view.days.slice(past.length);
  const editingKey = (day: CalendarDayView) =>
    editing !== null &&
    (day.exception
      ? editing.id === day.exception.id
      : editing.id === "new" &&
        editing.fixedDate &&
        editing.draft.date === day.date);

  return (
    <>
      <UiButton className="textButton settingsBack" onClick={back}>
        ‹ {t("settings.academicCalendar.study")}
      </UiButton>
      <SettingsHeader
        title="settings.academicCalendar.academicCalendar"
        actions={
          years.length > 1 && (
            <UiSelect
              aria-label={t("common.academicYear")}
              value={year}
              onChange={(value) => {
                setYear(Number(value));
                setEditing(null);
                setShowPast(false);
              }}
              data={years.map((item) => ({
                value: String(item),
                label: t("timetable.academicYear", { year: item }),
              }))}
            />
          )
        }
      />
      {status}
      <SettingsGroup title="settings.academicCalendar.quarters">
        <div className="quarterGrid" data-setting="quarters">
          {view.quarters.map((quarter) => (
            <div key={quarter.quarter}>
              <b>{quarter.quarter}</b>
              <span>
                {formatDay(quarter.from, locale)} –{" "}
                {formatDay(quarter.to, locale)}
              </span>
              {ORIGIN_NOTE[quarter.origin] && (
                <small>{t(ORIGIN_NOTE[quarter.origin]!)}</small>
              )}
            </div>
          ))}
        </div>
      </SettingsGroup>
      <SettingsGroup
        title="settings.academicCalendar.classDays"
        footer={
          readOnly
            ? t("settings.academicCalendar.changeClassDaysInThe")
            : undefined
        }
      >
        {!readOnly && (
          <div className="actions calendarAdd">
            <UiButton
              className="textButton"
              disabled={editing?.id === "new" && !editing.fixedDate}
              onClick={() => setEditing({ id: "new", draft: { ...blank } })}
            >
              ＋ {t("settings.academicCalendar.addADay")}
            </UiButton>
          </div>
        )}
        {editing?.id === "new" && !editing.fixedDate && editor}
        <div data-setting="days" className="calendarDays">
          {view.days.length === 0 && (
            <EmptyState compact icon={CalendarDays}>
              {t("settings.academicCalendar.noHolidaysOrMakeUp")}
            </EmptyState>
          )}
          {past.length > 0 && (
            <UiButton
              className="textButton calendarPast"
              onClick={() => setShowPast((value) => !value)}
            >
              {showPast
                ? t("settings.academicCalendar.hidePastDays")
                : t("settings.academicCalendar.showPastDays")}
            </UiButton>
          )}
          {days.map((day) => {
            const open = editingKey(day);
            const note = ORIGIN_NOTE[day.origin];
            const content = (
              <>
                <time dateTime={day.date}>
                  {formatDay(day.date, locale, { weekday: "short" })}
                </time>
                <span>
                  <b>{day.label}</b>
                  <small>
                    {[
                      describe(day, locale),
                      note && t(note),
                      day.official &&
                        t("settings.academicCalendar.officialLabel", {
                          label: day.official.label,
                        }),
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </small>
                </span>
              </>
            );
            return (
              <Fragment key={day.date}>
                {readOnly ? (
                  <div className="calendarDay">{content}</div>
                ) : (
                  <button
                    type="button"
                    className={`calendarDay${open ? " open" : ""}`}
                    aria-expanded={open}
                    onClick={() => (open ? setEditing(null) : beginEdit(day))}
                  >
                    {content}
                  </button>
                )}
                {open && editor}
              </Fragment>
            );
          })}
        </div>
      </SettingsGroup>
      <SettingsGroup title="today.importantDates">
        <div data-setting="important-dates" className="calendarDays">
          {view.importantDates.length === 0 ? (
            <EmptyState compact icon={CalendarDays}>
              {t("settings.academicCalendar.noImportantDatesForThis")}
            </EmptyState>
          ) : (
            view.importantDates.map((row) => (
              <div className="calendarDay" key={row.id}>
                <time dateTime={row.startDate}>
                  {formatDay(row.startDate, locale)}
                  {row.endDate !== row.startDate &&
                    ` – ${formatDay(row.endDate, locale)}`}
                </time>
                <span>
                  <b>
                    {pickLocalized(
                      row.label,
                      contentLanguage(data.settings),
                      row.category,
                    )}
                  </b>
                </span>
              </div>
            ))
          )}
        </div>
      </SettingsGroup>
    </>
  );
}
