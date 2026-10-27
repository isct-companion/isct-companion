import { t } from "../../i18n";
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
const themes: { id: ThemeId; title: string; bg: string; surface: string }[] = [
  { id: "warm", title: "Warm", bg: "#f3f3f0", surface: "#ffffff" },
  { id: "cool", title: "Cool", bg: "#f2f2f7", surface: "#ffffff" },
  { id: "paper", title: "Paper", bg: "#f3eee4", surface: "#fffdf8" },
  { id: "mist", title: "Mist", bg: "#edf1f6", surface: "#fbfcfe" },
  { id: "moss", title: "Moss", bg: "#eef2ec", surface: "#fcfdfb" },
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
    <div className="themeSwatches" role="radiogroup" aria-label={t("Theme")}>
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
      <SettingsHeader title="Appearance" />
      <SettingsGroup>
        <Row id="language" group title="Language">
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
        <Row id="content-language" group title="School content language">
          <Choice
            value={data.settings.contentLanguage ?? "auto"}
            options={[
              { value: "auto", label: t("Automatic") },
              { value: "en", label: "English" },
              { value: "ja", label: "日本語" },
            ]}
            onChange={(language) => set("contentLanguage", language)}
          />
        </Row>
        <Row id="appearance-mode" group title="Light and dark">
          <Choice
            value={data.settings.appearance}
            options={[
              { value: "system", label: t("Automatic") },
              { value: "light", label: t("Light") },
              { value: "dark", label: t("Dark") },
            ]}
            onChange={(appearance) => set("appearance", appearance)}
          />
        </Row>
        <Row id="theme" group title="Theme">
          <ThemeSwatches
            value={data.settings.theme ?? "warm"}
            onChange={(theme) => set("theme", theme)}
          />
        </Row>
        <Row id="text-size" group title="Text size">
          <Choice
            value={data.settings.textSize ?? "medium"}
            options={[
              { value: "small", label: t("Small") },
              { value: "medium", label: t("Medium") },
              { value: "large", label: t("Large") },
            ]}
            onChange={(textSize) => set("textSize", textSize)}
          />
        </Row>
        {closeOs && (
          <Row id="close-window" group title="When closing the window">
            <Choice
              value={data.settings.closeWindow ?? "ask"}
              options={[
                { value: "ask", label: t("Ask every time") },
                { value: "quit", label: t("Quit") },
                {
                  value: "background",
                  label: t(
                    closeOs === "macos" ? "Keep in Dock" : "Minimize to tray",
                  ),
                },
              ]}
              onChange={(closeWindow) => set("closeWindow", closeWindow)}
            />
          </Row>
        )}
        <Row id="bilingual" title="Show one language in bilingual LMS">
          <Toggle
            checked={data.settings.lmsBilingualFilter ?? false}
            onChange={(checked) => set("lmsBilingualFilter", checked)}
          />
        </Row>
      </SettingsGroup>
      <MoreOptions>
        <Row id="density" group title="Density">
          <Choice
            value={data.settings.density}
            options={[
              { value: "comfortable", label: t("Comfortable") },
              { value: "compact", label: t("Compact") },
            ]}
            onChange={(density) => set("density", density)}
          />
        </Row>
        <Row id="background" title="Background image">
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
