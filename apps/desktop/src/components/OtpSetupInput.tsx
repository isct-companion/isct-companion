import { useEffect, useRef, useState } from "react";
import {
  ActionIcon,
  Alert,
  Button,
  FileButton,
  Group,
  Input,
  Modal,
  Popover,
  SegmentedControl,
  Stack,
  Text,
  TextInput,
  Tooltip,
} from "@mantine/core";
import { t } from "../i18n";
import { desktopBridge } from "../app/native";
import {
  importOtp,
  otpImportError,
  readOtpImage,
  type OtpImportAccount,
} from "../platform/otp-import";
import { QrCamera } from "./QrCamera";
import "./otp-setup-input.css";

function OtpHelp() {
  const [opened, setOpened] = useState(false);
  const [source, setSource] = useState("google");
  return (
    <Popover
      opened={opened}
      onChange={setOpened}
      width={340}
      position="bottom-start"
      withinPortal
    >
      <Popover.Target>
        <Tooltip disabled={opened} label={t("How do I find the QR code?")}>
          <ActionIcon
            variant="subtle"
            color="gray"
            size="sm"
            aria-label={t("How do I find the QR code?")}
            aria-expanded={opened}
            onClick={() => setOpened((v) => !v)}
          >
            ?
          </ActionIcon>
        </Tooltip>
      </Popover.Target>
      <Popover.Dropdown>
        <Stack gap="sm">
          <Text size="sm" fw={600}>
            {t("How do I find the QR code?")}
          </Text>
          <SegmentedControl
            value={source}
            onChange={setSource}
            data={[
              { value: "google", label: "Google" },
              { value: "other", label: t("Microsoft / other") },
              { value: "new", label: t("Not set up") },
            ]}
          />
          <Text size="sm">
            {source === "google"
              ? t(
                  "In Google Authenticator, open Transfer accounts → Export accounts. Select only your school account, then scan its QR code here.",
                )
              : source === "other"
                ? t(
                    "Look for an export QR code in your authenticator. If it cannot export, keep using it and enter the verification code when signing in. Do not remove your current setup.",
                  )
                : t(
                    "Open the school account settings and set up App Authentication. Import the QR code shown during setup, keep it in your phone authenticator too, and finish verification on the school page.",
                  )}
          </Text>
          {source === "new" && (
            <Button
              variant="light"
              size="xs"
              onClick={() =>
                void desktopBridge?.openExternal(
                  "https://isct.ex-tic.com/idm/user/mfa_setting/",
                )
              }
            >
              {t("Open school verification settings")}
            </Button>
          )}
          <Text size="xs" c="dimmed">
            {t(
              "This field needs a setup link or secret, not the changing six-digit code. You can leave it blank and enter codes when signing in.",
            )}
          </Text>
        </Stack>
      </Popover.Dropdown>
    </Popover>
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
          <OtpHelp />
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
