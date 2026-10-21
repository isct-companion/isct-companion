import { useEffect, useRef, useState } from "react";
import {
  Alert,
  Button,
  FileButton,
  Group,
  Input,
  Modal,
  Stack,
  Text,
  TextInput,
} from "@mantine/core";
import { t } from "../i18n";
import { useTimeout } from "@mantine/hooks";
import { desktopBridge } from "../app/native";
import {
  importOtp,
  otpImportError,
  readOtpImage,
  type OtpImportAccount,
} from "../platform/otp-import";
import { QrCamera } from "./QrCamera";
import "./otp-setup-input.css";

type SetupSource = "new" | "reset" | "import";

function OtpSetupGuide({
  saved,
  disabled,
}: {
  saved: boolean;
  disabled: boolean;
}) {
  // A saved app secret says nothing about registration on the school site.
  const [source, setSource] = useState<SetupSource | null>(
    saved ? null : "new",
  );
  const [error, setError] = useState("");
  const openSettings = async () => {
    setError("");
    try {
      if (!desktopBridge)
        throw new Error(t("School settings are unavailable."));
      await desktopBridge.openExternal(
        "https://isct.ex-tic.com/idm/user/mfa_setting/",
      );
    } catch {
      setError(t("Could not open school settings. Try again."));
    }
  };
  return (
    <Stack gap="xs" mt="xs">
      <Group gap="xs">
        {(
          [
            ["new", "Set up at school"],
            ["reset", "Reset at school"],
            ["import", "Import from another app"],
          ] as const
        ).map(([key, label]) => (
          <Button
            key={key}
            variant={source === key ? "light" : "subtle"}
            size="compact-xs"
            disabled={disabled}
            aria-expanded={source === key}
            aria-controls="school-otp-guide"
            onClick={() => {
              setSource(source === key ? null : key);
              setError("");
            }}
          >
            {t(label)}
          </Button>
        ))}
      </Group>
      {source && (
        <Stack id="school-otp-guide" gap="xs" className="otpSetupGuide">
          {!saved && (
            <Text size="xs" c="dimmed">
              {t(
                "No OTP setup is saved in this app. Your school account may already have App Authentication registered.",
              )}
            </Text>
          )}
          {source === "import" ? (
            <>
              <Text size="sm">
                {t(
                  "In Google Authenticator, open Transfer accounts → Export accounts. Select only your school account, then scan its QR code here.",
                )}
              </Text>
              <Text size="sm">
                {t(
                  "For other authenticators, export a TOTP setup QR code or setup link. Use Scan QR, Select image, or paste the link above, then press Connect to save it.",
                )}
              </Text>
              <Text size="xs" c="dimmed">
                {t(
                  "Importing keeps the same school OTP setup. If your authenticator cannot export it, use Reset at school.",
                )}
              </Text>
            </>
          ) : (
            <>
              {source === "reset" ? (
                <Alert color="orange">
                  {t(
                    "Resetting replaces your school OTP setup. The old authenticator entry will stop working. Keep another working sign-in method available before removing App Authentication.",
                  )}
                </Alert>
              ) : null}
              <Text size="sm">
                {source === "reset"
                  ? t(
                      "1. Open school verification settings. Under App Authentication, remove the existing registration, then register it again.",
                    )
                  : t(
                      "1. Open school verification settings and register App Authentication. If it says Already Set, import from your authenticator or choose Reset at school.",
                    )}
              </Text>
              <Text size="sm">
                {t(
                  "2. Import the new QR code here using Scan QR or Select image, and add the same QR code to your authenticator. When resetting, replace its old school entry with the new one.",
                )}
              </Text>
              <Text size="sm">
                {t(
                  "3. Complete verification on the school page with a code from the new authenticator entry. After the school confirms registration, press Connect here to save this setup.",
                )}
              </Text>
              <Button
                variant="light"
                size="xs"
                disabled={disabled || !desktopBridge}
                onClick={() => void openSettings()}
                style={{ alignSelf: "flex-start" }}
              >
                {t("Open school verification settings")}
              </Button>
            </>
          )}
          <Text size="xs" c="dimmed">
            {t(
              "This field needs a setup link or secret, not the changing six-digit code. You can leave it blank and enter codes when signing in.",
            )}
          </Text>
          {error && (
            <Alert color="red" role="alert">
              {error}
            </Alert>
          )}
        </Stack>
      )}
    </Stack>
  );
}

