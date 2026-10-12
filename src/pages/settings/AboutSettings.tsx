import { Row, SettingsGroup, SettingsHeader } from "../../components/forms";

export function AboutSettings() {
  return (
    <>
      <SettingsHeader title="About" />
      <SettingsGroup>
        <Row id="version" group title="Version">
          <span className="aboutValue">{__APP_VERSION__}</span>
        </Row>
      </SettingsGroup>
    </>
  );
}
