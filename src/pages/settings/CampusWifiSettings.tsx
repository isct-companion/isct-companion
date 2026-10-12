import { useState } from "react";
import { t } from "../../i18n";
import { desktopBridge } from "../../app/native";
import type { CampusWifiOutcome } from "../../platform/desktop-bridge";
import { UiButton } from "../../components/mantine";
import { Row, SettingsGroup, SettingsHeader } from "../../components/forms";
import { errorText } from "../../app/shared";

const OUTCOME_TEXT: Record<CampusWifiOutcome, string> = {
  "signed-in": "Signed in to ScienceTokyo",
  "already-online": "Already connected",
  "needs-account": "Save the Tokyo Tech Portal account first",
  rejected: "The portal refused the Tokyo Tech account",
};

/** ScienceTokyo Wi-Fi: one sign-in per click, nothing in the background. */
export function CampusWifiSettings({
  notify,
}: {
  notify: (message: string) => void;
}) {
  const [connecting, setConnecting] = useState(false);
  const [result, setResult] = useState<string>();
  const connect = async () => {
    if (!desktopBridge?.campusWifiConnect) return;
    setConnecting(true);
    try {
      const outcome = await desktopBridge.campusWifiConnect();
      setResult(t(OUTCOME_TEXT[outcome]));
    } catch (error) {
      const message = errorText(error) ?? t("Sign-in failed");
      setResult(message);
      notify(message);
    } finally {
      setConnecting(false);
    }
  };
  return (
    <>
      <SettingsHeader
        title="Campus Wi-Fi"
        description="Joins ScienceTokyo and signs in to its login page with your Tokyo Tech Portal ID and password."
      />
      <SettingsGroup
        footer={t(
          "Runs only when you click. Sign-in lasts 12 hours; click again after it expires.",
        )}
      >
        <Row group id="campus-wifi" title="ScienceTokyo Wi-Fi" hint={result}>
          <UiButton
            className="button small"
            disabled={connecting}
            onClick={() => void connect()}
          >
            {t(connecting ? "Connecting…" : "Connect")}
          </UiButton>
        </Row>
      </SettingsGroup>
    </>
  );
}
