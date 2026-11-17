import { useEffect, useState } from "react";
import { desktopBridge } from "../app/native";
import { t } from "../i18n";
import type { SchoolOtpCode } from "../platform/desktop-bridge";
import { Button, Group, Modal, Stack, Text } from "./ui";

export function SchoolOtpCodeButton({ disabled }: { disabled: boolean }) {
  const [opened, setOpened] = useState(false);
  return (
    <>
      <Button
        size="xs"
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

  if (!visible) {
    return (
      <Text role="status" size="sm">
        {t(unavailable ? "settings.otp.codeUnavailable" : "common.loading")}
      </Text>
    );
  }
  const copyState =
    copyResult?.snapshot === visible.snapshot ? copyResult.status : null;
  return (
    <Stack gap="sm">
      <div className="schoolOtpCodeDigits" data-trace-sensitive>
        {visible.snapshot.otp}
      </div>
      <Text size="sm" c="dimmed">
        {t("settings.otp.expiresIn", { seconds: visible.seconds })}
      </Text>
      <Group>
        <Button onClick={() => void copy(visible.snapshot)}>
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
      </Group>
    </Stack>
  );
}
