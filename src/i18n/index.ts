import i18next from "i18next";
import { initReactI18next } from "react-i18next";
import ja from "./ja.json";
void i18next.use(initReactI18next).init({
  resources: { en: { translation: {} }, ja: { translation: ja } },
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
