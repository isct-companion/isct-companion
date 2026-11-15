import { Wifi } from "lucide-react";
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
import { InfoTip, UiButton } from "../../components/ui";
import { confirmAction } from "../../components/confirm";
import { ToolCard } from "./ToolCard";
import { errorText } from "../../app/shared";

const OUTCOME_TEXT: Record<
  Exclude<CampusWifiOutcome, "science-tokyo-x">,
  MessageKey
> = {
  "signed-in": "tools.campusWifi.signedInToScienceTokyo",
  "already-online": "tools.campusWifi.alreadyConnected",
  "needs-account": "tools.campusWifi.saveTheTokyoTechPortal",
  rejected: "tools.campusWifi.thePortalRefusedTheTokyo",
};

const STEP_TEXT: Record<CampusWifiStep, MessageKey> = {
  joining: "tools.campusWifi.joiningScienceTokyo",
  "opening-portal": "tools.campusWifi.openingTheSignInPage",
  "signing-in": "common.signingIn",
  verifying: "tools.campusWifi.waitingForTheNetworkTo",
};

/**
 * ScienceTokyo Wi-Fi: one sign-in per click, nothing in the background. The
 * Wi-Fi state is read once each time the page opens.
 */
export function CampusWifi({ notify }: { notify: (message: string) => void }) {
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
        t("tools.campusWifi.thisMacIsOnScienceTokyoX"),
        t("tools.campusWifi.switch"),
        { title: t("tools.campusWifi.switchToScienceTokyo"), danger: false },
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
      {step ? t(STEP_TEXT[step]) : t("tools.campusWifi.connecting")}
      <Progress
        className="toolProgress"
        value={progress}
        size="sm"
        animated
        transitionDuration={300}
      />
    </>
  ) : (
    (result ??
    (checking
      ? t("settings.academicCalendar.checking")
      : connected
        ? t("tools.campusWifi.signedInToScienceTokyo")
        : status === "science-tokyo-x"
          ? t("tools.campusWifi.scienceTokyoXNoSignIn")
          : status === "sign-in-required"
            ? t("tools.campusWifi.signInRequired")
            : status === "other"
              ? t("tools.campusWifi.notOnCampusWifi")
              : undefined))
  );

  return (
    <ToolCard
      icon={<Wifi aria-hidden strokeWidth={1.75} />}
      title="tools.campusWifi.campusWiFi"
      info={
        <InfoTip label={t("tools.campusWifi.wifiRulesLabel")}>
          {t("tools.campusWifi.wifiRules")}
        </InfoTip>
      }
      status={hint}
      note="tools.campusWifi.wifiHint"
      actions={
        // Signed in, there is nothing to press until the sign-in expires.
        !connected && (
          <UiButton
            className="button small"
            disabled={checking || connecting}
            onClick={() => void start()}
          >
            {t(
              connecting
                ? "tools.campusWifi.connecting"
                : "tools.campusWifi.connect",
            )}
          </UiButton>
        )
      }
    />
  );
}
