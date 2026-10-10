import { Row, SettingsGroup, SettingsHeader } from "../../components/forms";

/** Main open-source components shipped in the app, with their licenses. */
const LICENSES: Array<[name: string, license: string]> = [
  ["Tauri", "Apache-2.0 / MIT"],
  ["React", "MIT"],
  ["Mantine", "MIT"],
  ["Milkdown", "MIT"],
  ["CodeMirror", "MIT"],
  ["PDF.js", "Apache-2.0"],
  ["KaTeX", "MIT"],
  ["i18next", "MIT"],
  ["Zustand", "MIT"],
  ["rrule", "BSD-3-Clause"],
  ["PaddleOCR models", "Apache-2.0"],
  ["ONNX Runtime", "MIT"],
];

export function AboutSettings() {
  return (
    <>
      <SettingsHeader title="About" />
      <SettingsGroup>
        <Row id="version" group title="Version">
          <span className="aboutValue">{__APP_VERSION__}</span>
        </Row>
      </SettingsGroup>
      <SettingsGroup title="Open-source licenses">
        <ul className="aboutLicenses" data-setting="licenses">
          {LICENSES.map(([name, license]) => (
            <li key={name}>
              <span>{name}</span>
              <small>{license}</small>
            </li>
          ))}
        </ul>
      </SettingsGroup>
    </>
  );
}
