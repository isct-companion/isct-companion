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
} from "./ui";
import { t } from "../i18n";
import { useTimeout } from "usehooks-ts";
import { desktopBridge } from "../app/native";
import {
  importOtp,
  otpImportError,
  readOtpImage,
  type OtpImportAccount,
} from "../platform/otp-import";
import { QrCamera } from "./QrCamera";
import "./otp-setup-input.css";

type OtpPathId = "school" | "export" | "paste";

/**
 * One way to give the app the school's OTP setup. The three are mutually
 * exclusive, so they read as radio cards; only the chosen one shows its
 * controls.
 */
function OtpPath({
  value,
  current,
  onSelect,
  title,
  hint,
  recommended,
  children,
}: {
  value: OtpPathId;
  current: OtpPathId;
  onSelect: (value: OtpPathId) => void;
  title: string;
  hint: string;
  recommended?: boolean;
  children: React.ReactNode;
}) {
  const selected = value === current;
  return (
    <div className="otpPath" data-selected={selected || undefined}>
      <button
        type="button"
        role="radio"
        aria-checked={selected}
        className="otpPathHead"
        onClick={() => onSelect(value)}
      >
        <i aria-hidden="true" />
        <b>{title}</b>
        {recommended && (
          <span className="otpRecommended">{t("Recommended")}</span>
        )}
      </button>
      {selected && (
        <div className="otpPathBody">
          <Text size="xs" c="dimmed">
            {hint}
          </Text>
          {children}
        </div>
      )}
    </div>
  );
}

export function OtpSetupInput({
  value,
  saved,
  disabled,
  onChange,
  onScanState,
  openSchoolSettings,
}: {
  value: string;
  saved: boolean;
  disabled: boolean;
  onChange: (value: string) => void;
  onScanState: (scanning: boolean) => void;
  openSchoolSettings: (url: string, setupOtp?: boolean) => void;
}) {
  const [copied, setCopied] = useState(false);
  useTimeout(() => setCopied(false), copied ? 2000 : null);
  const copy = async () => {
    setError("");
    try {
      if (
        !desktopBridge ||
        !(await desktopBridge.copySchoolOtp(value.trim() || undefined))
      )
        throw new Error(t("Copy failed"));
      setCopied(true);
    } catch (reason) {
      setError(t(reason instanceof Error ? reason.message : String(reason)));
    }
  };
  // The SVG holds the secret, so it lives only while the dialog is open.
  const [qr, setQr] = useState("");
  const showQr = async () => {
    setError("");
    try {
      if (!desktopBridge) throw new Error(t("Copy failed"));
      setQr(await desktopBridge.schoolOtpQr(value.trim() || undefined));
    } catch (reason) {
      setError(t(reason instanceof Error ? reason.message : String(reason)));
    }
  };
  const [camera, setCamera] = useState(false);
  const [reading, setReading] = useState(false);
  const [accounts, setAccounts] = useState<OtpImportAccount[]>([]);
  const [error, setError] = useState("");
  const [label, setLabel] = useState("");
  const [changing, setChanging] = useState(false);
  const [path, setPath] = useState<OtpPathId>("school");
  // A newly saved setup closes the chooser again.
  useEffect(() => setChanging(false), [saved]);
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
    // What was read lands in the paste field, so it can be checked or copied.
    setPath("paste");
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
  const choosing = !saved || changing || Boolean(value.trim());
  const busy = disabled || reading || camera;
  const copyButton = (
    <Button
      variant="default"
      size="xs"
      aria-label={t("Copy OTP setup")}
      disabled={busy || (!value.trim() && !saved) || !desktopBridge}
      onClick={() => void copy()}
    >
      {t(copied ? "Copied" : "Copy setup")}
    </Button>
  );
  const exportButtons = (
    <Group gap={6} wrap="nowrap">
      {copyButton}
      <Button
        variant="default"
        size="xs"
        disabled={busy || (!value.trim() && !saved) || !desktopBridge}
        onClick={() => void showQr()}
      >
        {t("Show QR code")}
      </Button>
    </Group>
  );
  return (
    <Input.Wrapper
      className="otpSetupField"
      label={t("Automatic school verification codes")}
      id="school-otp-setup"
    >
      {!choosing ? (
        <div className="otpSaved" role="status">
          <span aria-hidden="true">✓</span>
          <Text size="sm">{t("Codes are filled in for you at sign-in.")}</Text>
          {exportButtons}
          <Button
            variant="subtle"
            size="xs"
            disabled={busy}
            onClick={() => setChanging(true)}
          >
            {t("Change")}
          </Button>
        </div>
      ) : (
        <div className="otpPaths" role="radiogroup">
          <OtpPath
            value="school"
            current={path}
            onSelect={setPath}
            title={t("Let the app set it up")}
            recommended
            hint={t(
              "Opens the school site. Sign in if asked; the app registers itself and saves the setup.",
            )}
          >
            <Button
              size="xs"
              disabled={busy || !desktopBridge}
              onClick={() =>
                openSchoolSettings(
                  "https://isct.ex-tic.com/idm/user/mfa_setting/",
                  true,
                )
              }
            >
              {t("Open the school site")}
            </Button>
            <Text size="xs" c="dimmed">
              {t(
                "Already registered elsewhere? The school asks to reset it; then update your other authenticators.",
              )}
            </Text>
          </OtpPath>
          <OtpPath
            value="export"
            current={path}
            onSelect={setPath}
            title={t("From an authenticator app")}
            hint={t(
              "Google Authenticator: Transfer accounts → Export accounts, select only the school account.",
            )}
          >
            <Group gap="xs">
              <Button
                size="xs"
                variant="default"
                disabled={busy}
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
                    size="xs"
                    variant="default"
                    disabled={busy}
                    loading={reading}
                  >
                    {t("Select image")}
                  </Button>
                )}
              </FileButton>
            </Group>
          </OtpPath>
          <OtpPath
            value="paste"
            current={path}
            onSelect={setPath}
            title={t("Paste a setup link or secret")}
            hint={t("Not the six-digit code that changes every 30 seconds.")}
          >
            <div className="otpSetupRow">
              <TextInput
                id="school-otp-setup"
                type="password"
                size="xs"
                aria-label={t("Automatic school verification codes")}
                value={value}
                autoCapitalize="none"
                autoComplete="off"
                spellCheck={false}
                disabled={disabled || reading}
                placeholder={t("otpauth://… or secret key")}
                onChange={(event) => {
                  onChange(event.target.value);
                  setLabel("");
                  setError("");
                }}
              />
              {exportButtons}
            </div>
          </OtpPath>
          {saved && changing && !value.trim() && (
            <Button
              variant="subtle"
              size="xs"
              color="gray"
              className="otpKeep"
              onClick={() => setChanging(false)}
            >
              {t("Keep the saved setup")}
            </Button>
          )}
        </div>
      )}
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
      {qr && (
        <Modal
          opened
          centered
          onClose={() => setQr("")}
          title={t("Add to another authenticator")}
        >
          <Stack gap="sm" align="center">
            <img
              className="otpQr"
              alt={t("OTP setup QR code")}
              src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(qr)}`}
            />
            <Text size="sm">
              {t(
                "Scan this with any authenticator app. If you reset OTP at school, the old school entry in your other authenticators no longer works: delete it and use this one.",
              )}
            </Text>
            <Text size="xs" c="dimmed">
              {t(
                "Anyone who sees this QR code can generate your verification codes. Close it when you are done.",
              )}
            </Text>
          </Stack>
        </Modal>
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
