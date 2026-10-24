import { useEffect } from "react";
import i18n from "../i18n";
import type { Workspace } from "../lib/model";

export function useAppearance(data: Workspace | undefined) {
  useEffect(() => {
    void i18n.changeLanguage(data?.settings.locale ?? "en");
    document.documentElement.lang = data?.settings.locale ?? "en";
  }, [data?.settings.locale]);
  // The generated dark CSS follows system changes through a media query.
  useEffect(() => {
    if (!data) return;
    document.documentElement.dataset.appearance = data.settings.appearance;
    document.documentElement.dataset.textSize =
      data.settings.textSize ?? "medium";
    document.body.style.backgroundImage = data.settings.customization
      ?.background
      ? `url("${data.settings.customization.background.replaceAll('"', "%22")}")`
      : "";
    document.getElementById("user-app-css")?.remove();
  }, [
    data?.settings.appearance,
    data?.settings.textSize,
    data?.settings.customization,
  ]);
}
