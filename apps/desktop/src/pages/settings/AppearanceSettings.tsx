import { t } from "../../i18n";
import type { Workspace } from "../../lib/model";
import { UiInput } from "../../components/mantine";
import {
  Choice,
  MoreOptions,
  Row,
  SettingsGroup,
  SettingsHeader,
  Toggle,
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
