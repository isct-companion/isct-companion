import { errorText } from "../lib/error-text";
import { Check } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import {
  Alert,
  Button,
  FileButton,
  Group,
  Input,
  Modal,
  SegmentedControl,
  Stack,
  Text,
  TextInput,
} from "./ui";
import { t } from "../i18n";
import { useTimeout } from "usehooks-ts";
import { desktopBridge } from "../platform/native";
import {
  importOtp,
  otpImportError,
  readOtpImage,
  type OtpImportAccount,
} from "../platform/otp-import";
import { NativeError } from "../platform/native-error";
import { confirmAction } from "./confirm";
import { OtpGuideButton } from "./OtpGuide";
import { SchoolOtpCodeButton } from "./SchoolOtpCode";
import { QrCamera } from "./QrCamera";
import "./otp-setup-input.css";

/** How the setup reaches the app: the app sets it up at school, or it is
 * imported from where the person already has it. */
type OtpBranch = "help" | "import";
/** Where an imported setup comes from. */
type OtpSource = "export" | "paste";

/**
 * One of the two ways to get school OTP into the app. They are mutually
 * exclusive, so they read as radio cards; each says what it does, and only
 * the chosen one shows its controls.
 */
function OtpPath({
  value,
  current,
  onSelect,
  title,
  hint,
  children,
}: {
  value: OtpBranch;
  current: OtpBranch;
  onSelect: (value: OtpBranch) => void;
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
        <span>
          <b>{title}</b>
          <small>{hint}</small>
        </span>
      </button>
      {selected && <div className="otpPathBody">{children}</div>}
    </div>
  );
}

