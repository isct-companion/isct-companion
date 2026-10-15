import i18next from "i18next";
import { initReactI18next } from "react-i18next";
import ja from "../../../../resources/locales/ja.json";
import zh from "../../../../resources/locales/zh.json";
export { intlLocale } from "./locale";
void i18next.use(initReactI18next).init({
  resources: {
    en: { translation: {} },
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
export const t = (key: string, values?: Record<string, unknown>) =>
  String(i18next.t(key, { defaultValue: key, ...values }));
