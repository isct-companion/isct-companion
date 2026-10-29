import { errorText } from "../../app/shared";
import { notify as appNotify } from "../../app/notify";
import { useEffect, useRef, useState } from "react";
import {
  Badge,
  Button,
  FileButton,
  Group,
  Paper,
  Progress,
  Stack,
  Text,
  TextInput,
} from "../../components/ui";
import { OtpSetupInput } from "../../components/OtpSetupInput";
import { MatrixCamera } from "../../components/MatrixCamera";
import { UiInput } from "../../components/ui";
import { confirmAction } from "../../components/confirm";
import { lookup, t, type MessageKey } from "../../i18n";

import { desktopBridge } from "../../app/native";
import {
  MATRIX_COLUMNS,
  MATRIX_ROWS,
  MATRIX_SIZE,
  matrixConfidence,
  type MatrixConfidence,
  normalizeMatrixText,
  parseMatrixText,
  recognizeMatrixImage,
} from "../../lib/matrix-ocr";
import type {
  SchoolAuthProfile,
  SchoolCredentialStatus,
  SchoolCredentials,
  SchoolLoginEvent,
} from "../../platform/desktop-bridge";
import { useSchoolSession } from "../../platform/school-session";

const emptyStatus: SchoolCredentialStatus = {
  science: {
    availability: "not-configured",
    saved: false,
    username: "",
    hasOtp: false,
    hasMatrix: false,
  },
  tokyoTech: {
    availability: "not-configured",
    saved: false,
    username: "",
    hasOtp: false,
    hasMatrix: false,
  },
};

type Draft = {
  username: string;
  password: string;
  otpSecret: string;
  matrix: string[];
};

const CONFIDENCE_NOTE = {
  high: "settings.schoolAccounts.confidenceHigh",
  medium: "settings.schoolAccounts.confidenceMedium",
  low: "settings.schoolAccounts.confidenceLow",
} satisfies Record<MatrixConfidence, MessageKey>;

// Steps the login script reports, in the order each portal shows them.
const STEPS = [
  { key: "identifier", label: "settings.schoolAccounts.id" },
  { key: "password", label: "settings.main.password" },
  { key: "code", label: "courses.code" },
  { key: "connected", label: "common.signedIn" },
] as const satisfies readonly { key: string; label: MessageKey }[];
// Portal-side steps that sit between the ones shown above.
const STEP_ALIASES: Record<string, string> = {
  agree: "password",
  method: "code",
  otp: "code",
  matrix: "code",
};
// Reset on every reported step; a single portal round trip is far shorter.
const STEP_TIMEOUT_MS = 30_000;

type Connection = {
  state: "idle" | "checking" | "timeout" | SchoolLoginEvent["state"];
  step: string;
  message: string;
};

function SignInSteps({ connection }: { connection: Connection }) {
  if (connection.state === "maintenance")
    return (
      <Text c="yellow" size="sm">
        {t("common.underMaintenance")}
      </Text>
    );
  const connected = connection.state === "connected";
  const current = connected
    ? STEPS.length - 1
    : Math.max(
        0,
        STEPS.findIndex((step) => step.key === connection.step),
      );
  const failed = !["checking", "progress", "connected"].includes(
    connection.state,
  );
  const value = connected ? 100 : ((current + 0.35) / STEPS.length) * 100;
  return (
    <Stack gap={5} className="signInProgress">
      <Group justify="space-between" gap="xs" wrap="nowrap">
        {STEPS.map((step, index) => (
          <Text
            key={step.key}
            size="xs"
            fw={index === current ? 700 : 500}
            c={
              connected || index < current
                ? "green"
                : index === current && failed
                  ? "red"
                  : index === current
                    ? undefined
                    : "dimmed"
            }
          >
            {t(step.label)}
          </Text>
        ))}
      </Group>
      <Progress
        value={value}
        size="sm"
        radius="xl"
        color={failed ? "red" : connected ? "green" : "campus"}
        animated={!failed && !connected}
      />
    </Stack>
  );
}

const emptyDraft = (): Draft => ({
  username: "",
  password: "",
  otpSecret: "",
  matrix: Array.from({ length: MATRIX_SIZE }, () => ""),
});

