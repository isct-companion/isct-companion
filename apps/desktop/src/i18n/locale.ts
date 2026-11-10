/** UI date formatting; school text has a separate Japanese/English preference. */
export function intlLocale(locale: string) {
  return (
    ({ en: "en-GB", ja: "ja-JP", zh: "zh-CN" } as Record<string, string>)[
      locale
    ] ?? "en-GB"
  );
}

const RELATIVE_UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ["day", 86_400_000],
  ["hour", 3_600_000],
  ["minute", 60_000],
];

/** "12 minutes ago" in the UI language; null under a minute so callers can
 * say "just now" in their own words. */
export function relativeTimeAgo(at: string, now: string, locale: string) {
  const elapsed = Date.parse(now) - Date.parse(at);
  if (!Number.isFinite(elapsed) || elapsed < 60_000) return null;
  const [unit, size] =
    RELATIVE_UNITS.find(([, size]) => elapsed >= size) ?? RELATIVE_UNITS[2];
  return new Intl.RelativeTimeFormat(intlLocale(locale), {
    numeric: "auto",
  }).format(-Math.floor(elapsed / size), unit);
}
