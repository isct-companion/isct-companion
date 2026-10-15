/** UI date formatting; school text has a separate Japanese/English preference. */
export function intlLocale(locale: string) {
  return (
    ({ en: "en-GB", ja: "ja-JP", zh: "zh-CN" } as Record<string, string>)[
      locale
    ] ?? "en-GB"
  );
}