export function OtpSetupInput({
  value,
  saved,
  disabled,
  onChange,
  onScanState,
}: {
  value: string;
  saved: boolean;
  disabled: boolean;
  onChange: (value: string) => void;
  onScanState: (scanning: boolean) => void;
}) {
  const [copied, setCopied] = useState(false);
  const copiedTimeout = useTimeout(() => setCopied(false), 2000);
  const copy = async () => {
    setError("");
    try {
      if (
        !desktopBridge ||
        !(await desktopBridge.copySchoolOtp(value.trim() || undefined))
      )
        throw new Error(t("Copy failed"));
      setCopied(true);
      copiedTimeout.start();
    } catch (reason) {
      setError(t(reason instanceof Error ? reason.message : String(reason)));
    }
  };
  const [camera, setCamera] = useState(false);
  const [reading, setReading] = useState(false);
  const [accounts, setAccounts] = useState<OtpImportAccount[]>([]);
  const [error, setError] = useState("");
  const [label, setLabel] = useState("");
  const generation = useRef(0);
  const resetImage = useRef<(() => void) | null>(null);
  useEffect(
    () => () => {
      generation.current += 1;
    },
    [],
  );
  useEffect(() => {
    onScanState(camera || reading);
    return () => onScanState(false);
  }, [camera, reading, onScanState]);
  const choose = (account: OtpImportAccount) => {
    onChange(account.setup);
    setLabel(account.label || account.issuer);
    setAccounts([]);
    setError("");
  };
  const load = async (source: string | File) => {
    const current = ++generation.current;
    setError("");
    setReading(true);
    try {
      let payload: string;
      try {
        payload =
          typeof source === "string" ? source : await readOtpImage(source);
      } catch (reason) {
        if (current === generation.current)
          setError(
            reason instanceof Error
              ? reason.message
              : t(
                  "No QR code found. Choose a clear image containing the whole QR code.",
                ),
          );
        return;
      }
      try {
        const found = await importOtp(payload);
        if (current !== generation.current) return;
        if (found.length === 1) choose(found[0]);
        else setAccounts(found);
      } catch (reason) {
        if (current === generation.current) setError(otpImportError(reason));
      }
    } finally {
      if (current === generation.current) setReading(false);
    }
  };
  return (
    <Input.Wrapper
      className="otpSetupField"
      label={
        <Group gap={5}>
          <span>{t("Automatic school verification codes")}</span>
        </Group>
      }
      id="school-otp-setup"
    >
      <div className="otpSetupRow">
        <TextInput
          id="school-otp-setup"
          type="password"
          aria-label={t("Automatic school verification codes")}
          value={value}
          autoCapitalize="none"
          autoComplete="off"
          spellCheck={false}
          disabled={disabled || reading}
          placeholder={saved ? "••••••••" : t("Paste a setup link or secret")}
          onChange={(event) => {
            onChange(event.target.value);
            setLabel("");
            setError("");
          }}
        />
        <Button.Group aria-label={t("Import authenticator QR code")}>
          <Button
            variant="default"
            aria-label={t("Copy OTP setup")}
            disabled={
              disabled ||
              reading ||
              camera ||
              (!value.trim() && !saved) ||
              !desktopBridge
            }
            onClick={() => void copy()}
          >
            {t(copied ? "Copied" : "Copy")}
          </Button>
          <Button
            variant="default"
            disabled={disabled || reading || camera}
            onClick={() => setCamera(true)}
          >
            {t("Scan QR")}
          </Button>
          <FileButton
            accept="image/png,image/jpeg,image/webp"
            onChange={(file) => {
              if (file) void load(file);
            }}
          >
            {(props) => (
              <Button
                {...props}
                variant="default"
                disabled={disabled || reading || camera}
                loading={reading}
              >
                {t("Select image")}
              </Button>
            )}
          </FileButton>
        </Button.Group>
      </div>
      <OtpSetupGuide saved={saved} disabled={disabled || reading || camera} />
      {label && (
        <Text size="xs" c="dimmed">
          {t("Read {{account}}. Connect to save this setup.", {
            account: label,
          })}
        </Text>
      )}
      {error && (
        <Alert color="red" role="alert" mt="xs">
          {error}
        </Alert>
      )}
      {camera && (
        <QrCamera
          onClose={() => setCamera(false)}
          onRead={(raw) => {
            setCamera(false);
            void load(raw);
          }}
        />
      )}
      {!!accounts.length && (
        <Modal
          opened
          onClose={() => setAccounts([])}
          title={t("Choose your school account")}
        >
          <Stack gap="xs" mah={360} style={{ overflowY: "auto" }}>
            {accounts.map((account, index) => (
              <Button
                key={index}
                variant="default"
                onClick={() => choose(account)}
              >
                {[account.issuer, account.label].filter(Boolean).join(" · ") ||
                  t("Account {{number}}", { number: index + 1 })}
              </Button>
            ))}
          </Stack>
        </Modal>
      )}
    </Input.Wrapper>
  );
}
