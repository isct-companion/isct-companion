import { Check, Copy } from "lucide-react";
import { useEffect, useState } from "react";
import { desktopBridge } from "../app/native";
import { t } from "../i18n";
import type { SchoolOtpCode } from "../platform/desktop-bridge";
import { Button, Modal, Stack, Text } from "./ui";

export function SchoolOtpCodeButton({ disabled }: { disabled: boolean }) {
  const [opened, setOpened] = useState(false);
  return (
    <>
      <Button
        size="xs"
        variant="default"
        disabled={disabled || !desktopBridge}
        onClick={() => setOpened(true)}
      >
        {t("settings.otp.showCode")}
      </Button>
      <Modal
        opened={opened}
        onClose={() => setOpened(false)}
        title={t("settings.otp.currentCode")}
        size="xs"
        centered
      >
        {opened && <CurrentCode />}
      </Modal>
    </>
  );
}

function CurrentCode() {
  const [visible, setVisible] = useState<{
    snapshot: SchoolOtpCode;
    seconds: number;
  } | null>(null);
  const [unavailable, setUnavailable] = useState(false);
  const [copyResult, setCopyResult] = useState<{
    snapshot: SchoolOtpCode;
    status: "copied" | "failed";
  } | null>(null);

  useEffect(() => {
    const bridge = desktopBridge;
    if (!bridge) return;
    let stop: (() => void) | undefined;
    function restart() {
      stop?.();
      setVisible(null);
      setUnavailable(false);
      setCopyResult(null);
      if (document.hidden) return;

      let current: SchoolOtpCode | null = null;
      let timer: ReturnType<typeof setTimeout>;
      let cancelled = false;
      stop = () => {
        cancelled = true;
        clearTimeout(timer);
      };
      async function tick() {
        if (!current || current.expiresAt * 1000 <= Date.now()) {
          setVisible(null);
          setCopyResult(null);
          try {
            current = (await bridge?.schoolOtpCode()) ?? null;
          } catch {
            current = null;
          }
          if (cancelled) return;
          if (!current) {
            setUnavailable(true);
            return;
          }
        }
        const now = Date.now();
        const seconds = Math.ceil((current.expiresAt * 1000 - now) / 1000);
        if (seconds > 0) setVisible({ snapshot: current, seconds });
        timer = setTimeout(() => void tick(), 1000 - (now % 1000));
      }
      void tick();
    }
    restart();
    document.addEventListener("visibilitychange", restart);
    window.addEventListener("focus", restart);
    return () => {
      stop?.();
      document.removeEventListener("visibilitychange", restart);
      window.removeEventListener("focus", restart);
    };
  }, []);

  async function copy(snapshot: SchoolOtpCode) {
    if (!desktopBridge || snapshot.expiresAt * 1000 <= Date.now()) return;
    try {
      await desktopBridge.copySchoolOtpCode(snapshot.otp);
      setCopyResult({ snapshot, status: "copied" });
    } catch {
      setCopyResult({ snapshot, status: "failed" });
    }
  }

  if (unavailable) {
    return (
      <Text role="status" size="sm">
        {t("settings.otp.codeUnavailable")}
      </Text>
    );
  }
  // Between two codes, and while the first one loads, the panel keeps its
  // size so the dialog does not jump at each rollover.
  if (!visible) {
    return (
      <Stack gap="space-7" aria-busy="true">
        <div className="schoolOtpCode">
          <span className="schoolOtpCodeDigits" data-pending>
            <span>···</span>
            <span>···</span>
          </span>
          <div className="schoolOtpCountdown" />
        </div>
        <Button size="sm" disabled>
          {t("common.loading")}
        </Button>
      </Stack>
    );
  }
  const copyState =
    copyResult?.snapshot === visible.snapshot ? copyResult.status : null;
  const { otp, expiresAt, period } = visible.snapshot;
  // Grouped as authenticators show it: 123 456.
  const half = Math.floor(otp.length / 2);
  return (
    <Stack gap="space-7">
      <div className="schoolOtpCode">
        <button
          type="button"
          className="schoolOtpCodeDigits"
          aria-label={`${otp}, ${t("settings.otp.copyCode")}`}
          title={t("settings.otp.copyCode")}
          data-trace-sensitive
          onClick={() => void copy(visible.snapshot)}
        >
          <span>{otp.slice(0, half)}</span>
          <span>{otp.slice(half)}</span>
        </button>
        <OtpCountdown
          key={expiresAt}
          expiresAt={expiresAt}
          seconds={visible.seconds}
          period={period}
        />
      </div>
      <Button
        size="sm"
        leftSection={
          copyState === "copied" ? (
            <Check size={14} strokeWidth={2} aria-hidden />
          ) : (
            <Copy size={14} strokeWidth={2} aria-hidden />
          )
        }
        onClick={() => void copy(visible.snapshot)}
      >
        {t(
          copyState === "copied"
            ? "settings.credentialImport.copied"
            : "settings.otp.copyCode",
        )}
      </Button>
      {copyState === "failed" && (
        <Text role="alert" size="sm" c="red">
          {t("settings.otp.copyFailed")}
        </Text>
      )}
    </Stack>
  );
}

/** What is left of the time step, as a ring that empties with the seconds
 * inside it. The ring runs on one CSS animation per code, started at the
 * share already spent, so it moves smoothly between the once-a-second
 * renders; it turns to the danger colour for the last five seconds. */
function OtpCountdown({
  expiresAt,
  seconds,
  period,
}: {
  expiresAt: number;
  seconds: number;
  period: number;
}) {
  const [spent] = useState(() =>
    Math.max(0, period - (expiresAt * 1000 - Date.now()) / 1000),
  );
  return (
    <div
      className="schoolOtpCountdown"
      role="timer"
      aria-label={t("settings.otp.expiresIn", { seconds })}
      data-ending={seconds <= 5 || undefined}
    >
      <svg viewBox="0 0 36 36" aria-hidden="true">
        <circle className="schoolOtpCountdownTrack" cx="18" cy="18" r="16" />
        <circle
          className="schoolOtpCountdownLeft"
          cx="18"
          cy="18"
          r="16"
          pathLength={1}
          style={{
            animationDuration: `${period}s`,
            animationDelay: `-${spent}s`,
          }}
        />
      </svg>
      <span aria-hidden="true">{seconds}</span>
    </div>
  );
}
