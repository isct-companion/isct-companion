import { t } from "../../i18n";
import type { Workspace } from "../../core/model";
import { UiInput } from "../../components/mantine";
import {
  Choice,
  MoreOptions,
  Row,
  SettingsGroup,
  SettingsHeader,
} from "../../components/forms";

type Settings = Workspace["settings"];

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
  return (
    <>
      <SettingsHeader
        title="Appearance & language"
        description="How the app looks on this device."
      />
      <SettingsGroup>
        <Row
          id="language"
          group
          title="Language"
          hint="Menus and course names from the syllabus"
        >
          <Choice
            value={data.settings.locale === "ja" ? "ja" : "en"}
            options={[
              { value: "en", label: "English" },
              { value: "ja", label: "日本語" },
            ]}
            onChange={(locale) => set("locale", locale)}
          />
        </Row>
        <Row id="theme" group title="Theme">
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
        <Row
          id="shortcuts"
          group
          title="Keyboard shortcuts"
          hint="⌘1–⌘4 switch pages · Esc closes panels"
        >
          <span />
        </Row>
      </SettingsGroup>
      <MoreOptions>
        <Row
          id="density"
          group
          title="Density"
          hint="Compact fits more rows on screen"
        >
          <Choice
            value={data.settings.density}
            options={[
              { value: "comfortable", label: t("Comfortable") },
              { value: "compact", label: t("Compact") },
            ]}
            onChange={(density) => set("density", density)}
          />
        </Row>
        <Row
          id="background"
          title="Background image"
          hint="Paste an image link. Leave empty for none."
        >
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
