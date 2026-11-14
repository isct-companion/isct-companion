import { Check } from "lucide-react";
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
import { NativeError } from "../platform/native-error";
import { OtpGuideButton } from "./OtpGuide";
import { QrCamera } from "./QrCamera";
import "./otp-setup-input.css";

type OtpPathId = "export" | "paste";

/**
 * One way to give the app the school's OTP setup. The two are mutually
 * exclusive, so they read as radio cards; only the chosen one shows its
 * controls.
 */
function OtpPath({
  value,
  current,
  onSelect,
  title,
  hint,
  children,
}: {
  value: OtpPathId;
  current: OtpPathId;
  onSelect: (value: OtpPathId) => void;
  title: string;
  hint: string;
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
  otpOnly,
  disabled,
  onChange,
  onScanState,
  openSetup,
  openManual,
}: {
  value: string;
  saved: boolean;
  otpOnly: boolean;
  disabled: boolean;
  onChange: (value: string) => void;
  onScanState: (scanning: boolean) => void;
  openSetup: () => void;
  openManual: () => void;
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
        throw new Error(t("settings.otp.copyFailed"));
      setCopied(true);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  };
  // The SVG holds the secret, so it lives only while the dialog is open.
  const [qr, setQr] = useState("");
  const showQr = async () => {
    setError("");
    try {
      if (!desktopBridge) throw new Error(t("settings.otp.copyFailed"));
      setQr(await desktopBridge.schoolOtpQr(value.trim() || undefined));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  };
  const [camera, setCamera] = useState(false);
  const [reading, setReading] = useState(false);
  const [accounts, setAccounts] = useState<OtpImportAccount[]>([]);
  const [error, setError] = useState("");
  // What the field holds was read: the account's name, or "" for a bare
  // secret. `null` until something is read.
  const [label, setLabel] = useState<string | null>(null);
  const [changing, setChanging] = useState(false);
  const [path, setPath] = useState<OtpPathId>("export");
  const [consent, setConsent] = useState(false);
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
  // `pasted` text is a setup link or secret, not a QR code, so a payload Core
  // cannot read is worded for what the person gave.
  const load = async (source: string | File, pasted = false) => {
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
              : t("settings.otp.noQRCodeFoundChoose"),
          );
        return;
      }
      try {
        const found = await importOtp(payload);
        if (current !== generation.current) return;
        if (found.length === 1) choose(found[0]);
        else setAccounts(found);
      } catch (reason) {
        if (current !== generation.current) return;
        setError(
          pasted &&
            reason instanceof NativeError &&
            reason.code === "otp-import-invalid"
            ? t("settings.credentialImport.notASetupLinkOrSecret")
            : otpImportError(reason),
        );
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
      aria-label={t("settings.otp.copyOTPSetup")}
      disabled={busy || (!value.trim() && !saved) || !desktopBridge}
      onClick={() => void copy()}
    >
      {t(
        copied
          ? "settings.credentialImport.copied"
          : "settings.credentialImport.copySetup",
      )}
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
        {t("settings.otp.showQRCode")}
      </Button>
    </Group>
  );
  return (
    <Input.Wrapper
      className="otpSetupField"
      label={t("settings.otp.automaticSchoolVerificationCodes")}
      id="school-otp-setup"
    >
      {choosing && (
        <Group gap="xs">
          <Button size="xs" disabled={busy} onClick={() => setConsent(true)}>
            {t("otpEnrollment.help")}
          </Button>
          <Button
            size="xs"
            variant="subtle"
            disabled={busy}
            onClick={openManual}
          >
            {t("otpEnrollment.manual")}
          </Button>
        </Group>
      )}
      <Modal
        opened={consent}
        onClose={() => setConsent(false)}
        title={t("otpEnrollment.help")}
      >
        <Stack gap="sm">
          <Text size="sm">{t("otpEnrollment.consent")}</Text>
          <Group justify="end">
            <Button
              variant="default"
              onClick={() => {
                setConsent(false);
                openManual();
              }}
            >
              {t("otpEnrollment.manual")}
            </Button>
            <Button
              onClick={() => {
                setConsent(false);
                openSetup();
              }}
            >
              {t("otpEnrollment.continue")}
            </Button>
          </Group>
        </Stack>
      </Modal>
      {!choosing ? (
        <div className="otpSaved" role="status">
          <Check size={16} strokeWidth={1.75} aria-hidden />
          <Text size="sm">
            {t(
              otpOnly
                ? "otpEnrollment.savedOnDevice"
                : "settings.credentialImport.codesAreFilledInFor",
            )}
          </Text>
          {exportButtons}
          <Button
            variant="subtle"
            size="xs"
            disabled={busy}
            onClick={() => setChanging(true)}
          >
            {t("settings.credentialImport.change")}
          </Button>
        </div>
      ) : (
        <div className="otpPaths" role="radiogroup">
          <OtpPath
            value="export"
            current={path}
            onSelect={setPath}
            title={t("settings.credentialImport.fromAnAuthenticatorApp")}
            hint={t("settings.credentialImport.exportFromGoogleAuthenticator")}
          >
            <Group gap="xs">
              <Button
                size="xs"
                variant="default"
                disabled={busy}
                onClick={() => setCamera(true)}
              >
                {t("settings.credentialImport.scanQR")}
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
                    {t("settings.otp.selectImage")}
                  </Button>
                )}
              </FileButton>
              <OtpGuideButton guide="google" />
            </Group>
          </OtpPath>
          <OtpPath
            value="paste"
            current={path}
            onSelect={setPath}
            title={t("settings.otp.pasteASetupLinkOr")}
            hint={t("settings.credentialImport.pasteFromTitechApp")}
          >
            <div className="otpSetupRow">
              <TextInput
                id="school-otp-setup"
                type="password"
                size="xs"
                aria-label={t("settings.otp.automaticSchoolVerificationCodes")}
                value={value}
                autoCapitalize="none"
                autoComplete="off"
                spellCheck={false}
                disabled={disabled || reading}
                placeholder={t("settings.credentialImport.otpauthOrSecretKey")}
                onChange={(event) => {
                  onChange(event.target.value);
                  setLabel(null);
                  setError("");
                }}
                // A paste or a finished entry is read at once; Connect is
                // only for whether the school accepts it.
                onPaste={(event) => {
                  const text = event.clipboardData.getData("text");
                  if (!text.trim()) return;
                  event.preventDefault();
                  void load(text, true);
                }}
                onBlur={() => {
                  if (value.trim() && label === null && !error)
                    void load(value, true);
                }}
              />
              {exportButtons}
            </div>
            <OtpGuideButton guide="titechApp" />
          </OtpPath>
          {saved && changing && !value.trim() && (
            <Button
              variant="subtle"
              size="xs"
              color="gray"
              className="otpKeep"
              onClick={() => setChanging(false)}
            >
              {t("settings.credentialImport.keepTheSavedSetup")}
            </Button>
          )}
        </div>
      )}
      {choosing && (
        <Text size="xs" c="dimmed">
          {t("settings.credentialImport.readOnThisDevice")}
        </Text>
      )}
      {label !== null && (
        <Text size="xs" c="dimmed">
          {t("settings.credentialImport.readAccountReadyOnceConnected", {
            account: label || t("settings.credentialImport.otpSecret"),
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
          title={t("settings.otp.addToAnotherAuthenticator")}
        >
          <Stack gap="sm" align="center">
            <img
              className="otpQr"
              alt={t("settings.otp.otpSetupQRCode")}
              src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(qr)}`}
            />
            <Text size="sm">
              {t("settings.otp.scanThisWithAnyAuthenticator")}
            </Text>
            <Text size="xs" c="dimmed">
              {t("settings.otp.anyoneWhoSeesThisQR")}
            </Text>
          </Stack>
        </Modal>
      )}
      {!!accounts.length && (
        <Modal
          opened
          onClose={() => setAccounts([])}
          title={t("settings.otp.chooseYourSchoolAccount")}
        >
          <Stack gap="xs" mah={360} style={{ overflowY: "auto" }}>
            {accounts.map((account, index) => (
              <Button
                key={index}
                variant="default"
                onClick={() => choose(account)}
              >
                {[account.issuer, account.label].filter(Boolean).join(" · ") ||
                  t("settings.otp.accountNumber", { number: index + 1 })}
              </Button>
            ))}
          </Stack>
        </Modal>
      )}
    </Input.Wrapper>
  );
}
