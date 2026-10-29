import i18next from "i18next";
import { initReactI18next } from "react-i18next";
// English is the source of truth: every key has its copy in en.json, and ja
// and zh carry the same key set.
import en from "../../../../resources/locales/en.json";
import ja from "../../../../resources/locales/ja.json";
import zh from "../../../../resources/locales/zh.json";
export { intlLocale, relativeTimeAgo } from "./locale";
void i18next.use(initReactI18next).init({
  resources: {
    en: { translation: en },
    ja: { translation: ja },
    zh: { translation: zh },
  },
  lng: "en",
  fallbackLng: "en",
  keySeparator: false,
  nsSeparator: false,
  interpolation: { escapeValue: false },
  react: { useSuspense: false },
  initAsync: false,
});
export default i18next;

/** A key of the shared catalogs (`resources/locales`). A label that reaches
 * `t` through a prop, a table or a state value is typed with this, so a
 * misspelt or missing key fails the build instead of showing in the UI. */
export type MessageKey = keyof typeof en;

/** The UI string for a catalog key, `{{name}}` placeholders filled. A key a
 * language lacks reads from the English catalog. */
export const t = (key: MessageKey, values?: Record<string, unknown>) =>
  String(i18next.t(key, values));

/** The English copy of a key, for matching a search against the source text
 * as well as the translated one. */
export const english = (key: MessageKey): string => en[key];

/** The string for a key some other component owns (Core's labels, a native
 * command's code); `undefined` when no catalog has it. */
export const lookup = (key: string, values?: Record<string, unknown>) =>
  i18next.exists(key) ? String(i18next.t(key, values)) : undefined;