function MatrixEditor({
  value,
  masked,
  onBeginEdit,
  onChange,
  notify,
  disabled,
  onScanState,
  onCopy,
  matrixOcrAvailable,
}: {
  disabled: boolean;
  onScanState: (scanning: boolean) => void;
  value: string[];
  masked: boolean;
  onBeginEdit: () => void;
  onChange: (value: string[]) => void;
  notify: (message: string) => void;
  onCopy: () => void;
  matrixOcrAvailable: boolean;
}) {
  const inputs = useRef<Array<HTMLInputElement | null>>([]);
  const [recognizing, setRecognizing] = useState(false);
  const [cameraOpen, setCameraOpen] = useState(false);
  const scanState = useRef(onScanState);
  scanState.current = onScanState;
  useEffect(() => {
    scanState.current(cameraOpen || recognizing);
    return () => scanState.current(false);
  }, [cameraOpen, recognizing]);
  const resetScanInput = useRef<(() => void) | null>(null);
  const [scanProgress, setScanProgress] = useState(0);
  const [confidence, setConfidence] = useState<Array<number | undefined>>([]);
  useEffect(() => {
    if (masked || value.every((cell) => !cell)) setConfidence([]);
  }, [masked, value]);
  const markManual = (indexes: number[]) =>
    setConfidence((current) => {
      const next = [...current];
      indexes.forEach((index) => {
        next[index] = undefined;
      });
      return next;
    });
  const write = (start: number, raw: string) => {
    onBeginEdit();
    const letters = normalizeMatrixText(raw);
    if (!letters) return;
    const next = [...value];
    [...letters].forEach((letter, offset) => {
      if (start + offset < MATRIX_SIZE) next[start + offset] = letter;
    });
    markManual([...letters].map((_, offset) => start + offset));
    onChange(next);
    inputs.current[Math.min(MATRIX_SIZE - 1, start + letters.length)]?.focus();
  };
  const load = (text: string) => {
    try {
      const cells = parseMatrixText(text);
      onBeginEdit();
      onChange(cells);
      setConfidence([]);
      notify(t("settings.schoolAccounts.matrixPastedVerifyItAgainst"));
    } catch (error) {
      appNotify.error(error);
    }
  };
  const recognize = async (file?: File) => {
    if (!file || recognizing || disabled) return;
    setRecognizing(true);
    onScanState(true);
    setScanProgress(0);
    try {
      const result = await recognizeMatrixImage(file, setScanProgress);
      if (result.every((cell) => !cell.text)) {
        notify(t("matrixCamera.noReadableLettersFoundCheck"));
        return;
      }
      onBeginEdit();
      onChange(result.map((cell) => cell.text));
      setConfidence(result.map((cell) => cell.confidence));
      notify(t("settings.schoolAccounts.scanCompleteVerifyAllLetters"));
    } catch (error) {
      appNotify.error(error);
    } finally {
      resetScanInput.current?.();
      setRecognizing(false);
      onScanState(false);
    }
  };
  return (
    <div className="matrixEditor">
      <div className="matrixTools">
        <b>{t("settings.schoolAccounts.matrix")}</b>
        <span>{masked ? 70 : value.filter(Boolean).length}/70</span>
        {matrixOcrAvailable && (
          <Button
            variant="default"
            size="compact-xs"
            disabled={recognizing || disabled || cameraOpen}
            onClick={() => {
              setCameraOpen(true);
              onScanState(true);
            }}
          >
            {t("settings.schoolAccounts.scanWithCamera")}
          </Button>
        )}
        {cameraOpen && (
          <MatrixCamera
            onClose={() => {
              setCameraOpen(false);
              onScanState(false);
            }}
            onComplete={(cells) => {
              onBeginEdit();
              onChange(cells.map((cell) => cell.text));
              setConfidence(cells.map((cell) => cell.confidence));
              setCameraOpen(false);
              onScanState(false);
              notify(t("settings.schoolAccounts.scanCompleteVerifyAllLetters"));
            }}
          />
        )}
        {matrixOcrAvailable && (
          <FileButton
            resetRef={resetScanInput}
            disabled={recognizing || disabled || cameraOpen}
            onChange={(file) => void recognize(file ?? undefined)}
            accept="image/*,.heic,.heif"
          >
            {(props) => (
              <Button
                {...props}
                variant="default"
                size="compact-xs"
                loading={recognizing}
                disabled={disabled}
              >
                {recognizing
                  ? t("settings.schoolAccounts.readingCount70", {
                      count: scanProgress,
                    })
                  : t("settings.schoolAccounts.scanImage")}
              </Button>
            )}
          </FileButton>
        )}
        {masked && (
          <Button
            variant="default"
            size="compact-xs"
            disabled={recognizing || disabled || cameraOpen}
            onClick={onCopy}
          >
            {t("settings.schoolAccounts.copy")}
          </Button>
        )}
      </div>
      {!masked && (
        <div
          className="matrixConfidenceLegend"
          aria-label={t("settings.schoolAccounts.recognitionConfidence")}
        >
          <span data-confidence="high">
            {t("settings.schoolAccounts.greenHigh")}
          </span>
          <span data-confidence="medium">
            {t("settings.schoolAccounts.yellowReview")}
          </span>
          <span data-confidence="low">
            {t("settings.schoolAccounts.redLowUnreadable")}
          </span>
          <span>{t("settings.schoolAccounts.uncoloredManualEntry")}</span>
        </div>
      )}
      {/* The saved Matrix keeps its 7×10 shape; typing over a cell replaces it. */}
      <div
        className="matrixGrid"
        aria-label={t("settings.schoolAccounts.tokyoTechMatrix")}
      >
        <span />
        {Array.from({ length: MATRIX_COLUMNS }, (_, column) => (
          <b key={column}>{String.fromCharCode(65 + column)}</b>
        ))}
        {Array.from({ length: MATRIX_ROWS }, (_, row) => (
          <div className="matrixRow" key={row}>
            <b>{row + 1}</b>
            {Array.from({ length: MATRIX_COLUMNS }, (_, column) => {
              const index = row * MATRIX_COLUMNS + column;
              return (
                <UiInput
                  key={column}
                  ref={(element) => {
                    inputs.current[index] = element;
                  }}
                  disabled={recognizing || disabled || cameraOpen}
                  aria-label={`${String.fromCharCode(65 + column)}${row + 1}`}
                  data-confidence={
                    !masked && confidence[index] !== undefined
                      ? matrixConfidence(value[index], confidence[index]!)
                      : undefined
                  }
                  title={
                    masked
                      ? t("settings.schoolAccounts.savedHidden")
                      : confidence[index] === undefined
                        ? t("settings.schoolAccounts.manualEntry")
                        : t(
                            CONFIDENCE_NOTE[
                              matrixConfidence(value[index], confidence[index]!)
                            ],
                          )
                  }
                  value={masked ? "" : value[index]}
                  placeholder={masked ? "•" : ""}
                  autoCapitalize="characters"
                  autoComplete="off"
                  spellCheck={false}
                  inputMode="text"
                  maxLength={1}
                  onFocus={(event) => {
                    event.currentTarget.select();
                  }}
                  onChange={(event) => {
                    onBeginEdit();
                    const letter =
                      normalizeMatrixText(event.target.value).at(-1) ?? "";
                    const next = [...value];
                    next[index] = letter;
                    onChange(next);
                    markManual([index]);
                    if (letter) inputs.current[index + 1]?.focus();
                  }}
                  onPaste={(event) => {
                    event.preventDefault();
                    const text = event.clipboardData.getData("text");
                    // A complete Matrix always replaces the grid from A1;
                    // shorter snippets continue from the focused cell.
                    if (text.replace(/\s/g, "").length >= MATRIX_SIZE)
                      load(text);
                    else write(index, text);
                  }}
                  onKeyDown={(event) => {
                    if (
                      event.key === "Backspace" &&
                      !value[index] &&
                      index > 0
                    ) {
                      event.preventDefault();
                      const next = [...value];
                      next[index - 1] = "";
                      onBeginEdit();
                      markManual([index - 1]);
                      onChange(next);
                      inputs.current[index - 1]?.focus();
                    } else if (event.key === "ArrowLeft" && index > 0) {
                      event.preventDefault();
                      inputs.current[index - 1]?.focus();
                    } else if (
                      event.key === "ArrowRight" &&
                      index + 1 < MATRIX_SIZE
                    ) {
                      event.preventDefault();
                      inputs.current[index + 1]?.focus();
                    } else if (event.key === "ArrowUp" && row > 0) {
                      event.preventDefault();
                      inputs.current[index - MATRIX_COLUMNS]?.focus();
                    } else if (
                      event.key === "ArrowDown" &&
                      row + 1 < MATRIX_ROWS
                    ) {
                      event.preventDefault();
                      inputs.current[index + MATRIX_COLUMNS]?.focus();
                    }
                  }}
                />
              );
            })}
          </div>
        ))}
      </div>
    </div>
  );
}