export function OtpSetupInput({
  value,
  saved,
  otpOnly,
  canShowCode,
  disabled,
  onChange,
  onScanState,
  openSetup,
}: {
  value: string;
  saved: boolean;
  otpOnly: boolean;
  canShowCode: boolean;
  disabled: boolean;
  onChange: (value: string) => void;
  onScanState: (scanning: boolean) => void;
  openSetup: () => void;
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
      setError(errorText(reason, { nativeMessage: false }) ?? String(reason));
    }
  };
  // The SVG holds the secret, so it lives only while the dialog is open.
  const [qr, setQr] = useState("");
  const showQr = async () => {
    setError("");
    try {
      if (!desktopBridge) throw new Error(t("settings.otp.copyFailed"));
      setQr(await desktopBridge.schoolOtpQr());
    } catch (reason) {
      setError(errorText(reason, { nativeMessage: false }) ?? String(reason));
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
  const [branch, setBranch] = useState<OtpBranch>("help");
  const [source, setSource] = useState<OtpSource>("export");
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
  // A good read from a scan, an image or a paste folds the chooser into one
  // line, like a saved setup. A typed entry read as the field loses focus
  // stays open: that blur is often the press on Connect, which must not move
  // away before the pointer comes up.
  const choose = (account: OtpImportAccount, fold = true) => {
    onChange(account.setup);
    // What was read lands in the paste field, so Change shows it there.
    setBranch("import");
    setSource("paste");
    setLabel(account.label || account.issuer);
    setAccounts([]);
    setError("");
    setChanging(!fold);
  };
  // `pasted` text is a setup link or secret, not a QR code, so a payload Core
  // cannot read is worded for what the person gave.
  const load = async (source: string | File, pasted = false, fold = true) => {
    const current = ++generation.current;
    setError("");
    // Only decoding an image takes long enough to hold Connect back; text
    // is read at once, and Connect sends the field as it is anyway.
    if (typeof source !== "string") setReading(true);
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
        if (found.length === 1) choose(found[0], fold);
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
  // What the field holds was read and is waiting for Connect.
  const read = label !== null && Boolean(value.trim());
  // The chooser folds to one line once a setup is saved or read; Change
  // opens it again, to fix a wrong image or link.
  const choosing = changing || (value.trim() ? !read : !saved);
  const busy = disabled || reading || camera;
  const readText = t(
    "settings.credentialImport.readAccountReadyOnceConnected",
    {
      account: label || t("settings.credentialImport.otpSecret"),
    },
  );
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
    <Group gap="space-3" wrap="nowrap">
      {copyButton}
      <Button
        variant="default"
        size="xs"
        disabled={busy || otpOnly || !canShowCode || !desktopBridge}
        title={otpOnly ? t("otpEnrollment.connectBeforeExport") : undefined}
        onClick={() => void showQr()}
      >
        {t("settings.otp.showQRCode")}
      </Button>
    </Group>
  );
  return (
    <Input.Wrapper
      className="otpSetupField"
      label={t("settings.main.oneTimePassword")}
      id="school-otp-setup"
    >
      {!choosing ? (
        <div className="otpSaved" role="status">
          <Check size={16} strokeWidth={1.75} aria-hidden />
          <Text size="sm">
            {read
              ? readText
              : t(
                  otpOnly
                    ? "otpEnrollment.savedOnDevice"
                    : "settings.credentialImport.codesAreFilledInFor",
                )}
          </Text>
          {saved && !read && canShowCode && (
            <SchoolOtpCodeButton disabled={busy} />
          )}
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
        <div
          className="otpPaths"
          role="radiogroup"
          aria-label={t("settings.main.oneTimePassword")}
        >
          <OtpPath
            value="help"
            current={branch}
            onSelect={setBranch}
            title={t("otpEnrollment.help")}
            hint={t("otpEnrollment.helpHint")}
          >
            {/* Asked once, before the school page opens, whatever the school
                has: Core then replaces an existing setup without asking again. */}
            <Button
              variant="default"
              disabled={busy}
              onClick={() =>
                void confirmAction(
                  t("otpEnrollment.confirm"),
                  t("otpEnrollment.start"),
                  { title: t("otpEnrollment.help") },
                ).then((confirmed) => {
                  if (confirmed) openSetup();
                })
              }
            >
              {t("otpEnrollment.start")}
            </Button>
          </OtpPath>
          <OtpPath
            value="import"
            current={branch}
            onSelect={setBranch}
            title={t("otpEnrollment.import")}
            hint={t("otpEnrollment.importHint")}
          >
            <SegmentedControl
              size="xs"
              value={source}
              onChange={(next) => setSource(next as OtpSource)}
              data={[
                {
                  value: "export",
                  label: t("settings.credentialImport.sourceAuthenticator"),
                },
                {
                  value: "paste",
                  label: t("settings.credentialImport.sourceTitechApp"),
                },
              ]}
            />
            {source === "export" ? (
              <>
                <Text size="xs" c="dimmed">
                  {t("settings.credentialImport.exportFromGoogleAuthenticator")}
                </Text>
                <Group gap="space-4">
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
              </>
            ) : (
              <>
                <Text size="xs" c="dimmed">
                  {t("settings.credentialImport.pasteFromTitechApp")}
                </Text>
                <div className="otpSetupRow">
                  <TextInput
                    id="school-otp-setup"
                    type="password"
                    size="xs"
                    aria-label={t(
                      "settings.credentialImport.setupLinkOrSecret",
                    )}
                    value={value}
                    autoCapitalize="none"
                    autoComplete="off"
                    spellCheck={false}
                    disabled={disabled || reading}
                    placeholder={t(
                      "settings.credentialImport.otpauthOrSecretKey",
                    )}
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
                        void load(value, true, false);
                    }}
                  />
                  {exportButtons}
                </div>
                <OtpGuideButton guide="titechApp" />
              </>
            )}
            <Text size="xs" c="dimmed">
              {t("settings.credentialImport.readOnThisDevice")}
            </Text>
          </OtpPath>
          {/* Opened with Change: fold back to the setup just read, or drop
              a wrong entry and keep the saved one. */}
          {changing && (read || saved) && (
            <Group gap="space-3" className="otpKeep">
              {read && (
                <Button
                  variant="subtle"
                  size="xs"
                  onClick={() => setChanging(false)}
                >
                  {t("settings.credentialImport.collapse")}
                </Button>
              )}
              {saved && (
                <Button
                  variant="subtle"
                  size="xs"
                  color="gray"
                  onClick={() => {
                    generation.current += 1;
                    onChange("");
                    setLabel(null);
                    setError("");
                    setChanging(false);
                  }}
                >
                  {t("settings.credentialImport.keepTheSavedSetup")}
                </Button>
              )}
            </Group>
          )}
        </div>
      )}
      {choosing && read && (
        <Text size="xs" c="green">
          {readText}
        </Text>
      )}
      {error && (
        <Alert color="red" role="alert" mt="space-4">
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
          <Stack gap="space-5" align="center">
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
          <Stack gap="space-4" mah={360} style={{ overflowY: "auto" }}>
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
