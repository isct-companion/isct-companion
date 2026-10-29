import { t, type MessageKey } from "../../i18n";
import type { Workspace } from "../../lib/model";
import { UiInput } from "../../components/ui";
import { nativeOs } from "../../platform/desktop-bridge";
import {
  Choice,
  MoreOptions,
  Row,
  SettingsGroup,
  SettingsHeader,
  Toggle,
} from "../../components/forms";

type Settings = Workspace["settings"];
type ThemeId = NonNullable<Settings["theme"]>;

/** Each theme's light background and surface, for its swatch. The values
 * mirror design/tokens (color.light.json for warm, theme.json for the
 * rest); the app itself reads them from generated/design/themes.css. */
const themes: {
  id: ThemeId;
  title: MessageKey;
  bg: string;
  surface: string;
}[] = [
  {
    id: "warm",
    title: "settings.main.warm",
    bg: "#f3f3f0",
    surface: "#ffffff",
  },
  {
    id: "cool",
    title: "settings.main.cool",
    bg: "#f2f2f7",
    surface: "#ffffff",
  },
  {
    id: "paper",
    title: "settings.main.paper",
    bg: "#f3eee4",
    surface: "#fffdf8",
  },
  {
    id: "mist",
    title: "settings.main.mist",
    bg: "#edf1f6",
    surface: "#fbfcfe",
  },
  {
    id: "moss",
    title: "settings.main.moss",
    bg: "#eef2ec",
    surface: "#fcfdfb",
  },
];

/** Five round swatches; choosing one applies and saves at once. */
function ThemeSwatches({
  value,
  onChange,
}: {
  value: ThemeId;
  onChange: (theme: ThemeId) => void;
}) {
  return (
    <div
      className="themeSwatches"
      role="radiogroup"
      aria-label={t("settings.main.theme")}
    >
      {themes.map((theme) => (
        <button
          type="button"
          role="radio"
          aria-checked={value === theme.id}
          aria-label={t(theme.title)}
          title={t(theme.title)}
          className="themeSwatch"
          data-theme-value={theme.id}
          key={theme.id}
          style={
            {
              "--swatch-bg": theme.bg,
              "--swatch-surface": theme.surface,
            } as React.CSSProperties
          }
          onClick={() => onChange(theme.id)}
        />
      ))}
    </div>
  );
}

export function AppearanceSettings({
  data,
  update,
}: {
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
}) {
  const set = <K extends keyof Settings>(key: K, value: Settings[K]) =>
    update((draft) => {
      draft.settings[key] = value;
      return draft;
    });
  // Only the macOS and Windows shells keep running after the window closes.
  const os = nativeOs();
  const closeOs = os === "macos" || os === "windows" ? os : undefined;
  return (
    <>
      <SettingsHeader title="settings.appearance.appearance" />
      <SettingsGroup>
        <Row id="language" group title="common.language">
          <Choice
            value={data.settings.locale}
            options={[
              { value: "en", label: "English" },
              { value: "ja", label: "日本語" },
              { value: "zh", label: "中文" },
            ]}
            onChange={(locale) => set("locale", locale)}
          />
        </Row>
        <Row
          id="content-language"
          group
          title="settings.main.schoolContentLanguage"
        >
          <Choice
            value={data.settings.contentLanguage ?? "auto"}
            options={[
              { value: "auto", label: t("common.automatic") },
              { value: "en", label: "English" },
              { value: "ja", label: "日本語" },
            ]}
            onChange={(language) => set("contentLanguage", language)}
          />
        </Row>
        <Row
          id="appearance-mode"
          group
          title="settings.appearance.lightAndDark"
        >
          <Choice
            value={data.settings.appearance}
            options={[
              { value: "system", label: t("common.automatic") },
              { value: "light", label: t("settings.appearance.light") },
              { value: "dark", label: t("settings.appearance.dark") },
            ]}
            onChange={(appearance) => set("appearance", appearance)}
          />
        </Row>
        <Row id="theme" group title="settings.main.theme">
          <ThemeSwatches
            value={data.settings.theme ?? "warm"}
            onChange={(theme) => set("theme", theme)}
          />
        </Row>
        <Row id="text-size" group title="common.textSize">
          <Choice
            value={data.settings.textSize ?? "medium"}
            options={[
              { value: "small", label: t("settings.appearance.small") },
              { value: "medium", label: t("settings.appearance.medium") },
              { value: "large", label: t("settings.appearance.large") },
            ]}
            onChange={(textSize) => set("textSize", textSize)}
          />
        </Row>
        {closeOs && (
          <Row
            id="close-window"
            group
            title="settings.appearance.closeWindow.title"
          >
            <Choice
              value={data.settings.closeWindow ?? "ask"}
              options={[
                {
                  value: "ask",
                  label: t("settings.appearance.closeWindow.ask"),
                },
                { value: "quit", label: t("common.quit") },
                {
                  value: "background",
                  label: t(
                    closeOs === "macos"
                      ? "window.close.keepInDock"
                      : "window.close.minimizeToTray",
                  ),
                },
              ]}
              onChange={(closeWindow) => set("closeWindow", closeWindow)}
            />
          </Row>
        )}
        <Row
          id="bilingual"
          title="settings.appearance.showOneLanguageInBilingual"
        >
          <Toggle
            checked={data.settings.lmsBilingualFilter ?? false}
            onChange={(checked) => set("lmsBilingualFilter", checked)}
          />
        </Row>
      </SettingsGroup>
      <MoreOptions>
        <Row id="density" group title="settings.appearance.density">
          <Choice
            value={data.settings.density}
            options={[
              {
                value: "comfortable",
                label: t("settings.appearance.comfortable"),
              },
              { value: "compact", label: t("settings.appearance.compact") },
            ]}
            onChange={(density) => set("density", density)}
          />
        </Row>
        <Row id="background" title="settings.appearance.backgroundImage">
          <UiInput
            value={data.settings.customization?.background ?? ""}
            placeholder="https://…"
            onChange={(event) =>
              update((draft) => {
                if (draft.settings.customization)
                  draft.settings.customization.background = event.target.value;
                return draft;
              })
            }
          />
        </Row>
      </MoreOptions>
    </>
  );
}
