import { intlLocale } from "../i18n/locale";
export const JST = "Asia/Tokyo";

/** Parse machine timestamps with an explicit offset, never locale date text. */
export function parseIsoTimestamp(value: unknown): Date | undefined {
  if (
    typeof value !== "string" ||
    !/^\d{4}-\d{2}-\d{2}T(?:[01]\d|2[0-3]):[0-5]\d:[0-5]\d(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(
      value,
    ) ||
    !isCivilDate(value.slice(0, 10))
  )
    return undefined;
  const date = new Date(value);
  return Number.isFinite(date.getTime()) ? date : undefined;
}

/** Date accepts overflow days, so confirm a civil date survives an ISO round trip. */
export function isCivilDate(value: unknown): value is string {
  if (typeof value !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(value))
    return false;
  const date = new Date(`${value}T00:00:00Z`);
  return (
    Number.isFinite(date.getTime()) && date.toISOString().slice(0, 10) === value
  );
}

/** Shift a validated ISO-shaped civil date without involving the host zone. */
export function addCivilDays(date: string, days: number): string {
  const value = new Date(`${date}T12:00:00Z`);
  value.setUTCDate(value.getUTCDate() + days);
  return value.toISOString().slice(0, 10);
}

/** Monday = 1 … Sunday = 7, for timetable display and navigation. */
export function civilWeekday(date: string): number {
  return new Date(`${date}T12:00:00Z`).getUTCDay() || 7;
}

/** Machine dates must not depend on a locale's display order or separators. */
export function tokyoDate(date: Date): string {
  const parts = new Intl.DateTimeFormat("en-CA", {
    timeZone: JST,
    calendar: "gregory",
    numberingSystem: "latn",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).formatToParts(date);
  return ["year", "month", "day"]
    .map((type) => parts.find((part) => part.type === type)!.value)
    .join("-");
}

/** Minutes since midnight in Japan time. */
export function jstMinutes(date: Date): number {
  const parts = new Intl.DateTimeFormat("en-GB", {
    timeZone: JST,
    numberingSystem: "latn",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).formatToParts(date);
  const hour = Number(parts.find((p) => p.type === "hour")?.value ?? 0);
  const minute = Number(parts.find((p) => p.type === "minute")?.value ?? 0);
  return hour * 60 + minute;
}

/** `HH:MM` in Japan time, as Core writes class start and end times. */
export function jstClock(date: Date): string {
  const minutes = jstMinutes(date);
  return `${String(Math.floor(minutes / 60)).padStart(2, "0")}:${String(minutes % 60).padStart(2, "0")}`;
}

/** Display timestamps in the academic zone; malformed legacy values never reach Intl. */
export function formatTimestamp(
  value: unknown,
  locale: string,
  options: Intl.DateTimeFormatOptions = {},
): string | undefined {
  const date = parseIsoTimestamp(value);
  return date
    ? formatJst(date, locale, {
        year: "numeric",
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
        ...options,
      })
    : undefined;
}

/** One display boundary for academic civil dates; an ISO date is JST midnight. */
export function formatCivilDate(
  value: string,
  locale: string,
  options: Intl.DateTimeFormatOptions = {},
): string {
  if (!isCivilDate(value)) return "";
  return (
    formatTimestamp(`${value}T00:00:00+09:00`, locale, {
      year: undefined,
      month: "short",
      day: "numeric",
      hour: undefined,
      minute: undefined,
      ...options,
    }) ?? ""
  );
}

/** Locale affects presentation only; every instant and civil date is shown in JST. */
export function formatJst(
  value: Date | string,
  locale: string,
  options: Intl.DateTimeFormatOptions,
): string {
  const date =
    value instanceof Date
      ? value
      : isCivilDate(value)
        ? parseIsoTimestamp(`${value}T00:00:00+09:00`)
        : parseIsoTimestamp(value);
  if (!date || !Number.isFinite(date.getTime())) return "";
  return new Intl.DateTimeFormat(intlLocale(locale), {
    hourCycle: "h23",
    ...options,
    timeZone: JST,
    calendar: "gregory",
  }).format(date);
}
