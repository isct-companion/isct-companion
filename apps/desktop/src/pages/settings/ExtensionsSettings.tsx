import { Badge } from "@mantine/core";
import { t } from "../../i18n";
import { UiButton, UiTextarea } from "../../components/mantine";
import {
  customizationNames,
  desktopBridge,
  type CustomizationSite,
} from "../../app/native";
import type { Workspace } from "../../lib/model";
import { SettingsHeader } from "../../components/forms";

/** Per-site CSS/JS injected into the school web view (styling only). */
export function ExtensionsSettings({
  data,
  update,
}: {
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
}) {
  const custom = data.settings.customization!;
  return (
    <>
      <SettingsHeader
        title="Site styles & scripts"
        description="Your own CSS and JavaScript for school sites. They apply the next time a site opens."
      />
      <div className="extensionList" data-setting="sites">
        {(Object.keys(custom.sites) as CustomizationSite[]).map((site) => {
          const enabled = data.settings.siteConsents?.[site] ?? false;
          return (
            <details className="siteEditor" key={site}>
              <summary>
                <b>{t(customizationNames[site])}</b>
                <Badge variant="light" color={enabled ? "green" : "gray"}>
                  {t(enabled ? "On" : "Off")}
                </Badge>
                <UiButton
                  className="button small"
                  disabled={!desktopBridge}
                  onClick={(event) => {
                    event.preventDefault();
                    update((draft) => {
                      draft.settings.siteConsents ??= {
                        portal: false,
                        lms: false,
                        syllabus: false,
                      };
                      draft.settings.siteConsents[site] = !enabled;
                      return draft;
                    });
                  }}
                >
                  {t(enabled ? "Disable" : "Enable")}
                </UiButton>
              </summary>
              <div className="siteEditorBody">
                <label>
                  CSS
                  <UiTextarea
                    aria-label={`${site} CSS`}
                    spellCheck={false}
                    value={custom.sites[site].css}
                    onChange={(event) =>
                      update((d) => {
                        d.settings.customization!.sites[site].css =
                          event.target.value;
                        return d;
                      })
                    }
                  />
                </label>
                <label>
                  JavaScript
                  <UiTextarea
                    aria-label={`${site} JavaScript`}
                    spellCheck={false}
                    value={custom.sites[site].js}
                    onChange={(event) =>
                      update((d) => {
                        d.settings.customization!.sites[site].js =
                          event.target.value;
                        return d;
                      })
                    }
                  />
                </label>
              </div>
            </details>
          );
        })}
      </div>
    </>
  );
}
