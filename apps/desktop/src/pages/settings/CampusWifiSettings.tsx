import { useEffect, useState } from "react";
import { Progress } from "../../components/ui";
import { t, type MessageKey } from "../../i18n";
import { desktopBridge } from "../../app/native";
import {
  CAMPUS_WIFI_STEPS,
  type CampusWifiOutcome,
  type CampusWifiStatus,
  type CampusWifiStep,
} from "../../platform/desktop-bridge";
import { UiButton } from "../../components/ui";
import { confirmAction } from "../../components/confirm";
import { Row, SettingsGroup, SettingsHeader } from "../../components/forms";
import { errorText } from "../../app/shared";

const OUTCOME_TEXT: Record<
  Exclude<CampusWifiOutcome, "science-tokyo-x">,
  MessageKey
> = {
  "signed-in": "settings.campusWifi.signedInToScienceTokyo",
  "already-online": "settings.campusWifi.alreadyConnected",
  "needs-account": "settings.campusWifi.saveTheTokyoTechPortal",
  rejected: "settings.campusWifi.thePortalRefusedTheTokyo",
};

const STEP_TEXT: Record<CampusWifiStep, MessageKey> = {
  joining: "settings.campusWifi.joiningScienceTokyo",
  "opening-portal": "settings.campusWifi.openingTheSignInPage",
  "signing-in": "common.signingIn",
  verifying: "settings.campusWifi.waitingForTheNetworkTo",
};

/**
 * ScienceTokyo Wi-Fi: one sign-in per click, nothing in the background. The
 * Wi-Fi state is read once each time the page opens.
 */
export function CampusWifiSettings({
  notify,
}: {
  notify: (message: string) => void;
}) {
  const [status, setStatus] = useState<CampusWifiStatus>();
  const [checking, setChecking] = useState(true);
  const [step, setStep] = useState<CampusWifiStep>();
  const [connecting, setConnecting] = useState(false);
  const [result, setResult] = useState<string>();
  const connected = status === "signed-in";

  useEffect(() => {
    let current = true;
    setChecking(true);
    void (async () => {
      const next = await desktopBridge
        ?.campusWifiStatus?.()
        .catch(() => undefined);
      if (!current) return;
      setStatus(next);
      setChecking(false);
    })();
    return () => {
      current = false;
    };
  }, []);

  const connect = async (leaveScienceTokyoX: boolean) => {
    if (!desktopBridge?.campusWifiConnect) return;
    setConnecting(true);
    setResult(undefined);
    try {
      const outcome = await desktopBridge.campusWifiConnect(
        leaveScienceTokyoX,
        setStep,
      );
      if (outcome === "science-tokyo-x") {
        setStatus("science-tokyo-x");
        return outcome;
      }
      if (outcome === "signed-in" || outcome === "already-online")
        setStatus("signed-in");
      setResult(t(OUTCOME_TEXT[outcome]));
      return outcome;
    } catch (error) {
      const message = errorText(error) ?? t("common.signInFailed");
      setResult(message);
      notify(message);
    } finally {
      setConnecting(false);
      setStep(undefined);
    }
  };

  // ScienceTokyoX needs no sign-in, so leaving it is the user's call.
  const start = async () => {
    if ((await connect(false)) !== "science-tokyo-x") return;
    if (
      await confirmAction(
        t("settings.campusWifi.thisMacIsOnScienceTokyoX"),
        t("settings.campusWifi.switch"),
        { title: t("settings.campusWifi.switchToScienceTokyo"), danger: false },
      )
    )
      await connect(true);
  };

  // Stages report when they start; the bar sits midway through the current one.
  const progress = step
    ? ((CAMPUS_WIFI_STEPS.indexOf(step) + 0.5) / CAMPUS_WIFI_STEPS.length) * 100
    : 0;
  const hint = connecting ? (
    <>
      {step ? t(STEP_TEXT[step]) : t("settings.campusWifi.connecting")}
      <Progress
        className="campusWifiProgress"
        value={progress}
        size="sm"
        animated
        transitionDuration={300}
      />
    </>
  ) : (
    (result ??
    (connected
      ? t("settings.campusWifi.signedInToScienceTokyo")
      : status === "science-tokyo-x"
        ? t("settings.campusWifi.connectedToScienceTokyoX")
        : undefined))
  );

  return (
    <>
      <SettingsHeader
        title="settings.campusWifi.campusWiFi"
        description="settings.campusWifi.joinsScienceTokyoAndSignsIn"
      />
      <SettingsGroup footer={t("settings.campusWifi.runsOnlyWhenYouClick")}>
        <Row
          group
          id="campus-wifi"
          title="settings.campusWifi.sciencetokyoWiFi"
          hint={hint}
        >
          <UiButton
            className={`button small${connected ? " campusWifiConnected" : ""}`}
            disabled={checking || connecting || connected}
            onClick={() => void start()}
          >
            {t(
              connected
                ? "settings.main.connected"
                : connecting
                  ? "settings.campusWifi.connecting"
                  : checking
                    ? "settings.academicCalendar.checking"
                    : "settings.campusWifi.connect",
            )}
          </UiButton>
        </Row>
      </SettingsGroup>
    </>
  );
}
