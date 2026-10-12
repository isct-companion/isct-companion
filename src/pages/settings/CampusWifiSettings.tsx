import { useEffect, useState } from "react";
import { Progress } from "@mantine/core";
import { t } from "../../i18n";
import { desktopBridge } from "../../app/native";
import {
  CAMPUS_WIFI_STEPS,
  type CampusWifiOutcome,
  type CampusWifiStatus,
  type CampusWifiStep,
} from "../../platform/desktop-bridge";
import { UiButton } from "../../components/mantine";
import { confirmAction } from "../../components/confirm";
import { Row, SettingsGroup, SettingsHeader } from "../../components/forms";
import { errorText } from "../../app/shared";

const OUTCOME_TEXT: Record<
  Exclude<CampusWifiOutcome, "science-tokyo-x">,
  string
> = {
  "signed-in": "Signed in to ScienceTokyo",
  "already-online": "Already connected",
  "needs-account": "Save the Tokyo Tech Portal account first",
  rejected: "The portal refused the Tokyo Tech account",
};

const STEP_TEXT: Record<CampusWifiStep, string> = {
  joining: "Joining ScienceTokyo…",
  "opening-portal": "Opening the sign-in page…",
  "signing-in": "Signing in…",
  verifying: "Waiting for the network to open…",
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
      const message = errorText(error) ?? t("Sign-in failed");
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
        t(
          "This Mac is on ScienceTokyoX, which needs no sign-in. Switch to ScienceTokyo anyway?",
        ),
        t("Switch"),
        { title: t("Switch to ScienceTokyo?"), danger: false },
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
      {step ? t(STEP_TEXT[step]) : t("Connecting…")}
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
      ? t("Signed in to ScienceTokyo")
      : status === "science-tokyo-x"
        ? t("Connected to ScienceTokyoX")
        : undefined))
  );

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
        <Row group id="campus-wifi" title="ScienceTokyo Wi-Fi" hint={hint}>
          <UiButton
            className={`button small${connected ? " campusWifiConnected" : ""}`}
            disabled={checking || connecting || connected}
            onClick={() => void start()}
          >
            {t(
              connected
                ? "Connected"
                : connecting
                  ? "Connecting…"
                  : checking
                    ? "Checking…"
                    : "Connect",
            )}
          </UiButton>
        </Row>
      </SettingsGroup>
    </>
  );
}
