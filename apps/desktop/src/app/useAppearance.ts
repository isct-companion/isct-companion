import { useEffect } from "react";
import i18n from "../i18n";
import type { Workspace } from "../lib/model";

export function useAppearance(data: Workspace | undefined) {
  useEffect(() => {
    void i18n.changeLanguage(data?.settings.locale ?? "en");
    document.documentElement.lang = data?.settings.locale ?? "en";
  }, [data?.settings.locale]);
  // "Automatic" follows the system while the app is open, not only at launch.
  useEffect(() => {
    if (
      data?.settings.appearance !== "system" ||
      typeof matchMedia !== "function"
    )
      return;
    const query = matchMedia("(prefers-color-scheme: dark)");
    const follow = () => {
      document.documentElement.dataset.mantineColorScheme = query.matches
        ? "dark"
        : "light";
    };
    query.addEventListener("change", follow);
    return () => query.removeEventListener("change", follow);
  }, [data?.settings.appearance]);
  useEffect(() => {
    if (!data) return;
    document.documentElement.dataset.appearance = data.settings.appearance;
    document.documentElement.dataset.mantineColorScheme =
      data.settings.appearance === "system"
        ? typeof matchMedia === "function" &&
          matchMedia("(prefers-color-scheme: dark)").matches
          ? "dark"
          : "light"
        : data.settings.appearance;
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