function Account({
  profile,
  title,
  summary,
  draft,
  setDraft,
  refresh,
  notify,
  showLoginPage,
  matrixOcrAvailable,
}: {
  profile: SchoolAuthProfile;
  title: string;
  summary: SchoolCredentialStatus["science"];
  draft: Draft;
  setDraft: (draft: Draft) => void;
  refresh: () => Promise<void>;
  notify: (message: string) => void;
  showLoginPage: (
    profile: SchoolAuthProfile,
    targetUrl?: string,
    setupOtp?: boolean,
  ) => void;
  matrixOcrAvailable: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const [matrixReplacing, setMatrixReplacing] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [connection, setConnection] = useState<Connection>({
    state: "idle",
    step: "",
    message: "",
  });
  const timeout = useRef<number | undefined>(undefined);
  // Only an attempt started from this card may pull the person to the page.
  const started = useRef(false);
  const armTimeout = () => {
    if (timeout.current) window.clearTimeout(timeout.current);
    timeout.current = window.setTimeout(() => {
      setConnection((current) => ({
        ...current,
        state: "timeout",
        message: t("settings.schoolAccounts.noResponseFromThePortal"),
      }));
      setBusy(false);
      started.current = false;
    }, STEP_TIMEOUT_MS);
  };
  useEffect(() => {
    const dispose = desktopBridge?.onSchoolLoginStatus?.((event) => {
      if (event.profile !== profile) return;
      if (event.state === "progress") {
        const step = STEP_ALIASES[event.message] ?? event.message;
        setConnection({ state: "progress", step, message: "" });
        setBusy(true);
        armTimeout();
        return;
      }
      if (timeout.current) window.clearTimeout(timeout.current);
      setConnection((current) => ({
        state: event.state,
        step: current.step,
        message: event.state === "connected" ? "" : event.message,
      }));
      setBusy(false);
      if (event.state === "needs-user" && started.current)
        showLoginPage(profile);
      started.current = false;
    });
    return () => {
      dispose?.();
      if (timeout.current) window.clearTimeout(timeout.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [profile]);
  const stored = useSchoolSession(desktopBridge, profile);
  // Signed in, failed, or not set up; nothing while the first result is out.
  const unavailable = summary.availability
    ? summary.availability === "not-configured" ||
      summary.availability === "missing"
    : !summary.saved;
  const session = unavailable
    ? "not-configured"
    : stored === "not-configured"
      ? undefined
      : stored;
  const credentials = (): SchoolCredentials => ({
    username: draft.username,
    password: draft.password,
    otpSecret: draft.otpSecret,
    matrix: draft.matrix.join(""),
  });
  const validateMatrix = () => {
    const count = draft.matrix.filter(Boolean).length;
    if (
      profile === "tokyo-tech" &&
      ((count > 0 && count < MATRIX_SIZE) || (matrixReplacing && count === 0))
    ) {
      notify(t("settings.schoolAccounts.fillAll70MatrixCells"));
      return false;
    }
    return true;
  };
  const connect = async () => {
    if (!desktopBridge || scanning || !validateMatrix()) return;
    const nextCredentials = credentials();
    setBusy(true);
    started.current = true;
    setConnection({ state: "checking", step: "", message: "" });
    armTimeout();
    try {
      const result = await desktopBridge.connectSchool(
        profile,
        nextCredentials,
      );
      setDraft({ ...emptyDraft(), username: draft.username.trim() });
      setMatrixReplacing(false);
      await refresh();
      // The sign-in events already showed the school's own words; a result
      // they did not cover still ends the checking state.
      if (result.state !== "active" && result.problem?.kind !== "needs-user") {
        if (timeout.current) window.clearTimeout(timeout.current);
        started.current = false;
        setConnection((current) =>
          current.state === "checking" || current.state === "progress"
            ? {
                state: result.state === "maintenance" ? "maintenance" : "error",
                step: current.step,
                message: result.problem?.message ?? t("common.signInFailed"),
              }
            : current,
        );
        setBusy(false);
      }
    } catch (error) {
      if (timeout.current) window.clearTimeout(timeout.current);
      started.current = false;
      setConnection((current) =>
        current.state === "maintenance"
          ? current
          : {
              state: "error",
              step: "",
              message:
                errorText(error) ??
                t("settings.schoolAccounts.signInCouldNotStart"),
            },
      );
      setBusy(false);
      await refresh().catch(() => undefined);
    }
  };
  const copyMatrix = async () => {
    if (
      !desktopBridge ||
      !(await confirmAction(
        t("settings.schoolAccounts.copyTheSavedMatrixAs"),
        t("settings.schoolAccounts.copy"),
      ))
    )
      return;
    try {
      if (await desktopBridge.copySchoolMatrix())
        notify(t("settings.schoolAccounts.matrixCopiedPasteItSoon"));
    } catch (error) {
      // Native commands reject with their error string.
      appNotify.error(error);
    }
  };
  const forget = async () => {
    if (
      !desktopBridge ||
      !(await confirmAction(
        t("settings.schoolAccounts.forgetSavedTitleCredentialsOn", { title }),
        t("settings.schoolAccounts.forget"),
      ))
    )
      return;
    setBusy(true);
    try {
      await desktopBridge.deleteSchoolCredentials(profile);
      setDraft(emptyDraft());
      setMatrixReplacing(false);
      setConnection({ state: "idle", step: "", message: "" });
      await refresh();
      notify(t("settings.schoolAccounts.accountForgotten", { account: title }));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Paper
      component="section"
      className="schoolAccount"
      data-setting={profile === "science" ? "science" : "tokyo-tech"}
      p="md"
      radius="md"
      withBorder
    >
      <Group component="header" justify="space-between" wrap="nowrap">
        <Text fw={700}>{title}</Text>
        {session && (
          <Badge
            variant="light"
            color={
              session === "active"
                ? "green"
                : session === "maintenance"
                  ? "yellow"
                  : session === "failed"
                    ? "red"
                    : "gray"
            }
          >
            {t(
              session === "active"
                ? "common.signedIn"
                : session === "maintenance"
                  ? "common.underMaintenance"
                  : session === "failed"
                    ? "common.signInFailed"
                    : "settings.schoolAccounts.notSetUp",
            )}
          </Badge>
        )}
      </Group>
      {connection.state !== "idle" && (
        <div className="schoolConnection" data-state={connection.state}>
          <SignInSteps connection={connection} />
          {connection.message && (
            <span>{lookup(connection.message) ?? connection.message}</span>
          )}
          {connection.state !== "connected" && (
            <Button
              variant="subtle"
              size="compact-xs"
              onClick={() => showLoginPage(profile)}
            >
              {t("settings.schoolAccounts.showPage")}
            </Button>
          )}
        </div>
      )}
      {summary.availability === "missing" && (
        <Text c="red" size="sm">
          {t("settings.schoolAccounts.savedCredentialsAreMissingFrom")}
        </Text>
      )}
      {summary.availability === "access-denied" && (
        <Text c="orange" size="sm">
          {t("settings.schoolAccounts.theSystemCredentialVaultIs")}
        </Text>
      )}
      <div className="credentialFields">
        <TextInput
          label={t("settings.schoolAccounts.id")}
          value={draft.username}
          autoCapitalize="none"
          autoComplete="username"
          onChange={(event) =>
            setDraft({ ...draft, username: event.target.value })
          }
        />
        {/* A saved secret shows as fixed dots; typing replaces it, blank keeps it. */}
        <TextInput
          type="password"
          label={t("settings.main.password")}
          value={draft.password}
          placeholder={summary.saved ? SAVED_SECRET : ""}
          autoComplete="current-password"
          onChange={(event) =>
            setDraft({ ...draft, password: event.target.value })
          }
        />
        {profile === "science" && (
          <OtpSetupInput
            value={draft.otpSecret}
            openSchoolSettings={(url, setupOtp) =>
              showLoginPage("science", url, setupOtp)
            }
            saved={summary.hasOtp}
            disabled={busy}
            onScanState={setScanning}
            onChange={(otpSecret) => setDraft({ ...draft, otpSecret })}
          />
        )}
      </div>
      {profile === "tokyo-tech" && (
        <MatrixEditor
          disabled={busy}
          onScanState={setScanning}
          value={draft.matrix}
          masked={summary.hasMatrix && !matrixReplacing}
          onBeginEdit={() => setMatrixReplacing(true)}
          onChange={(matrix) => setDraft({ ...draft, matrix })}
          notify={notify}
          onCopy={() => void copyMatrix()}
          matrixOcrAvailable={matrixOcrAvailable}
        />
      )}
      <div className="actions schoolAccountActions">
        <Button
          disabled={busy || scanning}
          loading={busy}
          onClick={() => void connect()}
        >
          {t("settings.campusWifi.connect")}
        </Button>
        {(summary.saved || summary.hasOtp) && (
          <Button
            variant="subtle"
            color="red"
            disabled={busy || scanning}
            onClick={() => void forget()}
          >
            {t("settings.schoolAccounts.forget")}
          </Button>
        )}
      </div>
    </Paper>
  );
}

/** What a saved password or OTP setup shows in its empty field. */
const SAVED_SECRET = "••••••••";

export function SchoolAccounts({
  otpSetupRevision = 0,
  notify,
  showLoginPage,
  matrixOcrAvailable,
}: {
  otpSetupRevision?: number;
  notify: (message: string) => void;
  showLoginPage: (
    profile: SchoolAuthProfile,
    targetUrl?: string,
    setupOtp?: boolean,
  ) => void;
  matrixOcrAvailable: boolean;
}) {
  const [status, setStatus] = useState(emptyStatus);
  const [science, setScience] = useState(emptyDraft);
  const [tokyoTech, setTokyoTech] = useState(emptyDraft);
  const refresh = async () => {
    if (!desktopBridge) return;
    const next = await desktopBridge.schoolCredentialStatus();
    setStatus(next);
    setScience((current) => ({
      ...current,
      username: current.username || next.science.username,
    }));
    setTokyoTech((current) => ({
      ...current,
      username: current.username || next.tokyoTech.username,
    }));
  };
  useEffect(() => {
    void refresh().catch((error) => appNotify.error(error));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const observedOtpSetup = useRef(otpSetupRevision);
  useEffect(() => {
    if (observedOtpSetup.current === otpSetupRevision) return;
    observedOtpSetup.current = otpSetupRevision;
    setScience((current) => ({ ...current, otpSecret: "" }));
    void refresh().catch((error) => appNotify.error(error));
  }, [otpSetupRevision]);
  if (!desktopBridge)
    return (
      <p className="settingsNote">
        {t("settings.schoolAccounts.schoolSignInNeedsThe")}
      </p>
    );
  return (
    <div className="schoolAccounts">
      <Account
        profile="science"
        title="Science Tokyo Portal"
        summary={status.science}
        draft={science}
        setDraft={setScience}
        refresh={refresh}
        notify={notify}
        showLoginPage={showLoginPage}
        matrixOcrAvailable={matrixOcrAvailable}
      />
      <Account
        profile="tokyo-tech"
        title="Tokyo Tech Portal"
        summary={status.tokyoTech}
        draft={tokyoTech}
        setDraft={setTokyoTech}
        refresh={refresh}
        notify={notify}
        showLoginPage={showLoginPage}
        matrixOcrAvailable={matrixOcrAvailable}
      />
    </div>
  );
}
